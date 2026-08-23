//! Reservation-based pipeline activation seam (#741).
//!
//! `start_pipeline` / `rebuild_pipeline` used to hold the `active` mutex across
//! their ~8 s streaming-ready wait, so EVERY contender — the status-snapshot
//! reader (`pipeline_snapshots_checked`, → operator dashboard + `/healthz`),
//! `stop_*`, `periodic_reap`, `pipeline_snapshot(:id)`, and every WHEP
//! POST/PATCH/DELETE — stalled up to 8 s behind each activation.
//!
//! This module carries the two seam helpers that let the wait happen WITHOUT the
//! lock, so the lock is held only for the brief reserve (check + build + start +
//! insert a `Starting` entry) and the brief finalize. The `Starting` entry (a) is
//! the per-source in-flight marker — a concurrent `start` for the same source
//! sees it and observer-joins instead of building a second pipeline — and (b)
//! makes the status reader show the source as `Starting` (→ `Connecting`, not the
//! alarming "no pipeline") during the wait, preserving #546.
//!
//! Both helpers are pure w.r.t. libndi (they take a `watch::Receiver` / a
//! `Mutex<HashMap>` + an `Arc<NdiPipeline>`), so they are unit-testable on every
//! CI host via `NdiPipeline::stopped_for_test()` — no SDK/GPU/gst-plugins needed.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{watch, Mutex};

use crate::pipeline::{NdiPipeline, PipelineState};

use super::ActiveSource;

/// Result of the streaming-ready wait for a reserved pipeline.
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub(in crate::manager) enum WaitOutcome {
    /// Reached `Streaming` within the budget.
    Streaming,
    /// The pipeline posted an error (encoder/element fault).
    Errored(String),
    /// The pipeline stopped (EOS / stopped out from under us) — returned
    /// IMMEDIATELY (the old inline wait's catch-all waited the full budget on a
    /// stopped pipeline).
    Stopped,
    /// Never reached `Streaming` within the budget — broadcaster silent / not
    /// producing (#448).
    TimedOut,
}

/// Result of finalizing an in-flight reservation after the (unlocked) wait.
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub(in crate::manager) enum Finalize {
    /// The reservation is still ours AND reached `Streaming` — the slot is left
    /// in place (the caller spawns/keeps the supervisor).
    Promote,
    /// The reservation is still ours but did NOT reach `Streaming` — the slot has
    /// been removed and the pipeline stopped; the caller reports the error.
    Removed(WaitOutcome),
    /// The slot is gone (a concurrent `stop`) or now holds a DIFFERENT pipeline
    /// (a concurrent activate-switch) — the map is left untouched and our orphan
    /// pipeline stopped; the caller must not publish a status for it.
    Superseded,
}

/// Wait — WITHOUT holding the `active` lock — for `rx`'s pipeline to reach
/// `Streaming`, bounded by `budget`. Returns as soon as the pipeline reaches a
/// terminal-for-startup state (Streaming / Errored / Stopped), or `TimedOut`
/// after the budget.
pub(in crate::manager) async fn wait_for_streaming(
    mut rx: watch::Receiver<PipelineState>,
    budget: Duration,
) -> WaitOutcome {
    let waited = tokio::time::timeout(budget, async {
        loop {
            let state = rx.borrow_and_update().clone();
            match state {
                PipelineState::Streaming => return WaitOutcome::Streaming,
                PipelineState::Errored(e) => return WaitOutcome::Errored(e),
                // #741: return immediately on Stopped — the old inline wait's
                // catch-all waited the full budget on a pipeline stopped/EOS'd
                // out from under it.
                PipelineState::Stopped => return WaitOutcome::Stopped,
                PipelineState::Starting => {}
            }
            if rx.changed().await.is_err() {
                // Sender dropped (pipeline gone) — treat as Stopped, never hang.
                return WaitOutcome::Stopped;
            }
        }
    })
    .await;
    waited.unwrap_or(WaitOutcome::TimedOut)
}

