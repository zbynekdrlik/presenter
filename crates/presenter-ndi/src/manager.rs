//! NdiManager — owns discovery + per-source GStreamer pipelines.
//!
//! Pre-WebRTC the module hosted a custom JPEG receiver/encoder. After the
//! #336 shared-encoder migration it manages one `NdiPipeline` per active NDI
//! source and bridges WHEP HTTP operations into direct
//! `pipeline.add_consumer` / `add_ice_candidate` / `remove_consumer` calls
//! (no `whepserversink` `emit_by_name`).
//!
//! The implementation is split across responsibility-focused submodules
//! (see #357 — split the over-cap god-file):
//! - [`lifecycle`] — construction, discovery, start/stop, active-map queries
//! - [`supervisor`] — per-source rebuild supervisor + backoff/cool-off state
//! - [`whep`] — WHEP HTTP bridge + pipeline snapshots
//!
//! The shared type definitions and the active-map state-check live here in the
//! module root so every submodule (and the regression tests) can reference
//! them at a stable path.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::discovery::{FinderShutdown, SourceList};
use crate::ndi_sdk::NdiLib;
use crate::pipeline::{NdiPipeline, PipelineState, StreamProfile};

mod lifecycle;
mod supervisor;
mod whep;

/// Status callback retained for backwards compatibility with the old MJPEG
/// status-reporting path. The WebRTC manager currently invokes it on
/// pipeline state transitions so the live-event hub keeps emitting
/// `NdiConnectionStatus` events.
pub type StatusCallback = Arc<dyn Fn(String) + Send + Sync>;

/// Sentinel error message returned by `whep_signaller_call` when the requested
/// source has no active pipeline. The WHEP HTTP shim string-matches on this
/// to translate the error into a 404. Exposed as a `pub const` so the shim
/// imports the same literal — preventing silent 503-instead-of-404 drift if
/// the message is ever rewritten.
pub const SOURCE_NOT_ACTIVE_ERR: &str = "source not active";

/// One operation in the WHEP signaller protocol.
pub enum WhepOp {
    /// SDP offer (or session-scoped re-offer). `profile` is parsed from the
    /// `?profile=` query at the HTTP layer; only the single shared 720p H264
    /// stream ships, so it always resolves to that stream.
    Post {
        id: Option<String>,
        body: Vec<u8>,
        profile: StreamProfile,
    },
    /// ICE trickle update.
    Patch {
        id: String,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
    },
    /// Explicit session teardown.
    Delete { id: String },
}

/// Reply built by `whep_signaller_call` from a pipeline operation result.
/// Status, headers, and body are mapped 1:1 by the axum HTTP shim into
/// the actual HTTP response.
pub struct WhepReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

struct ActiveSource {
    /// Wrapped in `Arc` so WHEP HTTP handlers can clone the pipeline
    /// reference out of the active-map mutex guard before calling potentially
    /// blocking pipeline methods (`add_consumer` spawn_blocks for ~10s,
    /// `add_ice_candidate` / `remove_consumer` also spawn_block). Without
    /// this, holding the active-map lock across those awaits serializes ALL
    /// WHEP operations on the manager, stalls `pipeline_snapshots()` (used
    /// by `/healthz`) and blocks the supervisor's `rebuild_pipeline`.
    pub(in crate::manager) pipeline: std::sync::Arc<NdiPipeline>,
    /// Supervisor task handle. Aborted on `stop_pipeline` / drop to prevent
    /// leaks. `None` only inside the regression-test constructors (which
    /// don't spawn a real supervisor) AND in the `rebuild_pipeline` re-insert
    /// path (the existing supervisor task is reused — see `spawn_supervisor`).
    pub(in crate::manager) supervisor: Option<tokio::task::JoinHandle<()>>,
}

/// Outcome of `check_active_entry` — drives `start_pipeline`'s control flow.
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
enum StateCheckOutcome {
    /// Active entry exists and the pipeline is healthy (Streaming or
    /// Starting). Caller should treat the request as a no-op.
    Idempotent,
    /// No entry, OR the existing entry's pipeline is dead (Stopped or
    /// Errored). In the dead case the entry has already been removed.
    /// Caller should proceed to build a fresh pipeline.
    Rebuild,
}

