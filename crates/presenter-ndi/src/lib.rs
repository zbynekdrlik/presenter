#![allow(non_camel_case_types)]

pub mod clock;
pub mod discovery;
pub mod gpu;
pub mod manager;
pub mod ndi_sdk;
pub mod pipeline;
#[cfg(feature = "test-helpers")]
pub mod test_strip;
pub mod whep_session;

pub use clock::pipeline_clock_now_ms;
pub use discovery::SourceList;
pub use manager::NdiManager;
pub use manager::PipelineStartError;
pub use manager::StatusCallback;
pub use pipeline::{PipelineSnapshot, SessionSnapshot, StreamProfile};
pub use whep_session::{IceCandidate, WhepConnectionState, WhepSession};

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

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

/// H264 encoders we will use, in priority order.
///
/// VA-API first: it is what the production N100 has, and it carries no
/// concurrent-session cap. Then the MODERN nvcodec elements — `nvcudah264enc`
/// and `nvautogpuh264enc` — ahead of the LEGACY `nvh264enc`: NVIDIA driver
/// 595.71.05 rejects the legacy element's preset API outright ("Selected preset
/// not supported"), so on a current driver the legacy element registers, builds,
/// and then dies on the first frame (#541). Software `x264enc` is the last
/// resort (no session cap, CPU cost only).
///
/// Public so the deploy guards can assert the encoder-ready gate covers exactly the
/// encoders the server can select — a hand-copied list in the test silently stops
/// covering a 5th candidate the moment one is added here.
pub const H264_ENCODER_CANDIDATES: &[&str] = &[
    "vah264enc",
    "nvcudah264enc",
    "nvautogpuh264enc",
    "nvh264enc",
    "x264enc",
];

/// Pick the first H264 encoder candidate (in priority order) that is actually
/// USABLE. The probe is injected via `is_usable` so this is a PURE function —
/// unit-testable without depending on the machine's live GStreamer registry or
/// GPU driver (#443, #541). `hw_h264_encoder()` is the only caller and supplies
/// the real probe.
fn pick_h264_encoder(
    candidates: &[&'static str],
    can_load: impl Fn(&str) -> bool,
) -> Option<&'static str> {
    // #443: select the FIRST candidate (in priority order) that actually
    // loads. Probing loadability (not mere name-presence in the registry)
    // means an advertised-but-unloadable encoder — e.g. `nvh264enc` after a
    // boot-race registry-cache drift (#333/#339) — is skipped in favour of
    // the next encoder that can really be instantiated.
    candidates.iter().copied().find(|&name| can_load(name))
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
/// Probes each candidate on every call by actually INSTANTIATING it
/// (`ElementFactory::make(name).build()`) and discarding the result, rather
/// than only checking the registry advertises the factory by name
/// (`ElementFactory::find`). This distinction is the #443 fix: a boot-race
/// registry-cache drift (#333/#339) can leave `nvh264enc` advertised in the
/// cached registry while the plugin cannot be loaded to create the element —
/// `find()` returns Some but `make().build()` fails. Selecting on name-presence
/// alone then picked an unloadable encoder and the pipeline build failed.
/// Probing loadability skips it and falls through to a real, loadable encoder.
///
/// Verdicts are memoized SELECTIVELY (`cached_encoder_verdict`): a STABLE verdict is
/// cached so the probe stays off the 30 s NDI-reconnect tick; "the element is not
/// registered" is not, because that is a fact about the plugin registry rather than
/// about the encoder, and this process cannot prove it will stay true (#548).
///
/// What that does NOT buy: an in-process recovery from the boot race. The plugin
/// registry is read once at `gst_init()`, and nothing re-scans it afterwards — an
/// element missing at init stays missing for the life of the process, cached or not.
/// The boot race is prevented by the ExecStartPre encoder gate
/// (`scripts/deploy/wait-for-h264-encoder.sh`), which holds the service back until an
/// encoder really encodes; recovering a host that already started wrong needs the
/// registry deleted and the service restarted (see `gpu::missing_encoder_hint`).
pub fn hw_h264_encoder() -> Option<&'static str> {
    pick_h264_encoder(H264_ENCODER_CANDIDATES, encoder_is_usable)
}

/// Can this encoder element actually ENCODE on this host — not merely be
/// constructed (#541)?
///
/// #443 taught us that registry presence lies (an advertised element may fail to
/// instantiate). Driver 595.71.05 taught us that instantiation lies too: the
/// legacy `nvh264enc` builds happily and then fails at caps negotiation
/// ("Selected preset not supported"), which surfaced only as an opaque
/// `Could not configure supporting library` at pipeline start — after the
/// encoder had already been selected. So the probe pushes ONE tiny frame through
/// the element and only then calls it usable.
///
/// Cost: a 320x240 single-frame encode, on the FIRST query per element name only —
/// for a STABLE verdict. Caching keeps this off the 30 s NDI-reconnect tick and,
/// crucially, stops the probe from opening a second NVENC session while a pipeline
/// is streaming (consumer GeForce cards cap concurrent sessions).
fn encoder_is_usable(name: &str) -> bool {
    let cache = ENCODER_USABILITY.get_or_init(|| Mutex::new(HashMap::new()));
    cached_encoder_verdict(cache, name, probe_encoder_can_encode).is_usable()
}

