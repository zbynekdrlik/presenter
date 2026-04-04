use crate::ndi_sdk::{
    NDIlib_audio_frame_v3_t, NDIlib_find_create_t, NDIlib_recv_create_v3_t, NDIlib_source_t,
    NDIlib_video_frame_v2_recv_t, NdiLib, NDILIB_FRAME_TYPE_AUDIO, NDILIB_FRAME_TYPE_NONE,
    NDILIB_FRAME_TYPE_VIDEO, NDILIB_RECV_BANDWIDTH_HIGHEST, NDILIB_RECV_COLOR_FORMAT_UYVY_BGRA,
};
use anyhow::{Context, Result};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Arc;
use tracing::debug;

// ---------------------------------------------------------------------------
// Public frame types (owned, safe copies of NDI frame data)
// ---------------------------------------------------------------------------

/// A captured video frame with pixel data copied out of NDI-owned memory.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub stride: u32,
    pub fourcc: u32,
    pub frame_rate_n: u32,
    pub frame_rate_d: u32,
}

/// A captured audio frame with sample data copied out of NDI-owned memory.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub sample_rate: u32,
    pub channels: u32,
    pub no_samples: u32,
    pub data: Vec<u8>,
    pub channel_stride: u32,
    pub fourcc: u32,
}

// ---------------------------------------------------------------------------
// NdiReceiver
// ---------------------------------------------------------------------------

/// Connects to a single NDI source and captures frames.
pub struct NdiReceiver {
    lib: Arc<NdiLib>,
    receiver: *mut std::ffi::c_void,
}

// The NDI receiver handle is safe to move between threads — the SDK is
// designed for this pattern (create on one thread, capture on another).
unsafe impl Send for NdiReceiver {}

impl NdiReceiver {
    /// Discover the named source and create a receiver connected to it.
    ///
    /// `timeout_secs` controls how long to wait for the source to appear
    /// on the network before giving up.
    pub fn connect(sdk: &Arc<NdiLib>, source_name: &str, timeout_secs: u32) -> Result<Self> {
        unsafe {
            let create_settings = NDIlib_find_create_t {
                show_local_sources: true,
                p_groups: std::ptr::null(),
                p_extra_ips: std::ptr::null(),
            };

            let finder = (sdk.find_create_v2)(&create_settings);
            if finder.is_null() {
                anyhow::bail!("NDIlib_find_create_v2 returned null");
            }

            let timeout_ms = timeout_secs.saturating_mul(1000);
            let _found = (sdk.find_wait_for_sources)(finder, timeout_ms);

            let mut num_sources: u32 = 0;
            let sources_ptr = (sdk.find_get_current_sources)(finder, &mut num_sources);

            let mut matched_source: Option<NDIlib_source_t> = None;

            if !sources_ptr.is_null() && num_sources > 0 {
                let sources = std::slice::from_raw_parts(sources_ptr, num_sources as usize);
                for src in sources {
                    if let Ok(name) = cstr_to_string(src.p_ndi_name) {
                        debug!("found NDI source: {name}");
                        if name.contains(source_name) {
                            matched_source = Some(*src);
                            break;
                        }
                    }
                }
            }

            let source = matched_source.context(format!(
                "NDI source '{source_name}' not found ({num_sources} sources visible)"
            ))?;

            // Create the receiver BEFORE destroying the finder, because
            // source pointers (p_ndi_name, p_url_address) are owned by
            // the finder instance. recv_create_v3 copies them internally.
            let recv_name =
                CString::new("presenter-ndi-receiver").context("CString for receiver name")?;

            let recv_settings = NDIlib_recv_create_v3_t {
                source_to_connect_to: source,
                color_format: NDILIB_RECV_COLOR_FORMAT_UYVY_BGRA,
                bandwidth: NDILIB_RECV_BANDWIDTH_HIGHEST,
                allow_video_fields: true,
                p_ndi_recv_name: recv_name.as_ptr(),
            };

            let receiver = (sdk.recv_create_v3)(&recv_settings);

            // Now safe to destroy the finder — receiver has its own copy.
            (sdk.find_destroy)(finder);

            if receiver.is_null() {
                anyhow::bail!("NDIlib_recv_create_v3 returned null");
            }

            debug!("NDI receiver created for source '{source_name}'");

            Ok(Self {
                lib: Arc::clone(sdk),
                receiver,
            })
        }
    }