/// Pure state-check for the active-source HashMap. Extracted from
/// `start_pipeline` so the regression test for the "dead pipeline left
/// in HashMap" bug can run without libndi/GPU/gst-plugins — see
/// `start_pipeline_state_check_tests` below.
///
/// Idempotency check must inspect the LIVE pipeline state, not just
/// HashMap presence. A pipeline that transitioned to `Stopped` (NDI
/// broadcaster EOS) or `Errored` (ndisrc fault) keeps its HashMap entry
/// alive — without this state check, both the manual re-activate path
/// and the 30s auto-reconnect loop (state/mod.rs) early-return `Ok` and
/// leave the dead pipeline sitting in the slot. The next WHEP POST then
/// sees `PipelineState::Stopped` and 503s the client, with no recovery
/// path short of an operator-driven `deactivate + activate` cycle.
/// Treat Streaming/Starting as a true idempotent no-op; treat
/// Stopped/Errored as dead and rebuild from scratch.
async fn check_active_entry(
    active: &mut HashMap<String, ActiveSource>,
    source_id: &str,
) -> StateCheckOutcome {
    if let Some(existing) = active.get(source_id) {
        match existing.pipeline.state() {
            PipelineState::Streaming | PipelineState::Starting => {
                return StateCheckOutcome::Idempotent;
            }
            PipelineState::Stopped | PipelineState::Errored(_) => {
                if let Some(dead) = active.remove(source_id) {
                    // CRITICAL: do NOT abort `dead.supervisor` here. The
                    // supervisor task is what CALLS `rebuild_pipeline ->
                    // check_active_entry` in the recovery path, so aborting
                    // its own JoinHandle would self-cancel at the next
                    // `.await` (pipeline.start / caps_ready) — orphaning the
                    // new pipeline we're about to build and leaving the
                    // active map empty. The supervisor's lifecycle is owned
                    // by `stop_pipeline` / `stop_all` (explicit deactivation
                    // paths only) — never by the rebuild path.
                    //
                    // After this Drops, `dead.supervisor: Option<JoinHandle>`
                    // is dropped too. Dropping a JoinHandle does NOT cancel
                    // its task in tokio (unlike abort), so the task keeps
                    // running — which is exactly what we need for the
                    // self-rebuild path. `rebuild_pipeline` then re-inserts
                    // a fresh ActiveSource with `supervisor: None`, and the
                    // still-running supervisor re-subscribes to the new
                    // pipeline's state_watcher via `state_watcher_for`.
                    dead.pipeline.stop().await;
                }
            }
        }
    }
    StateCheckOutcome::Rebuild
}

/// Remove every active-map entry whose `source_id` is NOT `keep_id`, stopping
/// each removed pipeline (and aborting its supervisor). Extracted from the
/// activate-switch path so the regression test for the "old source pipeline
/// leaks on switch" bug (#370) can run without libndi/GPU/gst-plugins — see
/// `stop_other_pipelines_tests` below.
///
/// #370: switching the active video source (deactivate A → activate B) used to
/// start B's pipeline while leaving A's pipeline + its `nvh264enc` encoder
/// streaming forever. The DB flipped A's sibling row to `is_active=false` but
/// the manager was never told, so two source pipelines (= two NVENC encoders)
/// kept running after every switch — NVENC contention + wasted GPU. This helper
/// reaps the orphaned siblings so exactly ONE source pipeline remains.
///
/// STUB (RED): currently a no-op — leaves every sibling in the map, reproducing
/// the leak. The GREEN commit makes it actually remove + stop the siblings.
async fn retain_only_active(_active: &mut HashMap<String, ActiveSource>, _keep_id: &str) {
    // RED placeholder — does nothing yet (the bug). GREEN fix follows.
}

pub struct NdiManager {
    pub(in crate::manager) _sdk: Arc<NdiLib>,
    pub(in crate::manager) source_list: SourceList,
    pub(in crate::manager) _finder_shutdown: FinderShutdown,
    /// Map source_id (UUID string) → ActiveSource pipeline.
    pub(in crate::manager) active: Mutex<HashMap<String, ActiveSource>>,
}

