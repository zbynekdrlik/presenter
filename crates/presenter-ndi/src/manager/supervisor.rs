//! Per-source pipeline supervisor: rebuild loop, rate-limiting / cool-off
//! state machine, and the `NdiManager` recovery methods that spawn and feed
//! it. Split out of the manager god-file (#357).
//!
//! `SupervisorState` is pure data (no async, no I/O) so the backoff/cool-off
//! logic is unit-testable on every CI host — the tests live in the module
//! root (`manager::start_pipeline_state_check_tests`).

use anyhow::{anyhow, Result};

use crate::pipeline::NdiPipeline;

use super::{check_active_entry, NdiManager, StateCheckOutcome};

/// Per-source supervisor bookkeeping: when the last rebuild was attempted,
/// and how many consecutive failures we've seen.
///
/// Pure data — no async, no I/O — so unit-testable on every CI host.
#[derive(Debug)]
pub(in crate::manager) struct SupervisorState {
    last_rebuild_at: std::time::Instant,
    /// 0 while the pipeline is healthy. Incremented by `mark_rebuild_failed`,
    /// reset to 0 by `mark_rebuild_succeeded`.
    consecutive_failures: u32,
}

/// Outcome of `SupervisorState::should_rebuild_now` — drives the supervisor's
/// next sleep duration.
#[derive(Debug)]
pub(in crate::manager) enum RebuildDecision {
    /// Wait this long, then attempt a rebuild. Zero duration means rebuild now.
    ProceedAfter(std::time::Duration),
}

impl SupervisorState {
    pub(in crate::manager) fn new() -> Self {
        Self {
            // Start with last_rebuild far enough in the past that the FIRST
            // rebuild attempt has zero wait.
            last_rebuild_at: std::time::Instant::now() - std::time::Duration::from_secs(3600),
            consecutive_failures: 0,
        }
    }

    pub(in crate::manager) fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// #337: after `COOL_OFF_THRESHOLD` consecutive failures the supervisor
    /// is "cooling off" — it sleeps `COOL_OFF_WINDOW` before each rebuild
    /// attempt instead of the prior 30s exp-backoff cap. Manual reactivation
    /// via the operator UI calls `start_pipeline`, which removes the dead
    /// active-map entry, builds a fresh pipeline, and spawns a NEW
    /// supervisor with a zero counter — so the old supervisor's cool-off
    /// is implicitly cleared. mark_rebuild_succeeded also clears it (a
    /// rebuild that succeeded inside the cool-off window means the fault
    /// self-resolved).
    pub(in crate::manager) fn is_cooling_off(&self) -> bool {
        self.consecutive_failures >= Self::COOL_OFF_THRESHOLD
    }

    /// #337: number of consecutive failures before entering cool-off. Picked
    /// from the issue body — small enough that an unrecoverable fault
    /// stops thrashing within seconds, large enough that a flaky NDI
    /// source (5-second LAN dropout) doesn't immediately cool off.
    const COOL_OFF_THRESHOLD: u32 = 5;

    /// #337: how long the supervisor waits between rebuild attempts once
    /// the threshold is crossed. 5 minutes is the operator-comfortable
    /// retry interval — long enough to stop log spam + CPU churn for
    /// unrecoverable faults; short enough that a self-healing transient
    /// fault (intermittent NDI broadcaster, GPU contention from another
    /// process) recovers without operator intervention.
    const COOL_OFF_WINDOW: std::time::Duration = std::time::Duration::from_secs(5 * 60);

    /// Rate-limit window: minimum 2 seconds between rebuild attempts.
    pub(in crate::manager) fn should_rebuild_now(
        &self,
        now: std::time::Instant,
    ) -> RebuildDecision {
        const RATE_LIMIT: std::time::Duration = std::time::Duration::from_secs(2);
        let since_last = now.duration_since(self.last_rebuild_at);
        if since_last >= RATE_LIMIT {
            RebuildDecision::ProceedAfter(std::time::Duration::ZERO)
        } else {
            RebuildDecision::ProceedAfter(RATE_LIMIT - since_last)
        }
    }

    /// #337: once `COOL_OFF_THRESHOLD` consecutive failures hit, the
    /// supervisor sleeps `COOL_OFF_WINDOW` (5 min) between attempts
    /// instead of the prior 30s exp-backoff cap. This stops log-spam +
    /// CPU churn for unrecoverable faults (encoder vanished, NDI source
    /// removed). Manual reactivation via `start_pipeline` spawns a fresh
    /// supervisor with a zero counter, which is the operator escape hatch.
    pub(in crate::manager) fn backoff_for_failure_count(&self) -> std::time::Duration {
        if self.consecutive_failures < Self::COOL_OFF_THRESHOLD {
            return std::time::Duration::ZERO;
        }
        Self::COOL_OFF_WINDOW
    }

    pub(in crate::manager) fn mark_rebuild_started(&mut self) {
        self.last_rebuild_at = std::time::Instant::now();
    }

