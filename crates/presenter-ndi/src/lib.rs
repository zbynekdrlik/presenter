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
        Ok(())
    });
    match outcome {
        Ok(()) => Ok(()),
        Err(msg) => Err(anyhow::anyhow!("{msg}")),
    }
}

/// Check whether the VAAPI H264 encoder element is available.
///
/// Returns true iff `gst::ElementFactory::find("vah264enc")` returns Some.
/// Use at startup to log loudly if the host is missing the VA-API driver, and
/// at pipeline-build time to fail loudly (refusing software-H264 fallback).
///
/// Probes the live element registry on every call — not cached. Cheap (hash
/// lookup), so callers don't need to memoize.
pub fn vah264enc_available() -> bool {
    gstreamer::ElementFactory::find("vah264enc").is_some()
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
    fn vah264enc_probe_returns_without_panic() {
        init().expect("gst init");
        // `vah264enc_available()` is host-hardware-dependent: returns true only when
        // an Intel VA-API device is present (`/dev/dri/renderD128` plus the
        // intel-media-va-driver). dev2 has NVIDIA, GH `ubuntu-latest` has no GPU.
        // The unit test only asserts the probe doesn't panic; the real failure mode
        // (pipeline build must fail loudly when vah264enc is missing) is exercised
        // in `pipeline.rs::tests::build_fails_when_vah264enc_missing` and at deploy
        // verification on the N100 production host.
        let _ = vah264enc_available();
    }
}