// Re-export the supervisor's pure-state types into the module root so the
// regression tests below (which use `super::*`) keep resolving them at their
// original `manager::` paths. These types are crate-internal — gating the
// re-export on `#[cfg(test)]` keeps the production build free of an
// otherwise-unused import under clippy `-D warnings`.
#[cfg(test)]
use self::supervisor::{RebuildDecision, SupervisorState};

#[cfg(test)]
mod start_pipeline_state_check_tests {
    use super::*;

    /// Empty HashMap → `Rebuild` outcome. Trivial case.
    #[tokio::test]
    async fn empty_map_requests_rebuild() {
        let mut active = HashMap::new();
        let outcome = check_active_entry(&mut active, "any-id").await;
        assert_eq!(outcome, StateCheckOutcome::Rebuild);
        assert!(active.is_empty());
    }

    /// REGRESSION TEST for the production bug surfaced 2026-05-20: an entry
    /// in the HashMap whose pipeline transitioned to `Stopped` (NDI
    /// broadcaster EOS) must be REMOVED so the caller rebuilds. The buggy
    /// version of start_pipeline early-returned Ok on HashMap presence
    /// alone, leaving the dead entry alive forever — WHEP POSTs then
    /// 503'd with "pipeline stopped" and recovery required a manual
    /// deactivate+activate cycle.
    ///
    /// Runs on every CI host (no libndi, no GPU, no gst-plugins required) —
    /// uses `NdiPipeline::stopped_for_test()` which constructs a
    /// pipeline-shaped value in `Stopped` state without invoking real
    /// GStreamer element building.
    #[tokio::test]
    async fn dead_stopped_entry_is_removed_and_rebuild_requested() {
        let mut active: HashMap<String, ActiveSource> = HashMap::new();
        let dead = crate::pipeline::NdiPipeline::stopped_for_test();
        assert_eq!(
            dead.state(),
            PipelineState::Stopped,
            "precondition: stopped_for_test must yield a Stopped pipeline",
        );
        active.insert(
            "test-id".to_string(),
            ActiveSource {
                pipeline: std::sync::Arc::new(dead),
                supervisor: None,
            },
        );
        assert!(active.contains_key("test-id"));

        let outcome = check_active_entry(&mut active, "test-id").await;

        assert_eq!(
            outcome,
            StateCheckOutcome::Rebuild,
            "REGRESSION: dead Stopped entry must trigger Rebuild, not Idempotent",
        );
        assert!(
            !active.contains_key("test-id"),
            "REGRESSION: dead Stopped entry must be removed from the active map",
        );
    }

    /// `Streaming` entry → true idempotent no-op. Confirms the healthy
    /// path is preserved (we don't accidentally remove live pipelines).
    #[tokio::test]
    async fn streaming_entry_is_left_alone_idempotent() {
        let mut active: HashMap<String, ActiveSource> = HashMap::new();
        let mut p = crate::pipeline::NdiPipeline::stopped_for_test();
        p.set_state_for_test(PipelineState::Streaming);
        assert_eq!(p.state(), PipelineState::Streaming);
        active.insert(
            "test-id".to_string(),
            ActiveSource {
                pipeline: std::sync::Arc::new(p),
                supervisor: None,
            },
        );

        let outcome = check_active_entry(&mut active, "test-id").await;

        assert_eq!(outcome, StateCheckOutcome::Idempotent);
        assert!(
            active.contains_key("test-id"),
            "Streaming entry must NOT be removed — that's the idempotent path",
        );
    }

    /// `Errored` entry → same outcome as Stopped: remove + rebuild. Catches
    /// regressions that only handle the Stopped variant.
    #[tokio::test]
    async fn errored_entry_is_removed_and_rebuild_requested() {
        let mut active: HashMap<String, ActiveSource> = HashMap::new();
        let mut p = crate::pipeline::NdiPipeline::stopped_for_test();
        p.set_state_for_test(PipelineState::Errored("ndisrc fault".to_string()));
        active.insert(
            "test-id".to_string(),
            ActiveSource {
                pipeline: std::sync::Arc::new(p),
                supervisor: None,
            },
        );

        let outcome = check_active_entry(&mut active, "test-id").await;

        assert_eq!(outcome, StateCheckOutcome::Rebuild);
        assert!(!active.contains_key("test-id"));
    }