    pub(in crate::manager) fn mark_rebuild_succeeded(&mut self) {
        self.consecutive_failures = 0;
    }

    pub(in crate::manager) fn mark_rebuild_failed(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }
}

impl NdiManager {
    /// Spawn the supervisor task for one source. Returns the JoinHandle so
    /// callers can store it in `ActiveSource.supervisor` and abort on stop.
    ///
    /// The supervisor:
    /// - Subscribes to the pipeline's state watcher
    /// - On Errored/Stopped, requests a rebuild via `rebuild_pipeline`
    /// - Rate-limits to 1 rebuild / 2s; enters a 5-minute cool-off after
    ///   `COOL_OFF_THRESHOLD` (5) consecutive failures (#337). During
    ///   cool-off the supervisor sleeps between rebuild attempts but does
    ///   not exit — manual reactivation via `start_pipeline` aborts this
    ///   supervisor and spawns a fresh one with a zero counter.
    /// - Re-subscribes to the FRESH state watcher after each successful
    ///   rebuild (the old watcher's pipeline was dropped)
    /// - Exits when the watcher closes (pipeline dropped) OR the task is
    ///   externally aborted via `stop_pipeline` / operator reactivate
    pub(in crate::manager) fn spawn_supervisor(
        self: &std::sync::Arc<Self>,
        source_id: String,
        ndi_name: String,
        mut watcher: tokio::sync::watch::Receiver<crate::pipeline::PipelineState>,
    ) -> tokio::task::JoinHandle<()> {
        let manager = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let mut state = SupervisorState::new();
            loop {
                // Wait for the next state change.
                if watcher.changed().await.is_err() {
                    // Pipeline dropped — exit cleanly.
                    tracing::debug!(source_id = %source_id, "supervisor: watcher closed, exiting");
                    return;
                }
                let current = watcher.borrow_and_update().clone();
                use crate::pipeline::PipelineState::*;
                match current {
                    Streaming | Starting => {
                        // Healthy transition: reset the consecutive-failure
                        // counter so prior errors don't carry over into the
                        // next backoff cycle if a future fault occurs. The
                        // spec pseudocode shows Starting as a no-op; we treat
                        // it as success here because reaching Starting means
                        // recovery is underway and any prior failure streak
                        // is irrelevant once the new pipeline begins running.
                        state.mark_rebuild_succeeded();
                    }
                    Errored(_) | Stopped => {
                        // Apply rate-limit + backoff.
                        let RebuildDecision::ProceedAfter(wait) =
                            state.should_rebuild_now(std::time::Instant::now());
                        let backoff = state.backoff_for_failure_count();
                        let total = wait.max(backoff);
                        if !total.is_zero() {
                            tracing::warn!(
                                source_id = %source_id,
                                wait_ms = total.as_millis() as u64,
                                consecutive_failures = state.consecutive_failures(),
                                "supervisor: backing off before rebuild"
                            );
                            tokio::time::sleep(total).await;
                        }
                        state.mark_rebuild_started();
                        match manager.rebuild_pipeline(&source_id, &ndi_name).await {
                            Ok(()) => {
                                tracing::info!(source_id = %source_id, "supervisor: rebuild succeeded");
                                // NOTE: do NOT mark_rebuild_succeeded() yet. We only
                                // know the pipeline build returned Ok — we don't yet
                                // know it survived the immediate state-watcher peek
                                // below. If we reset the counter here, the
                                // already_dead branch below would mark_failed and
                                // the counter would oscillate 0 → 1 → 0 → 1 forever,
                                // never crossing the cool-off threshold (deep-review
                                // 🟡 #1, 2026-05-24 PR #340). Reset only AFTER the
                                // peek confirms the new pipeline is alive.
                                // The fresh pipeline has a NEW state watcher; swap ours
                                // so we see ITS transitions, not the dead pipeline's.
                                match manager.state_watcher_for(&source_id).await {
                                    Some(w) => {
                                        watcher = w;
                                        // After re-subscribing, the new watcher's
                                        // "seen" mark is the current value. If the
                                        // fresh pipeline has ALREADY errored in the
                                        // window between active.insert and the
                                        // state_watcher_for clone, changed() would
                                        // block waiting for a further transition
                                        // that never comes. Peek the state now: if
                                        // it's already dead, mark the rebuild as a
                                        // failure and `continue` — that returns to
                                        // the outer loop's changed().await, which
                                        // then blocks until either (a) the dead
                                        // pipeline emits another transition
                                        // (unlikely) or (b) the 30s DB-ticker
                                        // backstop removes the entry and drops the
                                        // state_tx, which makes changed() return
                                        // Err and exits this supervisor cleanly.
                                        // A fresh supervisor is then spawned by
                                        // the ticker's start_pipeline. Worst-case
                                        // recovery window is ~30s, which is the
                                        // intended backstop behavior.
                                        let already_dead = matches!(
                                            *watcher.borrow_and_update(),
                                            Errored(_) | Stopped
                                        );
                                        if already_dead {
                                            // Mark as a failure for backoff bookkeeping —
                                            // the rebuild "succeeded" briefly but the
                                            // pipeline collapsed immediately, which is
                                            // a real failure of recovery.
                                            let was_cooling_off = state.is_cooling_off();
                                            state.mark_rebuild_failed();
                                            if !was_cooling_off && state.is_cooling_off() {
                                                tracing::warn!(
                                                    source_id = %source_id,
                                                    consecutive_failures = state.consecutive_failures(),
                                                    cool_off_minutes = 5,
                                                    "supervisor: NDI source entered cool-off — pausing retries (#337); \
                                                     manual reactivate via operator UI resumes immediately"
                                                );
                                            }
                                            continue;
                                        }
                                        // Pipeline confirmed alive on the new watcher
                                        // → reset the failure streak now (deferred
                                        // from above per deep-review 🟡 #1).
                                        state.mark_rebuild_succeeded();
                                    }
                                    None => {
                                        // Source no longer active (operator deactivated
                                        // between rebuild start and now). Exit.
                                        tracing::debug!(
                                            source_id = %source_id,
                                            "supervisor: source no longer active after rebuild, exiting"
                                        );
                                        return;
                                    }
                                }
                            }
                            Err(e) => {
                                let was_cooling_off = state.is_cooling_off();
                                state.mark_rebuild_failed();
                                tracing::warn!(
                                    source_id = %source_id,
                                    error = %e,
                                    consecutive_failures = state.consecutive_failures(),
                                    "supervisor: rebuild failed"
                                );
                                if !was_cooling_off && state.is_cooling_off() {
                                    tracing::warn!(
                                        source_id = %source_id,
                                        consecutive_failures = state.consecutive_failures(),
                                        cool_off_minutes = 5,
                                        "supervisor: NDI source entered cool-off — pausing retries (#337); \
                                         manual reactivate via operator UI resumes immediately"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    /// Fetch the live `state_watcher` for an active source. Used by the
    /// supervisor to re-subscribe after a successful rebuild (the old
    /// watcher's pipeline is dropped after rebuild).
    pub(in crate::manager) async fn state_watcher_for(
        &self,
        source_id: &str,
    ) -> Option<tokio::sync::watch::Receiver<crate::pipeline::PipelineState>> {
        let active = self.active.lock().await;
        active.get(source_id).map(|s| s.pipeline.state_watcher())
    }

    /// Rebuild the pipeline for a source whose existing entry is dead
    /// (Errored or Stopped). Reuses `check_active_entry` to clear the
    /// dead entry, then builds + starts a fresh pipeline. Does NOT
    /// spawn a new supervisor (the supervisor task that called us is
    /// still alive and will re-subscribe to the new state watcher via
    /// `state_watcher_for`).
    pub(in crate::manager) async fn rebuild_pipeline(
        &self,
        source_id: &str,
        ndi_name: &str,
    ) -> Result<()> {
        let mut active = self.active.lock().await;
        // Force-remove the dead entry. If somehow it has become healthy in
        // the meantime, leave it alone (idempotent).
        if let StateCheckOutcome::Idempotent = check_active_entry(&mut active, source_id).await {
            return Ok(());
        }

        let whep_url = format!("/ndi/whep/{}", source_id);
        let pipeline = NdiPipeline::build(ndi_name, whep_url)?;
        pipeline.start().await?;

        // Wait for the pipeline to reach Streaming — same rationale as
        // start_pipeline: state-watcher replaces the whepserversink pad
        // caps-wait in the new shared-encoder topology.
        let mut watcher = pipeline.state_watcher();
        let streaming_ready = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                let state = watcher.borrow_and_update().clone();
                match state {
                    crate::pipeline::PipelineState::Errored(ref e) => {
                        return Err(anyhow!("pipeline errored: {e}"));
                    }
                    crate::pipeline::PipelineState::Streaming => return Ok(()),
                    _ => {}
                }
                if watcher.changed().await.is_err() {
                    return Err(anyhow!("state watcher closed unexpectedly"));
                }
            }
        })
        .await;

        match streaming_ready {
            Ok(Ok(())) => {
                active.insert(
                    source_id.to_string(),
                    super::ActiveSource {
                        pipeline: std::sync::Arc::new(pipeline),
                        // Supervisor task is reused — it'll fetch the new watcher
                        // from us via `state_watcher_for`.
                        supervisor: None,
                    },
                );
                Ok(())
            }
            Ok(Err(e)) => {
                pipeline.stop().await;
                Err(e)
            }
            Err(_) => {
                pipeline.stop().await;
                Err(anyhow!(
                    "NDI source '{ndi_name}' did not reach Streaming within 8s on rebuild"
                ))
            }
        }
    }
}
