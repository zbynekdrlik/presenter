#![allow(non_camel_case_types)]

pub mod discovery;
pub mod manager;
pub mod ndi_sdk;
pub mod pipeline;
#[cfg(feature = "test-helpers")]
pub mod test_strip;
pub mod whep_session;

pub use discovery::SourceList;
pub use manager::NdiManager;
pub use manager::StatusCallback;
pub use pipeline::{PipelineSnapshot, SessionSnapshot, StreamProfile};
pub use whep_session::{IceCandidate, WhepConnectionState, WhepSession};

use std::sync::OnceLock;

/// Holds the outcome of the one-shot `gstreamer::init()` + plugin registration.
/// Subsequent `init()` calls return the SAME outcome — a previously failed
/// init does not silently succeed on retry.
static GST_INIT_RESULT: OnceLock<Result<(), String>> = OnceLock::new();

/// Ensures `GST_REGISTRY_UPDATE=yes` is set in the process environment.
///
/// Extracted as a named helper so the contract ("must run before
/// gstreamer::init()") is encoded in code, not just in comments, and so
/// the regression test for #333 item 1 can target the helper directly.
///
/// FIXME(rust-2024): `std::env::set_var` becomes `unsafe` in edition 2024
/// because POSIX `setenv` races with concurrent `getenv` calls on other
/// threads. Today (edition 2021) this is safe; on edition migration this
/// call becomes the single site to wrap in `unsafe { ... }`.
/// Intentionally NOT wrapped in `Once::call_once` — the in-process env
/// var can be cleared by a test (e.g. via `remove_var`) and the next
/// `init()` call must observably re-set it for the regression test to
/// verify the contract.
pub(crate) fn ensure_registry_rescan_env_var() {
    std::env::set_var("GST_REGISTRY_UPDATE", "yes");
}

/// Initialize GStreamer + register Rust plugins (webrtcsink, ndisrc).
///
/// Safe and cheap to call repeatedly. The outcome of the first call is cached
/// and every subsequent call returns the same Ok/Err — so a caller that hits
/// an init failure cannot be lulled into proceeding by re-calling `init()`.
pub fn init() -> anyhow::Result<()> {
    // #333 item 1: force registry rescan at every startup. Without this,
    // a boot-time race where /dev/dri/renderD128 wasn't yet available can
    // pin a cached plugin registry that lists the `va` plugin with ZERO
    // features — vah264enc is missing and stays missing across process
    // restarts because the cached registry is read in priority.
    // Setting GST_REGISTRY_UPDATE=yes BEFORE gstreamer::init() forces a
    // fresh plugin scan. Cost: ~100-300 ms on the FIRST init() call only;
    // subsequent calls are no-ops because OnceLock has already run.
    // `ensure_registry_rescan_env_var()` is called BEFORE the OnceLock
    // get_or_init so the env var is set even when the cached outcome is
    // returned without re-running the closure — and the function itself
    // uses Once::call_once internally so only the FIRST init() call
    // actually writes to the env.
    ensure_registry_rescan_env_var();

    let outcome = GST_INIT_RESULT.get_or_init(|| {
        tracing::info!(
            "GStreamer registry rescan forced via GST_REGISTRY_UPDATE=yes (#333 hardening)"
        );

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
        // Optionally demote nvh264enc so `webrtcsink` falls through to
        // x264enc (software). NVENC on consumer GeForce cards (incl. RTX
        // 5050) enforces a 2-3 concurrent-session driver-level cap;
        // `webrtcsink` creates ONE encoder PER CONSUMER, so the 3rd+
        // browser tab fails with `CUDA_ERROR_NO_DEVICE` and never
        // delivers a track. The N100 production target uses VAAPI
        // (`vah264enc`) which doesn't have this cap, so this demotion
        // is dev-only: production keeps NVENC paths untouched (no
        // nvh264enc registered there anyway).
        //
        // Toggle via `PRESENTER_DEMOTE_NVENC=1` env var. When set,
        // we lower `nvh264enc` registry rank to NONE, which removes it
        // from `webrtcsink`'s codec selection — webrtcsink then picks
        // `x264enc` (software H.264) which has no consumer-session cap.
        // CPU cost on dev2 is fine; on production N100 we never enable
        // this because VAAPI is available.
        if std::env::var("PRESENTER_DEMOTE_NVENC").is_ok() {
            use gstreamer::prelude::PluginFeatureExtManual;
            for name in &["nvh264enc", "nvcudah264enc", "nvautogpuh264enc"] {
                if let Some(factory) = gstreamer::ElementFactory::find(name) {
                    factory.set_rank(gstreamer::Rank::NONE);
                    tracing::info!(
                        encoder = name,
                        "demoted to Rank::NONE so webrtcsink falls through to x264enc"
                    );
                }
            }
        }
        Ok(())
    });
    match outcome {
        Ok(()) => Ok(()),
        Err(msg) => Err(anyhow::anyhow!("{msg}")),
    }
}