    /// Rate-limiter: two Errored transitions within 2s must produce
    /// exactly ONE rebuild attempt.
    #[tokio::test]
    async fn supervisor_rate_limits_rapid_errors() {
        let mut state = SupervisorState::new();
        let outcome1 = state.should_rebuild_now(std::time::Instant::now());
        assert!(matches!(outcome1, RebuildDecision::ProceedAfter(d) if d.is_zero()));
        state.mark_rebuild_started();

        // 100ms later — well within the 2s rate limit.
        let outcome2 = state
            .should_rebuild_now(std::time::Instant::now() + std::time::Duration::from_millis(100));
        // Decision must defer to "after the rate-limit window".
        // Allow a small tolerance (50ms) for the real time elapsed between
        // mark_rebuild_started() and the should_rebuild_now() call.
        match outcome2 {
            RebuildDecision::ProceedAfter(d) => {
                assert!(
                    d >= std::time::Duration::from_millis(1850),
                    "expected ~2s wait, got {d:?}"
                );
            }
        }
    }

    /// #337: prior exp-backoff (2s/4s/8s/16s/30s cap) is replaced with a
    /// flat 5-minute cool-off window once `COOL_OFF_THRESHOLD` failures
    /// hit. The window stays at 5 min regardless of how many further
    /// failures accumulate — no further growth, no risk of an
    /// integer-overflow timer pathology.
    #[tokio::test]
    async fn supervisor_cool_off_window_stays_flat_after_threshold() {
        let mut state = SupervisorState::new();
        for _ in 0..5 {
            state.mark_rebuild_failed();
        }
        assert_eq!(
            state.backoff_for_failure_count(),
            std::time::Duration::from_secs(5 * 60),
            "at threshold (5 failures): cool-off = 5 min"
        );
        for _ in 0..50 {
            state.mark_rebuild_failed();
            assert_eq!(
                state.backoff_for_failure_count(),
                std::time::Duration::from_secs(5 * 60),
                "many failures: cool-off STAYS at 5 min, doesn't grow further"
            );
        }
    }

    /// mark_rebuild_succeeded resets the failure counter.
    #[tokio::test]
    async fn supervisor_resets_on_success() {
        let mut state = SupervisorState::new();
        for _ in 0..3 {
            state.mark_rebuild_failed();
        }
        state.mark_rebuild_succeeded();
        assert_eq!(state.consecutive_failures(), 0);
    }

    /// #337 RED: after 5 consecutive failures, supervisor enters
    /// CoolingOff state. Without an explicit cool-off ceiling, the
    /// 30s-capped exponential backoff retries forever and produces
    /// continuous log spam + repeated encoder-rebuild CPU churn for an
    /// unrecoverable fault (e.g. encoder vanished). With cool-off, the
    /// supervisor pauses for 5 min and lets the operator manually
    /// reactivate to retry sooner.
    ///
    /// FAILS before the GREEN fix in the next commit (is_cooling_off
    /// stub always returns false).
    #[tokio::test]
    async fn supervisor_enters_cool_off_at_5_consecutive_failures() {
        let mut state = SupervisorState::new();
        for _ in 0..4 {
            state.mark_rebuild_failed();
        }
        assert!(
            !state.is_cooling_off(),
            "4 failures must NOT trigger cool-off (threshold is 5)"
        );
        state.mark_rebuild_failed();
        assert!(
            state.is_cooling_off(),
            "5 consecutive failures must trigger cool-off (#337)"
        );
    }