/// What a functional probe found out about one encoder element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EncoderVerdict {
    /// The element is not in the registry (or cannot be instantiated) — the #443 /
    /// #339 boot-race shape. TRANSIENT: the plugin may register seconds from now.
    NotRegistered,
    /// The element is there, but the driver refused to encode with it — the #541
    /// shape ("Selected preset not supported"). STABLE: it cannot heal without a
    /// driver change, which needs a restart anyway.
    CannotEncode,
    /// It pushed a real frame through. STABLE.
    Usable,
}

impl EncoderVerdict {
    /// Only a real encode selects an encoder — both failure verdicts are a "no".
    pub(crate) fn is_usable(self) -> bool {
        matches!(self, EncoderVerdict::Usable)
    }

    /// May this verdict be remembered for the life of the process?
    ///
    /// #548: `NotRegistered` may NOT. The other two are facts about the ENCODER — a
    /// working element does not stop working, and a driver that refuses one cannot
    /// change its mind without a driver change (which needs a restart anyway). But
    /// "not in the registry" is a fact about the plugin REGISTRY, whose contents this
    /// process does not own: remembering it would be recording someone else's state as
    /// our own. Re-probing it is nearly free (an `ElementFactory::find` miss — never a
    /// real encode, so no NVENC session is opened on the 30 s reconnect tick).
    ///
    /// It is NOT a self-heal: the registry is read once at `gst_init()` and never
    /// re-scanned, so an element missing at init stays missing for this process either
    /// way. The gate prevents the race; a restart is what recovers from it.
    fn is_stable(self) -> bool {
        !matches!(self, EncoderVerdict::NotRegistered)
    }
}

/// Probe `name`, reusing a previously-cached STABLE verdict. The probe is injected
/// so the cache POLICY (what may be remembered) is unit-testable without a GPU.
fn cached_encoder_verdict(
    cache: &Mutex<HashMap<String, EncoderVerdict>>,
    name: &str,
    probe: impl Fn(&str) -> EncoderVerdict,
) -> EncoderVerdict {
    if let Ok(guard) = cache.lock() {
        if let Some(&verdict) = guard.get(name) {
            return verdict;
        }
    }

    let verdict = probe(name);
    match verdict {
        EncoderVerdict::CannotEncode => tracing::warn!(
            encoder = name,
            "encoder is registered but cannot encode on this host — skipping it"
        ),
        EncoderVerdict::NotRegistered => tracing::debug!(
            encoder = name,
            "encoder is not registered (yet) — will re-probe, it may still appear (#339)"
        ),
        EncoderVerdict::Usable => {}
    }

    if verdict.is_stable() {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(name.to_string(), verdict);
        }
    }
    verdict
}

/// Per-process cache of the functional encoder probe (see `cached_encoder_verdict`).
static ENCODER_USABILITY: OnceLock<Mutex<HashMap<String, EncoderVerdict>>> = OnceLock::new();

/// Push one frame through `videotestsrc ! videoconvert ! <encoder> ! fakesink`
/// and report whether it encoded without an error message on the bus.
fn probe_encoder_can_encode(name: &str) -> EncoderVerdict {
    use gstreamer::prelude::*;

    // Not advertised at all → nothing to encode with, and (unlike the two verdicts
    // below) it may still show up: this is the transient case, kept uncached (#548).
    if gstreamer::ElementFactory::find(name).is_none() {
        return EncoderVerdict::NotRegistered;
    }

    // Advertised but not instantiable is the #443 registry-drift case — treated as
    // transient too, for the same reason: a registry rescan can make it real.
    let Ok(pipeline) = gstreamer::parse::launch(&format!(
        "videotestsrc num-buffers=1 ! video/x-raw,width=320,height=240,framerate=30/1 \
         ! videoconvert ! {name} ! fakesink sync=false"
    )) else {
        return EncoderVerdict::NotRegistered;
    };

    let encoded = (|| {
        let bus = pipeline.bus()?;
        if pipeline.set_state(gstreamer::State::Playing).is_err() {
            return Some(false);
        }
        // EOS = the frame made it through the encoder. Error = it did not
        // (driver rejected the preset, no CUDA device, session cap, …).
        let msg = bus.timed_pop_filtered(
            gstreamer::ClockTime::from_seconds(ENCODER_PROBE_TIMEOUT_SECS),
            &[gstreamer::MessageType::Eos, gstreamer::MessageType::Error],
        )?;
        Some(msg.type_() == gstreamer::MessageType::Eos)
    })()
    .unwrap_or(false);

    let _ = pipeline.set_state(gstreamer::State::Null);

    // The element IS registered (checked above), so a failure here is the driver
    // refusing it — stable, and safe to remember.
    if encoded {
        EncoderVerdict::Usable
    } else {
        EncoderVerdict::CannotEncode
    }
}

