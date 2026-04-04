use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use crate::discovery::{self, NdiSourceInfo};
use crate::encoder::{bgra_to_yuv420, uyvy_to_yuv420, VideoEncoder};
use crate::ndi_sdk::NdiLib;
use crate::receiver::NdiReceiver;
use crate::webrtc_session::WebRtcSession;

struct ActiveStream {
    stop_signal: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

/// Orchestrates NDI capture, H.264 encoding, and WebRTC session management.
pub struct NdiManager {
    sdk: Arc<NdiLib>,
    active_stream: Mutex<Option<ActiveStream>>,
    sessions: Arc<Mutex<Vec<Arc<WebRtcSession>>>>,
}

impl NdiManager {
    /// Try to create a new manager by loading the NDI SDK.
    ///
    /// Returns `None` if the NDI runtime is not available on this system.
    pub fn try_new() -> Option<Self> {
        let sdk = NdiLib::load().ok()?;
        Some(Self {
            sdk: Arc::new(sdk),
            active_stream: Mutex::new(None),
            sessions: Arc::new(Mutex::new(Vec::new())),
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

    /// Start capturing from the named NDI source and encoding to H.264.
    ///
    /// Encoded frames are pushed to all connected WebRTC sessions.
    pub async fn start_stream(&self, ndi_name: &str) -> Result<()> {
        self.stop_stream().await;

        let sdk = Arc::clone(&self.sdk);
        let sessions = Arc::clone(&self.sessions);
        let source_name = ndi_name.to_string();
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

        let task = tokio::task::spawn_blocking(move || {
            run_capture_loop(sdk, sessions, source_name, stop_rx);
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

    /// Create a new WHEP session from an SDP offer, returning the SDP answer.
    pub async fn create_whep_session(&self, sdp_offer: String) -> Result<String> {
        let session = Arc::new(WebRtcSession::new().await?);
        let sdp_answer = session.accept_offer(sdp_offer).await?;
        self.sessions.lock().await.push(session);
        Ok(sdp_answer)
    }
}

/// Blocking capture loop that runs inside `spawn_blocking`.
fn run_capture_loop(
    sdk: Arc<NdiLib>,
    sessions: Arc<Mutex<Vec<Arc<WebRtcSession>>>>,
    source_name: String,
    mut stop_rx: tokio::sync::watch::Receiver<bool>,
) {
    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("no tokio runtime handle in capture loop: {e}");
            return;
        }
    };

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
    let mut encoder: Option<VideoEncoder> = None;

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

        // Lazily create encoder on first frame
        let enc = match encoder.as_mut() {
            Some(e) => e,
            None => match VideoEncoder::new(frame.width, frame.height) {
                Ok(e) => {
                    encoder = Some(e);
                    // SAFETY: we just assigned Some, so this branch always succeeds
                    match encoder.as_mut() {
                        Some(e) => e,
                        None => continue,
                    }
                }
                Err(e) => {
                    tracing::error!("failed to create encoder: {e}");
                    continue;
                }
            },
        };

        // Convert pixel format to YUV420
        let yuv = if frame.fourcc == fourcc_uyvy {
            uyvy_to_yuv420(&frame.data, frame.width, frame.height)
        } else if frame.fourcc == fourcc_bgra || frame.fourcc == fourcc_bgrx {
            bgra_to_yuv420(&frame.data, frame.width, frame.height)
        } else {
            tracing::warn!("unsupported fourcc: 0x{:08x}", frame.fourcc);
            continue;
        };

        let encoded = match enc.encode(&yuv) {
            Ok(data) => data,
            Err(e) => {
                tracing::error!("encode error: {e}");
                continue;
            }
        };

        if encoded.is_empty() {
            continue;
        }

        let duration = if frame.frame_rate_n > 0 && frame.frame_rate_d > 0 {
            std::time::Duration::from_secs_f64(
                frame.frame_rate_d as f64 / frame.frame_rate_n as f64,
            )
        } else {
            std::time::Duration::from_millis(33) // ~30 fps default
        };

        let sessions_ref = sessions.clone();
        rt.spawn(async move {
            let sessions = sessions_ref.lock().await;
            for session in sessions.iter() {
                let _ = session.write_video(encoded.clone(), duration).await;
            }
        });
    }

    tracing::info!("NDI capture stream stopped");
}