    /// #337 RED: while cooling off, the supervisor must wait 5 minutes
    /// before its next rebuild attempt — NOT the 30s exp-backoff cap.
    #[tokio::test]
    async fn supervisor_cool_off_window_is_five_minutes() {
        let mut state = SupervisorState::new();
        for _ in 0..5 {
            state.mark_rebuild_failed();
        }
        assert_eq!(
            state.backoff_for_failure_count(),
            std::time::Duration::from_secs(5 * 60),
            "cool-off window must be 5 minutes (#337) — not the prior 2s exp-backoff entry"
        );
    }

    /// #337 RED: mark_rebuild_succeeded clears cool-off. Without this,
    /// a manual reactivation that succeeds once would still leave the
    /// counter pinned, sending the next failure straight back into
    /// cool-off.
    #[tokio::test]
    async fn supervisor_cool_off_clears_on_success() {
        let mut state = SupervisorState::new();
        for _ in 0..5 {
            state.mark_rebuild_failed();
        }
        assert!(state.is_cooling_off());
        state.mark_rebuild_succeeded();
        assert!(
            !state.is_cooling_off(),
            "successful rebuild must clear the cool-off flag (#337)"
        );
        assert_eq!(state.consecutive_failures(), 0);
    }
}

/// #370 — regression tests for the activate-switch pipeline leak. The bug:
/// switching the active video source started the new source's pipeline but
/// left the old source's pipeline (and its encoder) running, so two source
/// pipelines accumulated after every switch. `retain_only_active` reaps the
/// siblings so exactly ONE source remains in the active map after a switch.
///
/// Runs on every CI host (no libndi, no GPU, no gst-plugins) using the same
/// `NdiPipeline::stopped_for_test()` shaping the `check_active_entry` tests use.
#[cfg(test)]
mod stop_other_pipelines_tests {
    use super::*;

    fn insert_stopped(active: &mut HashMap<String, ActiveSource>, id: &str) {
        active.insert(
            id.to_string(),
            ActiveSource {
                pipeline: std::sync::Arc::new(crate::pipeline::NdiPipeline::stopped_for_test()),
                supervisor: None,
            },
        );
    }

    /// RED: after activating source B while source A is already active, the
    /// active map must hold EXACTLY ONE source — B. The buggy version (no
    /// sibling reap) left A's pipeline in the map alongside B → two encoders.
    #[tokio::test]
    async fn activate_switch_leaves_exactly_one_source() {
        let mut active: HashMap<String, ActiveSource> = HashMap::new();
        insert_stopped(&mut active, "source-a");
        insert_stopped(&mut active, "source-b");
        assert_eq!(active.len(), 2, "precondition: both A and B in the map");

        // Simulate "activate B": keep B, reap every other source.
        retain_only_active(&mut active, "source-b").await;

        assert_eq!(
            active.len(),
            1,
            "REGRESSION #370: switching to B must leave exactly ONE source pipeline, \
             not leak A's pipeline + encoder alongside B's"
        );
        assert!(
            active.contains_key("source-b"),
            "the newly-activated source B must remain active",
        );
        assert!(
            !active.contains_key("source-a"),
            "REGRESSION #370: the previously-active source A must be stopped on switch",
        );
    }

    /// The kept source survives even when several stale siblings exist (e.g.
    /// rapid switches A→B→C left A and B both leaked). Activating D reaps all.
    #[tokio::test]
    async fn activate_reaps_all_stale_siblings() {
        let mut active: HashMap<String, ActiveSource> = HashMap::new();
        for id in ["source-a", "source-b", "source-c", "source-d"] {
            insert_stopped(&mut active, id);
        }

        retain_only_active(&mut active, "source-d").await;

        assert_eq!(active.len(), 1, "all stale siblings must be reaped");
        assert!(active.contains_key("source-d"));
    }

    /// Re-activating the already-active source is a no-op for the map (it stays
    /// the only entry) — guards against accidentally stopping the kept source.
    #[tokio::test]
    async fn reactivate_same_source_keeps_it() {
        let mut active: HashMap<String, ActiveSource> = HashMap::new();
        insert_stopped(&mut active, "source-a");

        retain_only_active(&mut active, "source-a").await;

        assert_eq!(active.len(), 1);
        assert!(active.contains_key("source-a"));
    }
}