    /// Capture a single video frame, blocking up to `timeout_ms`.
    ///
    /// Returns `Ok(None)` when the timeout elapses without a video frame.
    pub fn capture_video(&self, timeout_ms: u32) -> Result<Option<VideoFrame>> {
        unsafe {
            let mut video = std::mem::zeroed::<NDIlib_video_frame_v2_recv_t>();

            let frame_type = (self.lib.recv_capture_v3)(
                self.receiver,
                &mut video,
                std::ptr::null_mut(), // no audio
                std::ptr::null_mut(), // no metadata
                timeout_ms,
            );

            if frame_type == NDILIB_FRAME_TYPE_VIDEO {
                let frame = copy_video_frame(&video)?;
                (self.lib.recv_free_video_v2)(self.receiver, &video);
                return Ok(Some(frame));
            }

            if frame_type == NDILIB_FRAME_TYPE_NONE {
                return Ok(None);
            }

            // Got a different frame type (audio / metadata) — not what we
            // asked for, but not an error either.
            Ok(None)
        }
    }

    /// Capture a single audio frame, blocking up to `timeout_ms`.
    ///
    /// Returns `Ok(None)` when the timeout elapses without an audio frame.
    pub fn capture_audio(&self, timeout_ms: u32) -> Result<Option<AudioFrame>> {
        unsafe {
            let mut audio = std::mem::zeroed::<NDIlib_audio_frame_v3_t>();

            let frame_type = (self.lib.recv_capture_v3)(
                self.receiver,
                std::ptr::null_mut(), // no video
                &mut audio,
                std::ptr::null_mut(), // no metadata
                timeout_ms,
            );

            if frame_type == NDILIB_FRAME_TYPE_AUDIO {
                let frame = copy_audio_frame(&audio)?;
                (self.lib.recv_free_audio_v3)(self.receiver, &audio);
                return Ok(Some(frame));
            }

            if frame_type == NDILIB_FRAME_TYPE_NONE {
                return Ok(None);
            }

            Ok(None)
        }
    }
}

impl Drop for NdiReceiver {
    fn drop(&mut self) {
        if !self.receiver.is_null() {
            unsafe {
                (self.lib.recv_destroy)(self.receiver);
            }
            debug!("NDI receiver destroyed");
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Copy video frame data out of NDI-owned memory into a safe `Vec<u8>`.
unsafe fn copy_video_frame(frame: &NDIlib_video_frame_v2_recv_t) -> Result<VideoFrame> {
    let height = frame.yres as usize;
    let stride = frame.line_stride_in_bytes as usize;
    let total_bytes = height * stride;

    if frame.p_data.is_null() || total_bytes == 0 {
        anyhow::bail!("NDI video frame has null data or zero size");
    }

    let data = std::slice::from_raw_parts(frame.p_data, total_bytes).to_vec();

    Ok(VideoFrame {
        width: frame.xres as u32,
        height: frame.yres as u32,
        data,
        stride: stride as u32,
        fourcc: frame.fourcc,
        frame_rate_n: frame.frame_rate_n as u32,
        frame_rate_d: frame.frame_rate_d as u32,
    })
}

/// Copy audio frame data out of NDI-owned memory into a safe `Vec<u8>`.
unsafe fn copy_audio_frame(frame: &NDIlib_audio_frame_v3_t) -> Result<AudioFrame> {
    let channels = frame.no_channels as usize;
    let channel_stride = frame.channel_stride_in_bytes as usize;
    let total_bytes = channels * channel_stride;

    if frame.p_data.is_null() || total_bytes == 0 {
        anyhow::bail!("NDI audio frame has null data or zero size");
    }

    let data = std::slice::from_raw_parts(frame.p_data, total_bytes).to_vec();

    Ok(AudioFrame {
        sample_rate: frame.sample_rate as u32,
        channels: channels as u32,
        no_samples: frame.no_samples as u32,
        data,
        channel_stride: channel_stride as u32,
        fourcc: frame.fourcc,
    })
}

/// Convert a C string pointer to an owned Rust `String`.
fn cstr_to_string(ptr: *const c_char) -> Result<String> {
    if ptr.is_null() {
        anyhow::bail!("null C string pointer");
    }
    let cstr = unsafe { CStr::from_ptr(ptr) };
    Ok(cstr
        .to_str()
        .context("invalid UTF-8 in C string")?
        .to_owned())
}