/// Finalize an in-flight reservation under the `active` lock. `ours` is the
/// pipeline Arc the caller reserved; `outcome` is what the (unlocked) wait
/// observed. See [`Finalize`]. Any pipeline this removes/orphans is stopped
/// OUTSIDE the lock so the finalize never holds `active` across `stop().await`.
pub(in crate::manager) async fn finalize_reservation(
    active: &Mutex<HashMap<String, ActiveSource>>,
    source_id: &str,
    ours: &Arc<NdiPipeline>,
    outcome: WaitOutcome,
) -> Finalize {
    // Decide under the lock; STOP any removed/orphaned pipeline OUTSIDE it so the
    // finalize never holds `active` across `stop().await` (the whole #741 point).
    let removed: Option<WaitOutcome> = {
        let mut guard = active.lock().await;
        let is_ours = guard
            .get(source_id)
            .map(|slot| Arc::ptr_eq(&slot.pipeline, ours))
            .unwrap_or(false);
        if !is_ours {
            // Slot gone (concurrent stop) or now a DIFFERENT pipeline (concurrent
            // activate-switch) — leave the map untouched.
            None
        } else {
            match outcome {
                // Ours and Streaming → leave the slot for the caller to promote.
                WaitOutcome::Streaming => return Finalize::Promote,
                // Ours but not Streaming → drop the slot; stop the pipeline below.
                other => {
                    guard.remove(source_id);
                    Some(other)
                }
            }
        }
    }; // guard dropped here — stop() runs unlocked
    match removed {
        Some(o) => {
            ours.stop().await;
            Finalize::Removed(o)
        }
        None => {
            // Our orphan pipeline is not in the map — stop it (idempotent; Drop
            // would also tear it down) so a superseded start never leaks it.
            ours.stop().await;
            Finalize::Superseded
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stopped_arc() -> Arc<NdiPipeline> {
        Arc::new(NdiPipeline::stopped_for_test())
    }

    fn reserve(map: &mut HashMap<String, ActiveSource>, id: &str, p: &Arc<NdiPipeline>) {
        map.insert(
            id.to_string(),
            ActiveSource {
                pipeline: Arc::clone(p),
                supervisor: None,
            },
        );
    }

    #[tokio::test]
    async fn wait_returns_streaming() {
        let (tx, rx) = watch::channel(PipelineState::Starting);
        tx.send(PipelineState::Streaming).unwrap();
        assert_eq!(
            wait_for_streaming(rx, Duration::from_secs(1)).await,
            WaitOutcome::Streaming
        );
    }

    #[tokio::test]
    async fn wait_returns_errored_with_message() {
        let (tx, rx) = watch::channel(PipelineState::Starting);
        tx.send(PipelineState::Errored("boom".to_string())).unwrap();
        assert_eq!(
            wait_for_streaming(rx, Duration::from_secs(1)).await,
            WaitOutcome::Errored("boom".to_string())
        );
    }

    // #741: the new Stopped arm returns immediately — the old inline wait's
    // catch-all waited the FULL budget on a pipeline stopped out from under it.
    #[tokio::test]
    async fn wait_returns_stopped_promptly_not_after_budget() {
        let (tx, rx) = watch::channel(PipelineState::Starting);
        tx.send(PipelineState::Stopped).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            wait_for_streaming(rx, Duration::from_secs(30)).await,
            WaitOutcome::Stopped
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "Stopped must return promptly, not wait the whole budget",
        );
    }

    #[tokio::test]
    async fn wait_times_out_when_stuck_starting() {
        // Keep the sender alive so the channel stays open and the state never
        // leaves Starting → the budget must elapse to TimedOut.
        let (_tx, rx) = watch::channel(PipelineState::Starting);
        assert_eq!(
            wait_for_streaming(rx, Duration::from_millis(50)).await,
            WaitOutcome::TimedOut
        );
    }

    #[tokio::test]
    async fn wait_reports_stopped_when_watcher_closes() {
        let (tx, rx) = watch::channel(PipelineState::Starting);
        drop(tx); // sender gone → changed() errors → treat as Stopped, not a hang.
        assert_eq!(
            wait_for_streaming(rx, Duration::from_secs(1)).await,
            WaitOutcome::Stopped
        );
    }

    #[tokio::test]
    async fn finalize_promotes_our_streaming_reservation() {
        let p = stopped_arc();
        let active = Mutex::new(HashMap::new());
        {
            let mut g = active.lock().await;
            reserve(&mut g, "A", &p);
        }
        assert_eq!(
            finalize_reservation(&active, "A", &p, WaitOutcome::Streaming).await,
            Finalize::Promote
        );
        assert!(
            active.lock().await.contains_key("A"),
            "Promote must leave the slot in place",
        );
    }

    #[tokio::test]
    async fn finalize_removes_our_errored_reservation() {
        let p = stopped_arc();
        let active = Mutex::new(HashMap::new());
        {
            let mut g = active.lock().await;
            reserve(&mut g, "A", &p);
        }
        assert_eq!(
            finalize_reservation(&active, "A", &p, WaitOutcome::Errored("x".to_string())).await,
            Finalize::Removed(WaitOutcome::Errored("x".to_string()))
        );
        assert!(
            !active.lock().await.contains_key("A"),
            "Removed must drop the slot",
        );
    }

    #[tokio::test]
    async fn finalize_removes_our_timed_out_reservation() {
        let p = stopped_arc();
        let active = Mutex::new(HashMap::new());
        {
            let mut g = active.lock().await;
            reserve(&mut g, "A", &p);
        }
        assert_eq!(
            finalize_reservation(&active, "A", &p, WaitOutcome::TimedOut).await,
            Finalize::Removed(WaitOutcome::TimedOut)
        );
        assert!(!active.lock().await.contains_key("A"));
    }

    // A concurrent activate-switch replaced our slot with a DIFFERENT pipeline
    // before we finalized → Superseded, and the superseding slot is untouched.
    #[tokio::test]
    async fn finalize_supersedes_when_slot_replaced() {
        let ours = stopped_arc();
        let theirs = stopped_arc();
        let active = Mutex::new(HashMap::new());
        {
            let mut g = active.lock().await;
            reserve(&mut g, "A", &theirs);
        }
        assert_eq!(
            finalize_reservation(&active, "A", &ours, WaitOutcome::Streaming).await,
            Finalize::Superseded
        );
        let g = active.lock().await;
        assert!(
            Arc::ptr_eq(&g.get("A").expect("slot present").pipeline, &theirs),
            "the superseding slot must be left untouched",
        );
    }

    // A concurrent stop removed our slot before we finalized → Superseded.
    #[tokio::test]
    async fn finalize_supersedes_when_slot_absent() {
        let ours = stopped_arc();
        let active: Mutex<HashMap<String, ActiveSource>> = Mutex::new(HashMap::new());
        assert_eq!(
            finalize_reservation(&active, "A", &ours, WaitOutcome::Streaming).await,
            Finalize::Superseded
        );
    }
}
