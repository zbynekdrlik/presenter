#![allow(non_camel_case_types)]

pub mod discovery;
pub mod manager;
pub mod ndi_sdk;
pub mod pipeline;

pub use discovery::SourceList;
pub use manager::NdiManager;
pub use manager::StatusCallback;

use std::sync::OnceLock;

/// Holds the outcome of the one-shot `gstreamer::init()` + plugin registration.
/// Subsequent `init()` calls return the SAME outcome — a previously failed
/// init does not silently succeed on retry.
static GST_INIT_RESULT: OnceLock<Result<(), String>> = OnceLock::new();

/// Initialize GStreamer + register Rust plugins (webrtcsink, ndisrc).
///
/// Safe and cheap to call repeatedly. The outcome of the first call is cached
/// and every subsequent call returns the same Ok/Err — so a caller that hits
/// an init failure cannot be lulled into proceeding by re-calling `init()`.
pub fn init() -> anyhow::Result<()> {
    let outcome = GST_INIT_RESULT.get_or_init(|| {
        if let Err(e) = gstreamer::init() {
            return Err(format!("gstreamer init failed: {e}"));
        }
        if let Err(e) = gstrswebrtc::plugin_register_static() {
            return Err(format!("webrtcsink plugin register failed: {e}"));
        }
        if let Err(e) = gstndi::plugin_register_static() {
            return Err(format!("ndisrc plugin register failed: {e}"));
        }
        // rtpgccbwe is required by webrtcsink for congestion control;
        // without it codec discovery drifts after the first consumer.
        if let Err(e) = gstrsrtp::plugin_register_static() {
            return Err(format!("rsrtp plugin register failed: {e}"));
        }
        Ok(())
    });
    match outcome {
        Ok(()) => Ok(()),
        Err(msg) => Err(anyhow::anyhow!("{msg}")),
    }
}

/// Detect which hardware H264 encoder is registered with GStreamer.
///
/// Returns the element name (`"vah264enc"` Intel iris Xe / N100, or
/// `"nvh264enc"` NVIDIA NVENC) when one is available, `None` otherwise.
/// The order is: Intel VA-API first (matches production hardware), NVIDIA
/// NVENC second (so dev2's GeForce can drive the same pipeline).
///
/// Either path is HARDWARE-accelerated. We deliberately do NOT fall back to
/// software H264 — `x264enc` at 720p30 would burn ~150% CPU on the N100,
/// which is the whole reason the original MJPEG fell over.
///
/// Probes the live element registry on every call — cheap (hash lookup), no
/// memoization needed.
pub fn hw_h264_encoder() -> Option<&'static str> {
    ["vah264enc", "nvh264enc"]
        .into_iter()
        .find(|name| gstreamer::ElementFactory::find(name).is_some())
}

#[cfg(test)]
mod gst_init_tests {
    use super::*;

    #[test]
    fn init_is_idempotent() {
        init().expect("first init must succeed");
        init().expect("second init must succeed (no-op)");
    }

    #[test]
    fn hw_h264_encoder_probe_returns_without_panic() {
        init().expect("gst init");
        // Host-hardware-dependent: returns Some("vah264enc") on Intel/iris Xe
        // (production N100), Some("nvh264enc") on NVIDIA (dev2), or None on
        // GH `ubuntu-latest` with no GPU. The unit test only asserts the probe
        // doesn't panic; the real fail-loudly behavior (pipeline build must
        // refuse when no HW encoder is available) is exercised in
        // `pipeline.rs::tests::build_fails_when_no_hw_h264_encoder` and at
        // deploy verification on the production host.
        let _ = hw_h264_encoder();
    }
}