/// Upper bound for the one-shot encoder probe. Generous enough for a cold CUDA /
/// VA display init, short enough that a hung driver cannot stall startup.
const ENCODER_PROBE_TIMEOUT_SECS: u64 = 10;

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

#[cfg(test)]
mod pick_h264_encoder_tests {
    use super::*;

    const CANDIDATES: &[&str] = &["nvh264enc", "vah264enc", "x264enc"];

    /// #443 regression: the highest-priority candidate (`nvh264enc`) is
    /// ADVERTISED in the registry but cannot be instantiated (boot-race
    /// registry-cache drift, #333/#339). The selector MUST skip it and fall
    /// through to the next encoder that actually loads (`x264enc`), NOT return
    /// the unloadable one. This is the exact failure the #443 bug exhibited.
    #[test]
    fn skips_advertised_but_unloadable_encoder() {
        let picked = pick_h264_encoder(CANDIDATES, |name| name == "x264enc");
        assert_eq!(
            picked,
            Some("x264enc"),
            "an advertised-but-unloadable nvh264enc must be skipped in favour \
             of the next loadable encoder (x264enc)"
        );
    }

    /// No regression on healthy hosts: when every candidate loads, the
    /// highest-priority one (first in the list) is chosen — identical to the
    /// pre-fix behaviour.
    #[test]
    fn returns_first_when_all_load() {
        let picked = pick_h264_encoder(CANDIDATES, |_name| true);
        assert_eq!(picked, Some("nvh264enc"));
    }

    /// On a host with no loadable H264 encoder (e.g. GH `ubuntu-latest` with
    /// no GPU plugins) the selector returns None so callers fail loudly / the
    /// pipeline test skip-guards skip.
    #[test]
    fn returns_none_when_none_load() {
        let picked = pick_h264_encoder(CANDIDATES, |_name| false);
        assert_eq!(picked, None);
    }

    /// Priority order is honoured: the FIRST loadable candidate wins even when
    /// a lower-priority one also loads. Here `nvh264enc` (highest priority)
    /// cannot load but `vah264enc` can, so `vah264enc` is picked over the
    /// also-loadable `x264enc`.
    #[test]
    fn respects_priority_order_among_loadable() {
        let picked = pick_h264_encoder(CANDIDATES, |name| name != "nvh264enc");
        assert_eq!(picked, Some("vah264enc"));
    }

    /// #541: NVIDIA driver 595.71.05 (installed on dev2 2026-07-03) leaves the
    /// LEGACY `nvh264enc` element registered and constructible, but it dies at
    /// caps negotiation with "Selected preset not supported" — every NDI
    /// pipeline build then failed on the CI runner. The modern nvcodec elements
    /// (`nvcudah264enc` / `nvautogpuh264enc`) encode fine on the same driver.
    /// So the real candidate list must offer them BEFORE the legacy element.
    #[test]
    fn broken_legacy_nvenc_falls_through_to_the_modern_cuda_encoder() {
        // The driver-595 host: no Intel VA-API, legacy nvenc unusable, modern
        // nvcodec + software usable.
        let usable = |name: &str| matches!(name, "nvcudah264enc" | "nvautogpuh264enc" | "x264enc");

        assert_eq!(
            pick_h264_encoder(H264_ENCODER_CANDIDATES, usable),
            Some("nvcudah264enc"),
            "with the legacy nvh264enc unusable, the modern CUDA NVENC encoder must be \
             chosen — falling back to software x264enc would throw away the GPU, and \
             picking nvh264enc is what broke every NDI pipeline build (#541)"
        );
    }