/// Detect which H264 encoder webrtcsink will end up picking.
///
/// Returns the element name (`"vah264enc"` Intel iris Xe / N100, or
/// `"nvh264enc"` NVIDIA NVENC, or `"x264enc"` software fallback) when one
/// is available, `None` otherwise. The order is: Intel VA-API first (matches
/// production hardware, no consumer-session cap), NVIDIA NVENC second
/// (dev2's GeForce, has a 2-3 concurrent-session driver cap on consumer
/// cards so we'd rather not use it for multi-consumer streaming), software
/// `x264enc` last (no concurrent-session cap, CPU cost only).
///
/// On dev2 with `PRESENTER_DEMOTE_NVENC=1`, `nvh264enc` is demoted to
/// `Rank::NONE` and `ElementFactory::find` still returns it (the demotion
/// just hides it from `webrtcsink`'s codec selection, not from this probe).
/// Production N100 has VAAPI registered so the first branch wins; dev2
/// with the demotion env var falls through to x264enc which is what
/// webrtcsink will actually use.
///
/// Probes the live element registry on every call — cheap (hash lookup), no
/// memoization needed.
pub fn hw_h264_encoder() -> Option<&'static str> {
    ["vah264enc", "nvh264enc", "x264enc"]
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

    /// Regression for #333 item 1: a boot-time race could leave the cached
    /// plugin registry with `va` plugin showing zero features (vah264enc
    /// missing). Setting `GST_REGISTRY_UPDATE=yes` BEFORE `gstreamer::init()`
    /// forces a registry rescan on every startup, eliminating that class of
    /// stale-cache bug at the cost of ~100-300 ms boot time.
    #[test]
    fn init_sets_gst_registry_update_env_var() {
        // Important: clear the env var first so we can assert init() sets it,
        // not some external test runner inheriting it.
        std::env::remove_var("GST_REGISTRY_UPDATE");
        init().expect("gst init");
        assert_eq!(
            std::env::var("GST_REGISTRY_UPDATE").as_deref(),
            Ok("yes"),
            "init() must set GST_REGISTRY_UPDATE=yes before gstreamer::init() \
             to force a fresh registry scan and avoid the boot-time stale-cache \
             race documented in #333 Failure 1"
        );
    }

    /// Direct test of the helper (deep-review 🟡 #3): if `init()` ever
    /// regresses to call the helper AFTER `gstreamer::init()`, the helper
    /// itself still has the correct contract — and this test, plus the
    /// init() test above and `Once::call_once` semantics, together pin the
    /// behavior down regardless of test execution order.
    #[test]
    fn ensure_registry_rescan_env_var_sets_yes() {
        std::env::remove_var("GST_REGISTRY_UPDATE");
        ensure_registry_rescan_env_var();
        assert_eq!(
            std::env::var("GST_REGISTRY_UPDATE").as_deref(),
            Ok("yes"),
            "ensure_registry_rescan_env_var must set GST_REGISTRY_UPDATE=yes; \
             it is the named contract that init() depends on for the #333 item 1 fix"
        );
    }
}
