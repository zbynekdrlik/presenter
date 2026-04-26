use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{watch, Mutex};

use crate::discovery::{self, FinderShutdown, SourceList};
use crate::ndi_sdk::NdiLib;
use crate::receiver::{NdiReceiver, VideoFrame};
use crate::tier::Tier;
use crate::tier_registry::{TierRegistry, TierSubscription};

/// Callback for reporting NDI connection status changes.
pub type StatusCallback = Arc<dyn Fn(String) + Send + Sync>;

struct ActiveStream {
    stop_signal: watch::Sender<bool>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
}

/// Orchestrates NDI discovery, capture, and adaptive MJPEG encoding.
///
/// Discovery runs in a persistent background thread — sources accumulate
/// over time via mDNS. Capture runs in an OS thread that publishes the
/// newest raw frame to a `tokio::sync::watch` channel; per-tier JPEG
/// encoders subscribe via `TierRegistry`.
pub struct NdiManager {
    sdk: Arc<NdiLib>,
    source_list: SourceList,
    _finder_shutdown: FinderShutdown,
    active_stream: Mutex<Option<ActiveStream>>,
    raw_frame_tx: watch::Sender<Option<Arc<VideoFrame>>>,
    tier_registry: Arc<TierRegistry>,
}

impl NdiManager {
    /// Try to create a new manager by loading the NDI SDK.
    ///
    /// Returns `None` if the NDI runtime is not available on this system.
    /// Immediately starts a persistent finder thread for source discovery.
    pub fn try_new() -> Option<Self> {
        let sdk = NdiLib::load().ok()?;
        let sdk = Arc::new(sdk);
        let (source_list, finder_shutdown) = discovery::spawn_persistent_finder(Arc::clone(&sdk));
        let (raw_frame_tx, raw_frame_rx) = watch::channel(None);
        let tier_registry = TierRegistry::new(raw_frame_rx);
        Some(Self {
            sdk,
            source_list,
            _finder_shutdown: finder_shutdown,
            active_stream: Mutex::new(None),
            raw_frame_tx,
            tier_registry,
        })
    }

    pub fn is_available(&self) -> bool {
        true
    }

    pub fn discover_sources(&self, _timeout_ms: u32) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Subscribe to a JPEG broadcast for a given adaptive tier.
    pub async fn subscribe_tier(&self, tier: Tier) -> TierSubscription {
        self.tier_registry.subscribe(tier).await
    }

    /// Start capturing from the named NDI source.
    ///
    /// Spawns one OS thread for frame capture; tier encoders are spawned
    /// lazily by `TierRegistry` as subscribers register.
    pub async fn start_stream(
        &self,
        ndi_name: &str,
        status_cb: Option<StatusCallback>,
    ) -> Result<()> {
        self.stop_stream().await;

        let sdk = Arc::clone(&self.sdk);
        let raw_tx = self.raw_frame_tx.clone();
        let source_name = ndi_name.to_string();
        let (stop_tx, stop_rx) = watch::channel(false);

        let capture_thread = std::thread::Builder::new()
            .name("ndi-capture".into())
            .spawn(move || {
                run_capture_thread(sdk, source_name, raw_tx, stop_rx, status_cb);
            })?;

        let mut active = self.active_stream.lock().await;
        *active = Some(ActiveStream {
            stop_signal: stop_tx,
            capture_thread: Some(capture_thread),
        });

        Ok(())
    }

    pub async fn is_streaming(&self) -> bool {
        self.active_stream.lock().await.is_some()
    }

    pub async fn stop_stream(&self) {
        let mut active = self.active_stream.lock().await;
        if let Some(mut stream) = active.take() {
            let _ = stream.stop_signal.send(true);
            // Capture thread checks stop_rx every iteration; clear the frame so subscribers see "stream gone"
            let _ = self.raw_frame_tx.send(None);
            if let Some(h) = stream.capture_thread.take() {
                let _ = h.join();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Capture thread
// ---------------------------------------------------------------------------

fn run_capture_thread(
    sdk: Arc<NdiLib>,
    source_name: String,
    raw_tx: watch::Sender<Option<Arc<VideoFrame>>>,
    mut stop_rx: watch::Receiver<bool>,
    status_cb: Option<StatusCallback>,
) {
    let receiver = match NdiReceiver::connect(&sdk, &source_name, 10) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("failed to connect to NDI source '{source_name}': {e}");
            if let Some(cb) = &status_cb {
                cb("disconnected".to_string());
            }
            return;
        }
    };

    let mut connected = false;
    let mut last_frame_time = std::time::Instant::now();
    let mut capture_timeout_ms: u32 = 50;

    tracing::info!("NDI capture thread started for '{source_name}'");

    loop {
        if *stop_rx.borrow() {
            break;
        }

        match receiver.capture_video(capture_timeout_ms) {
            Ok(Some(frame)) => {
                if frame.frame_rate_d > 0 && frame.frame_rate_n > 0 {
                    let period = (1000 * frame.frame_rate_d as u64) / frame.frame_rate_n as u64;
                    capture_timeout_ms = (period as u32).clamp(16, 200);
                }

                if !connected {
                    connected = true;
                    tracing::info!(
                        "NDI connected: {}x{} @ {}/{}fps",
                        frame.width,
                        frame.height,
                        frame.frame_rate_n,
                        frame.frame_rate_d
                    );
                    if let Some(cb) = &status_cb {
                        cb("connected".to_string());
                    }
                }
                last_frame_time = std::time::Instant::now();

                // Publish to watch — newest replaces previous; `Arc` so consumers don't copy data.
                let _ = raw_tx.send(Some(Arc::new(frame)));
            }
            Ok(None) => {
                if connected && last_frame_time.elapsed() > std::time::Duration::from_secs(3) {
                    connected = false;
                    tracing::warn!("NDI signal lost for '{source_name}'");
                    if let Some(cb) = &status_cb {
                        cb("disconnected".to_string());
                    }
                }
                if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow_and_update() {
                    break;
                }
            }
            Err(e) => {
                tracing::error!("NDI capture error: {e}");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }

    tracing::info!("NDI capture thread stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(id: u32) -> VideoFrame {
        VideoFrame {
            width: id,
            height: 1,
            data: vec![0u8; 4],
            stride: 4,
            fourcc: 0,
            frame_rate_n: 30,
            frame_rate_d: 1,
        }
    }

    #[test]
    fn watch_newest_wins() {
        let (tx, mut rx) = watch::channel::<Option<Arc<VideoFrame>>>(None);
        tx.send(Some(Arc::new(make_frame(1)))).unwrap();
        tx.send(Some(Arc::new(make_frame(2)))).unwrap();
        // After multiple sends, watch holds only the newest
        assert_eq!(rx.borrow_and_update().as_ref().unwrap().width, 2);
    }

    #[test]
    fn watch_starts_empty() {
        let (_tx, rx) = watch::channel::<Option<Arc<VideoFrame>>>(None);
        assert!(rx.borrow().is_none());
    }
}
