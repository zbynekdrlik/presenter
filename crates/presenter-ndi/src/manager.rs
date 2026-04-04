use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use tokio::sync::{broadcast, Mutex};

use crate::discovery::{self, NdiSourceInfo};
use crate::encoder::JpegEncoder;
use crate::ndi_sdk::NdiLib;
use crate::receiver::NdiReceiver;

struct ActiveStream {
    stop_signal: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

/// Orchestrates NDI capture and MJPEG encoding.
///
/// Frames are published to a broadcast channel that WebSocket handlers subscribe to.
pub struct NdiManager {
    sdk: Arc<NdiLib>,
    active_stream: Mutex<Option<ActiveStream>>,
    /// Broadcast channel for JPEG frames. Each subscriber gets every frame.
    frame_tx: broadcast::Sender<Bytes>,
}

impl NdiManager {
    /// Try to create a new manager by loading the NDI SDK.
    ///
    /// Returns `None` if the NDI runtime is not available on this system.
    pub fn try_new() -> Option<Self> {
        let sdk = NdiLib::load().ok()?;
        let (frame_tx, _) = broadcast::channel(4); // small buffer, drop old frames
        Some(Self {
            sdk: Arc::new(sdk),
            active_stream: Mutex::new(None),
            frame_tx,
        })
    }

    /// Whether the NDI SDK is loaded and available.
    pub fn is_available(&self) -> bool {
        true
    }

    /// Discover NDI sources on the local network.
    pub fn discover_sources(&self, timeout_ms: u32) -> Result<Vec<NdiSourceInfo>> {
        discovery::discover_sources(&self.sdk, timeout_ms)
    }

    /// Subscribe to the JPEG frame stream.
    pub fn subscribe_frames(&self) -> broadcast::Receiver<Bytes> {
        self.frame_tx.subscribe()
    }

    /// Start capturing from the named NDI source and encoding to JPEG.
    pub async fn start_stream(&self, ndi_name: &str) -> Result<()> {
        self.stop_stream().await;

        let sdk = Arc::clone(&self.sdk);
        let frame_tx = self.frame_tx.clone();
        let source_name = ndi_name.to_string();
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

        let task = tokio::task::spawn_blocking(move || {
            run_capture_loop(sdk, frame_tx, source_name, stop_rx);
        });

        let mut active = self.active_stream.lock().await;
        *active = Some(ActiveStream {
            stop_signal: stop_tx,
            task,
        });

        Ok(())
    }

    /// Stop the active NDI capture stream, if any.
    pub async fn stop_stream(&self) {
        let mut active = self.active_stream.lock().await;
        if let Some(stream) = active.take() {
            let _ = stream.stop_signal.send(true);
            let _ = stream.task.await;
        }
    }
}

/// Blocking capture loop that runs inside `spawn_blocking`.
fn run_capture_loop(
    sdk: Arc<NdiLib>,
    frame_tx: broadcast::Sender<Bytes>,
    source_name: String,
    mut stop_rx: tokio::sync::watch::Receiver<bool>,
) {
    let receiver = match NdiReceiver::connect(&sdk, &source_name, 10) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("failed to connect to NDI source: {e}");
            return;
        }
    };

    let fourcc_uyvy = u32::from_le_bytes([b'U', b'Y', b'V', b'Y']);
    let fourcc_bgra = u32::from_le_bytes([b'B', b'G', b'R', b'A']);
    let fourcc_bgrx = u32::from_le_bytes([b'B', b'G', b'R', b'X']);
    let encoder = JpegEncoder::new(75); // quality 75 — good balance of size/speed

    tracing::info!("NDI capture started for '{source_name}'");

    loop {
        if *stop_rx.borrow() {
            break;
        }

        let frame = match receiver.capture_video(100) {
            Ok(Some(f)) => f,
            Ok(None) => {
                if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow_and_update() {
                    break;
                }
                continue;
            }
            Err(e) => {
                tracing::error!("NDI capture error: {e}");
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };

        // Encode to JPEG based on pixel format
        let jpeg = if frame.fourcc == fourcc_bgra || frame.fourcc == fourcc_bgrx {
            encoder.encode_bgra(&frame.data, frame.width, frame.height)
        } else if frame.fourcc == fourcc_uyvy {
            encoder.encode_uyvy(&frame.data, frame.width, frame.height)
        } else {
            tracing::warn!("unsupported fourcc: 0x{:08x}", frame.fourcc);
            continue;
        };

        let jpeg = match jpeg {
            Ok(data) => data,
            Err(e) => {
                tracing::error!("JPEG encode error: {e}");
                continue;
            }
        };

        // Broadcast to all subscribers (WebSocket handlers)
        // If no subscribers, the frame is dropped — that's fine
        let _ = frame_tx.send(Bytes::from(jpeg));
    }

    tracing::info!("NDI capture stream stopped");
}
