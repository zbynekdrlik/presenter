use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use tokio::sync::{broadcast, watch, Mutex};

use crate::discovery::{self, FinderShutdown, SourceList};
use crate::encoder::{uyvy_to_bgra, ResizingEncoder};
use crate::ndi_sdk::NdiLib;
use crate::receiver::{NdiReceiver, VideoFrame};

const TARGET_HEIGHT: u32 = 720;
const TARGET_FPS: u32 = 20;
const JPEG_QUALITY: i32 = 75;
const JPEG_BROADCAST_CAPACITY: usize = 8;

/// Callback for reporting NDI connection status changes.
pub type StatusCallback = Arc<dyn Fn(String) + Send + Sync>;

struct ActiveStream {
    stop_signal: watch::Sender<bool>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
    encode_task: Option<tokio::task::JoinHandle<()>>,
}

/// Orchestrates NDI discovery, capture, and a single shared MJPEG broadcast.
///
/// Discovery runs in a persistent background thread (mDNS source list).
/// Capture runs in an OS thread that publishes raw frames to a `tokio::sync::watch`
/// channel; one async encode task consumes them, applies a frame-rate accumulator
/// to throttle to `TARGET_FPS`, resizes to `TARGET_HEIGHT` via `ResizingEncoder`,
/// JPEG-encodes at quality `JPEG_QUALITY`, and broadcasts to all connected clients.
pub struct NdiManager {
    sdk: Arc<NdiLib>,
    source_list: SourceList,
    _finder_shutdown: FinderShutdown,
    active_stream: Mutex<Option<ActiveStream>>,
    raw_frame_tx: watch::Sender<Option<Arc<VideoFrame>>>,
    raw_frame_rx: watch::Receiver<Option<Arc<VideoFrame>>>,
    jpeg_tx: broadcast::Sender<Bytes>,
}

impl NdiManager {
    /// Try to create a new manager by loading the NDI SDK.
    ///
    /// Returns `None` if the NDI runtime is not available on this system.
    pub fn try_new() -> Option<Self> {
        let sdk = NdiLib::load().ok()?;
        let sdk = Arc::new(sdk);
        let (source_list, finder_shutdown) = discovery::spawn_persistent_finder(Arc::clone(&sdk));
        let (raw_frame_tx, raw_frame_rx) = watch::channel(None);
        let (jpeg_tx, _) = broadcast::channel(JPEG_BROADCAST_CAPACITY);
        Some(Self {
            sdk,
            source_list,
            _finder_shutdown: finder_shutdown,
            active_stream: Mutex::new(None),
            raw_frame_tx,
            raw_frame_rx,
            jpeg_tx,
        })
    }

    pub fn is_available(&self) -> bool {
        true
    }

    pub fn discover_sources(&self, _timeout_ms: u32) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Subscribe to the single shared JPEG broadcast.
    pub fn subscribe_frames(&self) -> broadcast::Receiver<Bytes> {
        self.jpeg_tx.subscribe()
    }

    /// Start capturing from the named NDI source.
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
            .spawn({
                let stop_rx = stop_rx.clone();
                move || {
                    run_capture_thread(sdk, source_name, raw_tx, stop_rx, status_cb);
                }
            })?;

        let encode_task = tokio::spawn(run_encode_task(
            self.raw_frame_rx.clone(),
            self.jpeg_tx.clone(),
            stop_rx,
        ));

