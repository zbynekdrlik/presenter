use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use tokio::sync::{broadcast, Mutex};

use crate::discovery::{self, FinderShutdown, SourceList};
use crate::encoder::JpegEncoder;
use crate::ndi_sdk::NdiLib;
use crate::receiver::{NdiReceiver, VideoFrame};

type FrameSlot = Arc<std::sync::Mutex<Option<VideoFrame>>>;

/// Callback for reporting NDI connection status changes.
pub type StatusCallback = Arc<dyn Fn(String) + Send + Sync>;

struct ActiveStream {
    stop_signal: tokio::sync::watch::Sender<bool>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
    encode_thread: Option<std::thread::JoinHandle<()>>,
}

/// Orchestrates NDI discovery, capture, and MJPEG encoding.
///
/// Discovery runs in a persistent background thread — sources accumulate
/// over time via mDNS. Capture and encode run in separate OS threads
/// connected by a shared frame slot (newest frame wins).
pub struct NdiManager {
    sdk: Arc<NdiLib>,
    source_list: SourceList,
    _finder_shutdown: FinderShutdown,
    active_stream: Mutex<Option<ActiveStream>>,
    frame_tx: broadcast::Sender<Bytes>,
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
        let (frame_tx, _) = broadcast::channel(8);
        Some(Self {
            sdk,
            source_list,
            _finder_shutdown: finder_shutdown,
            active_stream: Mutex::new(None),
            frame_tx,
        })
    }

    /// Whether the NDI SDK is loaded and available.
    pub fn is_available(&self) -> bool {
        true
    }

    /// Read currently known NDI sources from the persistent finder.
    ///
    /// Returns instantly — no network scan is performed. The `timeout_ms`
    /// parameter is kept for API compatibility but is ignored.
    pub fn discover_sources(&self, _timeout_ms: u32) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Subscribe to the JPEG frame stream.
    pub fn subscribe_frames(&self) -> broadcast::Receiver<Bytes> {
        self.frame_tx.subscribe()
    }

    /// Start capturing from the named NDI source and encoding to JPEG.
    ///
    /// Spawns two OS threads: one for frame capture, one for JPEG encoding.
    /// The optional `status_cb` is called with "connected" on first frame
    /// and "disconnected" after 3 seconds without frames.
    pub async fn start_stream(
        &self,
        ndi_name: &str,
        status_cb: Option<StatusCallback>,
    ) -> Result<()> {
        self.stop_stream().await;

        let sdk = Arc::clone(&self.sdk);
        let frame_tx = self.frame_tx.clone();
        let source_name = ndi_name.to_string();
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let frame_slot: FrameSlot = Arc::new(std::sync::Mutex::new(None));
        let condvar = Arc::new(std::sync::Condvar::new());

        // Capture thread
        let slot_c = Arc::clone(&frame_slot);
        let condvar_c = Arc::clone(&condvar);
        let stop_rx_c = stop_rx.clone();
        let capture_thread = std::thread::Builder::new()
            .name("ndi-capture".into())
            .spawn(move || {
                run_capture_thread(sdk, source_name, slot_c, condvar_c, stop_rx_c, status_cb);
            })?;

        // Encode thread
        let slot_e = Arc::clone(&frame_slot);
        let condvar_e = Arc::clone(&condvar);
        let encode_thread = std::thread::Builder::new()
            .name("ndi-encode".into())
            .spawn(move || {
                run_encode_thread(slot_e, condvar_e, frame_tx, stop_rx);
            })?;

        let mut active = self.active_stream.lock().await;
        *active = Some(ActiveStream {
            stop_signal: stop_tx,
            capture_thread: Some(capture_thread),
            encode_thread: Some(encode_thread),
        });

        Ok(())
    }

    /// Stop the active NDI capture stream, if any.
    pub async fn stop_stream(&self) {
        let mut active = self.active_stream.lock().await;
        if let Some(mut stream) = active.take() {
            let _ = stream.stop_signal.send(true);
            if let Some(h) = stream.capture_thread.take() {
                let _ = h.join();
            }
            if let Some(h) = stream.encode_thread.take() {
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
    frame_slot: FrameSlot,
    condvar: Arc<std::sync::Condvar>,
    stop_rx: tokio::sync::watch::Receiver<bool>,
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
    let mut capture_timeout_ms: u32 = 50; // fallback before first frame

    tracing::info!("NDI capture thread started for '{source_name}'");

    loop {
        if *stop_rx.borrow() {
            break;
        }

        match receiver.capture_video(capture_timeout_ms) {
            Ok(Some(frame)) => {
                // Adapt timeout to actual frame rate
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

                // Write to shared slot — newest frame wins
                {
                    let mut slot = frame_slot.lock().unwrap_or_else(|e| e.into_inner());
                    *slot = Some(frame);
                }
                condvar.notify_one();
            }
            Ok(None) => {
                // Timeout — check for disconnect
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

// ---------------------------------------------------------------------------
// Encode thread
// ---------------------------------------------------------------------------

fn run_encode_thread(
    frame_slot: FrameSlot,
    condvar: Arc<std::sync::Condvar>,
    frame_tx: broadcast::Sender<Bytes>,
    stop_rx: tokio::sync::watch::Receiver<bool>,
) {
    let fourcc_uyvy = u32::from_le_bytes([b'U', b'Y', b'V', b'Y']);
    let fourcc_bgra = u32::from_le_bytes([b'B', b'G', b'R', b'A']);
    let fourcc_bgrx = u32::from_le_bytes([b'B', b'G', b'R', b'X']);
    let encoder = JpegEncoder::new(75);

    tracing::info!("NDI encode thread started");

    loop {
        if *stop_rx.borrow() {
            break;
        }

        // Wait for a frame (with timeout so we can check stop signal)
        let frame = {
            let slot = frame_slot.lock().unwrap_or_else(|e| e.into_inner());
            let (mut slot, _) = condvar
                .wait_timeout(slot, std::time::Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner());
            slot.take()
        };

        let frame = match frame {
            Some(f) => f,
            None => continue,
        };

        let jpeg = if frame.fourcc == fourcc_bgra || frame.fourcc == fourcc_bgrx {
            encoder.encode_bgra(&frame.data, frame.width, frame.height)
        } else if frame.fourcc == fourcc_uyvy {
            encoder.encode_uyvy(&frame.data, frame.width, frame.height)
        } else {
            tracing::warn!("unsupported fourcc: 0x{:08x}", frame.fourcc);
            continue;
        };

        match jpeg {
            Ok(data) => {
                let _ = frame_tx.send(Bytes::from(data));
            }
            Err(e) => {
                tracing::error!("JPEG encode error: {e}");
            }
        }
    }

    tracing::info!("NDI encode thread stopped");
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
    fn frame_slot_newest_wins() {
        let slot: FrameSlot = Arc::new(std::sync::Mutex::new(None));
        {
            let mut s = slot.lock().unwrap();
            *s = Some(make_frame(1));
        }
        {
            let mut s = slot.lock().unwrap();
            *s = Some(make_frame(2));
        }
        let frame = slot.lock().unwrap().take();
        assert_eq!(frame.unwrap().width, 2);
    }

    #[test]
    fn frame_slot_empty_read() {
        let slot: FrameSlot = Arc::new(std::sync::Mutex::new(None));
        let frame = slot.lock().unwrap().take();
        assert!(frame.is_none());
    }
}