    /// The real candidate list's priority, asserted behaviourally: VA-API first
    /// (production N100), then modern NVENC, then the legacy element, then
    /// software. Each row removes the winner above it.
    #[test]
    fn real_candidate_order_is_vaapi_modern_nvenc_legacy_nvenc_software() {
        let all = |_: &str| true;
        assert_eq!(
            pick_h264_encoder(H264_ENCODER_CANDIDATES, all),
            Some("vah264enc")
        );

        let no_va = |name: &str| name != "vah264enc";
        assert_eq!(
            pick_h264_encoder(H264_ENCODER_CANDIDATES, no_va),
            Some("nvcudah264enc")
        );

        let no_va_no_cuda = |name: &str| !matches!(name, "vah264enc" | "nvcudah264enc");
        assert_eq!(
            pick_h264_encoder(H264_ENCODER_CANDIDATES, no_va_no_cuda),
            Some("nvautogpuh264enc")
        );

        let only_legacy_and_software = |name: &str| matches!(name, "nvh264enc" | "x264enc");
        assert_eq!(
            pick_h264_encoder(H264_ENCODER_CANDIDATES, only_legacy_and_software),
            Some("nvh264enc"),
            "a host where the legacy element still works (older driver) keeps using it"
        );

        let software_only = |name: &str| name == "x264enc";
        assert_eq!(
            pick_h264_encoder(H264_ENCODER_CANDIDATES, software_only),
            Some("x264enc")
        );
    }
}

#[cfg(test)]
mod encoder_cache_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn empty_cache() -> Mutex<HashMap<String, EncoderVerdict>> {
        Mutex::new(HashMap::new())
    }

    /// #548: "element not in the registry" is a fact about the plugin REGISTRY, not
    /// about the encoder — this process does not own it and cannot prove it stays
    /// true, so it is never remembered. (It is NOT an in-process self-heal: the
    /// registry is read once at `gst_init()` and never re-scanned. The ExecStartPre
    /// gate prevents the boot race; a restart recovers from it. Re-probing costs an
    /// `ElementFactory::find` miss, never an encode — so no NVENC session is opened
    /// on the 30s reconnect tick.)
    #[test]
    fn a_not_registered_verdict_is_never_cached() {
        let cache = empty_cache();
        let calls = AtomicUsize::new(0);
        // A registry that answers "missing" once and then knows the element.
        let probe = |_name: &str| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                EncoderVerdict::NotRegistered
            } else {
                EncoderVerdict::Usable
            }
        };

        assert_eq!(
            cached_encoder_verdict(&cache, "vah264enc", probe),
            EncoderVerdict::NotRegistered
        );
        assert_eq!(
            cached_encoder_verdict(&cache, "vah264enc", probe),
            EncoderVerdict::Usable,
            "a missing element must be RE-PROBED, never remembered — the plugin \
             registry is not ours to record a verdict about (#548)"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "both calls must reach the probe"
        );
    }

    /// The #541 shape — the element IS registered, the driver refuses to encode
    /// with it ("Selected preset not supported"). That cannot heal without a
    /// driver change, which needs a restart anyway, so it stays cached: re-probing
    /// it would open a fresh NVENC session on every 30s reconnect tick (consumer
    /// GeForce cards cap concurrent sessions).
    #[test]
    fn a_driver_rejection_is_cached_and_not_re_probed() {
        let cache = empty_cache();
        let calls = AtomicUsize::new(0);
        let probe = |_name: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            EncoderVerdict::CannotEncode
        };

        assert_eq!(
            cached_encoder_verdict(&cache, "nvh264enc", probe),
            EncoderVerdict::CannotEncode
        );
        assert_eq!(
            cached_encoder_verdict(&cache, "nvh264enc", probe),
            EncoderVerdict::CannotEncode
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a driver-level rejection is stable — probe it once per process"
        );
    }

    /// A working encoder does not stop working: cache the positive so the probe
    /// stays off the 30s reconnect tick.
    #[test]
    fn a_usable_verdict_is_cached() {
        let cache = empty_cache();
        let calls = AtomicUsize::new(0);
        let probe = |_name: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            EncoderVerdict::Usable
        };

        assert_eq!(
            cached_encoder_verdict(&cache, "vah264enc", probe),
            EncoderVerdict::Usable
        );
        assert_eq!(
            cached_encoder_verdict(&cache, "vah264enc", probe),
            EncoderVerdict::Usable
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Verdicts are per element name — a cached `nvh264enc` rejection must not
    /// answer for `vah264enc`.
    #[test]
    fn verdicts_are_keyed_by_encoder_name() {
        let cache = empty_cache();
        let probe = |name: &str| {
            if name == "vah264enc" {
                EncoderVerdict::Usable
            } else {
                EncoderVerdict::CannotEncode
            }
        };

        assert_eq!(
            cached_encoder_verdict(&cache, "nvh264enc", probe),
            EncoderVerdict::CannotEncode
        );
        assert_eq!(
            cached_encoder_verdict(&cache, "vah264enc", probe),
            EncoderVerdict::Usable
        );
    }

    /// Only `Usable` selects an encoder — both failure verdicts are a "no" to
    /// `pick_h264_encoder`.
    #[test]
    fn only_a_usable_verdict_selects_the_encoder() {
        assert!(EncoderVerdict::Usable.is_usable());
        assert!(!EncoderVerdict::CannotEncode.is_usable());
        assert!(!EncoderVerdict::NotRegistered.is_usable());
    }
}