        let mut active = self.active_stream.lock().await;
        *active = Some(ActiveStream {
            stop_signal: stop_tx,
            capture_thread: Some(capture_thread),
            encode_task: Some(encode_task),
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
            let _ = self.raw_frame_tx.send(None);
            if let Some(h) = stream.capture_thread.take() {
                let _ = h.join();
            }
            if let Some(h) = stream.encode_task.take() {
                h.abort();
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

// ---------------------------------------------------------------------------
// Encode task — frame-skip accumulator, SIMD resize, JPEG encode, broadcast.
// ---------------------------------------------------------------------------

async fn run_encode_task(
    mut raw_rx: watch::Receiver<Option<Arc<VideoFrame>>>,
    jpeg_tx: broadcast::Sender<Bytes>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let fourcc_uyvy = u32::from_le_bytes([b'U', b'Y', b'V', b'Y']);
    let fourcc_bgra = u32::from_le_bytes([b'B', b'G', b'R', b'A']);
    let fourcc_bgrx = u32::from_le_bytes([b'B', b'G', b'R', b'X']);
    let mut encoder = ResizingEncoder::new(JPEG_QUALITY, TARGET_HEIGHT);

    // Frame-skip phase accumulator: emit when phase >= source_fps.
    let mut phase: u64 = 0;

    tracing::info!(
        target_height = TARGET_HEIGHT,
        target_fps = TARGET_FPS,
        "NDI encode task started"
    );

    loop {
        tokio::select! {
            res = stop_rx.changed() => {
                if res.is_err() || *stop_rx.borrow() { break; }
            }
            res = raw_rx.changed() => {
                if res.is_err() { break; }
            }
        }

        let frame = match raw_rx.borrow_and_update().as_ref() {
            Some(f) => Arc::clone(f),
            None => continue,
        };

        // Compute source fps (Resolume sends 30/1 typically). Fall back to
        // TARGET_FPS if metadata is missing/zero — that means "emit every frame".
        let source_fps: u64 = if frame.frame_rate_d > 0 && frame.frame_rate_n > 0 {
            (frame.frame_rate_n as u64) / (frame.frame_rate_d as u64).max(1)
        } else {
            TARGET_FPS as u64
        };
        let source_fps = source_fps.max(TARGET_FPS as u64);

        phase += TARGET_FPS as u64;
        if phase < source_fps {
            continue;
        }
        phase -= source_fps;

        let (bgra, w, h) = if frame.fourcc == fourcc_bgra || frame.fourcc == fourcc_bgrx {
            (frame.data.clone(), frame.width, frame.height)
        } else if frame.fourcc == fourcc_uyvy {
            (
                uyvy_to_bgra(&frame.data, frame.width, frame.height),
                frame.width,
                frame.height,
            )
        } else {
            tracing::warn!(
                fourcc = format!("0x{:08x}", frame.fourcc),
                "unsupported fourcc; skipping"
            );
            continue;
        };

        match encoder.encode(&bgra, w, h) {
            Ok(jpeg) => {
                let _ = jpeg_tx.send(Bytes::from(jpeg));
            }
            Err(e) => {
                tracing::error!("JPEG encode error: {e}");
            }
        }
    }

    tracing::info!("NDI encode task stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(id: u32, fourcc: u32, w: u32, h: u32) -> VideoFrame {
        VideoFrame {
            width: w,
            height: h,
            data: vec![id as u8; (w * h * 4) as usize],
            stride: w * 4,
            fourcc,
            frame_rate_n: 30,
            frame_rate_d: 1,
        }
    }

    #[test]
    fn watch_newest_wins() {
        let (tx, mut rx) = watch::channel::<Option<Arc<VideoFrame>>>(None);
        tx.send(Some(Arc::new(make_frame(
            1,
            u32::from_le_bytes([b'B', b'G', b'R', b'A']),
            1,
            1,
        ))))
        .unwrap();
        tx.send(Some(Arc::new(make_frame(
            2,
            u32::from_le_bytes([b'B', b'G', b'R', b'A']),
            1,
            1,
        ))))
        .unwrap();
        // After multiple sends, watch holds only the newest (data filled with id=2).
        let snap = rx.borrow_and_update();
        assert!(snap.as_ref().unwrap().data.iter().all(|&b| b == 2));
    }

    #[test]
    fn watch_starts_empty() {
        let (_tx, rx) = watch::channel::<Option<Arc<VideoFrame>>>(None);
        assert!(rx.borrow().is_none());
    }

    /// 30 fps source → TARGET_FPS=20 should produce 2 emits per 3 raw frames.
    #[test]
    fn frame_skip_accumulator_30_to_20_emits_2_of_3() {
        let mut phase: u64 = 0;
        let source_fps: u64 = 30;
        let target_fps: u64 = TARGET_FPS as u64;
        let mut emits = 0;
        let total = 30;
        for _ in 0..total {
            phase += target_fps;
            if phase < source_fps {
                continue;
            }
            phase -= source_fps;
            emits += 1;
        }
        assert_eq!(
            emits, 20,
            "30→20 fps accumulator should emit exactly 20 of 30 frames"
        );
    }

    /// 60 fps source → TARGET_FPS=20 should produce 1 emit per 3 raw frames.
    #[test]
    fn frame_skip_accumulator_60_to_20_emits_1_of_3() {
        let mut phase: u64 = 0;
        let source_fps: u64 = 60;
        let target_fps: u64 = TARGET_FPS as u64;
        let mut emits = 0;
        let total = 60;
        for _ in 0..total {
            phase += target_fps;
            if phase < source_fps {
                continue;
            }
            phase -= source_fps;
            emits += 1;
        }
        assert_eq!(
            emits, 20,
            "60→20 fps accumulator should emit exactly 20 of 60 frames"
        );
    }
}
