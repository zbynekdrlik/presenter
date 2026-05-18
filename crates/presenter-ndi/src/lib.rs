#![allow(non_camel_case_types)]

pub mod discovery;
pub mod encoder;
mod manager;
pub mod ndi_sdk;
pub mod receiver;

pub use discovery::SourceList;
pub use manager::NdiManager;
pub use manager::StatusCallback;

use std::sync::Once;

static GST_INIT: Once = Once::new();

/// Initialize GStreamer + register Rust plugins (webrtcsink, ndisrc, whip/whep).
///
/// Safe to call multiple times; subsequent calls are no-ops. Returns an error if
/// GStreamer cannot initialize OR if a required Rust plugin fails to register.
pub fn init() -> anyhow::Result<()> {
    let mut result: anyhow::Result<()> = Ok(());
    GST_INIT.call_once(|| {
        if let Err(e) = gstreamer::init() {
            result = Err(anyhow::anyhow!("gstreamer init failed: {e}"));
            return;
        }
        if let Err(e) = gstrswebrtc::plugin_register_static() {
            result = Err(anyhow::anyhow!("webrtcsink plugin register failed: {e}"));
            return;
        }
        if let Err(e) = gstwebrtchttp::plugin_register_static() {
            result = Err(anyhow::anyhow!("webrtchttp plugin register failed: {e}"));
            return;
        }
        if let Err(e) = gstndi::plugin_register_static() {
            result = Err(anyhow::anyhow!("ndisrc plugin register failed: {e}"));
            return;
        }
    });
    result
}

/// Check whether the VAAPI H264 encoder element is available.
///
/// Returns true iff `gst::ElementFactory::find("vah264enc")` returns Some.
/// Use at startup to fail loudly if the host is missing the VA-API driver.
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
    fn vah264enc_present_when_vaapi_installed() {
        init().expect("gst init");
        // On the dev/prod host we install gstreamer1.0-vaapi.
        // On CI runners (ubuntu-latest) we also install it via this task.
        // If this assertion fails locally, install `gstreamer1.0-vaapi` first.
        assert!(
            vah264enc_available(),
            "vah264enc not available — install gstreamer1.0-vaapi + intel-media-va-driver-non-free"
        );
    }
}
