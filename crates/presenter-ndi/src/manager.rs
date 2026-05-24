//! NdiManager — owns discovery + per-source GStreamer pipelines.
//!
//! Pre-WebRTC the module hosted a custom JPEG receiver/encoder. After the
//! WebRTC migration it manages one `NdiPipeline` per active NDI source and
//! exposes a WHEP signaller bridge for the HTTP shim.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer::glib;
use gstreamer::prelude::*;
use tokio::sync::Mutex;

use crate::discovery::{self, FinderShutdown, SourceList};
use crate::ndi_sdk::NdiLib;
use crate::pipeline::{NdiPipeline, PipelineState};

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
    /// SDP offer (or session-scoped re-offer).
    Post { id: Option<String>, body: Vec<u8> },
    /// ICE trickle update.
    Patch {
        id: String,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
    },
    /// Explicit session teardown.
    Delete { id: String },
}

/// Result returned by `whepserversink`'s signaller, flattened into plain
/// Rust types. Header names and values are extracted from the gstreamer
/// `Structure` inside the manager so consumers (e.g. the axum WHEP router)
/// don't need to depend on gstreamer.
pub struct WhepReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

struct ActiveSource {
    pipeline: NdiPipeline,
    /// Supervisor task handle. Aborted on `stop_pipeline` / drop to prevent
    /// leaks. `None` only inside the regression-test constructors (which
    /// don't spawn a real supervisor) AND in the `rebuild_pipeline` re-insert
    /// path (the existing supervisor task is reused — see `spawn_supervisor`).
    supervisor: Option<tokio::task::JoinHandle<()>>,
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

/// Per-source supervisor bookkeeping: when the last rebuild was attempted,
/// and how many consecutive failures we've seen.
///
/// Pure data — no async, no I/O — so unit-testable on every CI host.
#[derive(Debug)]
struct SupervisorState {
    last_rebuild_at: std::time::Instant,
    /// 0 while the pipeline is healthy. Incremented by `mark_rebuild_failed`,
    /// reset to 0 by `mark_rebuild_succeeded`.
    consecutive_failures: u32,
}

/// Outcome of `SupervisorState::should_rebuild_now` — drives the supervisor's
/// next sleep duration.
#[derive(Debug)]
enum RebuildDecision {
    /// Wait this long, then attempt a rebuild. Zero duration means rebuild now.
    ProceedAfter(std::time::Duration),
}

impl SupervisorState {
    fn new() -> Self {
        Self {
            // Start with last_rebuild far enough in the past that the FIRST
            // rebuild attempt has zero wait.
            last_rebuild_at: std::time::Instant::now() - std::time::Duration::from_secs(3600),
            consecutive_failures: 0,
        }
    }

    fn consecutive_failures(&self) -> u32 {
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
    fn is_cooling_off(&self) -> bool {
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
    fn should_rebuild_now(&self, now: std::time::Instant) -> RebuildDecision {
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
    fn backoff_for_failure_count(&self) -> std::time::Duration {
        if self.consecutive_failures < Self::COOL_OFF_THRESHOLD {
            return std::time::Duration::ZERO;
        }
        Self::COOL_OFF_WINDOW
    }

    fn mark_rebuild_started(&mut self) {
        self.last_rebuild_at = std::time::Instant::now();
    }

    fn mark_rebuild_succeeded(&mut self) {
        self.consecutive_failures = 0;
    }

    fn mark_rebuild_failed(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }
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
                if let Some(mut dead) = active.remove(source_id) {
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

pub struct NdiManager {
    _sdk: Arc<NdiLib>,
    source_list: SourceList,
    _finder_shutdown: FinderShutdown,
    /// Map source_id (UUID string) → ActiveSource pipeline.
    active: Mutex<HashMap<String, ActiveSource>>,
}

impl NdiManager {
    pub fn try_new() -> Option<Self> {
        let sdk = Arc::new(NdiLib::load().ok()?);
        let (source_list, finder_shutdown) = discovery::spawn_persistent_finder(Arc::clone(&sdk));
        Some(Self {
            _sdk: sdk,
            source_list,
            _finder_shutdown: finder_shutdown,
            active: Mutex::new(HashMap::new()),
        })
    }

    pub fn is_available(&self) -> bool {
        true
    }

    pub fn discover_sources(&self, _timeout_ms: u32) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Start a pipeline for the given source.
    ///
    /// `source_id` = UUID from the `video_sources` DB row (used as the WHEP URL key).
    /// `ndi_name` = NDI broadcaster name (e.g. "STREAM-SNV (stream)").
    ///
    /// Returns only AFTER the pipeline has transitioned to `Streaming` — i.e.
    /// the ndisrc has connected to the broadcaster, ndisrcdemux has negotiated
    /// video/audio caps, and webrtcsink's input pads are ready. Without this
    /// wait, an early WHEP POST hits webrtcsink before input caps are set
    /// and panics on `in_caps.unwrap()` (gst-plugin-webrtc imp.rs:3548).
    ///
    /// A 7-second timeout caps the wait — long enough for ndisrc to find the
    /// source on a healthy LAN, short enough that a missing/dead broadcaster
    /// reports back quickly to the operator.
    pub async fn start_pipeline(
        self: &std::sync::Arc<Self>,
        source_id: &str,
        ndi_name: &str,
    ) -> Result<()> {
        let mut active = self.active.lock().await;
        if let StateCheckOutcome::Idempotent = check_active_entry(&mut active, source_id).await {
            return Ok(());
        }

        let whep_url = format!("/ndi/whep/{}", source_id);
        let mut pipeline = NdiPipeline::build(ndi_name, whep_url)?;
        pipeline.start().await?;

        // Wait for webrtcsink's video sink-pad to have negotiated caps. Two
        // states aren't enough on their own:
        //  - `pipeline.state == Playing` fires almost immediately on a live
        //    source (NoPreroll) — before ndisrc has actually sent any frame.
        //  - The `streams` HashMap inside webrtcsink only gets `in_caps` set
        //    when the input pad receives its first CAPS event, which is when
        //    ndisrcdemux has identified video/audio formats from real NDI data.
        //
        // Polling `sink_element.sink_pad("video_0").current_caps()` is the
        // most reliable signal that caps are set. Without this, an early WHEP
        // POST hits webrtcsink while `in_caps == None` and panics at
        // gst-plugin-webrtc imp.rs:3548 (`in_caps.unwrap()`).
        //
        // 8-second budget: ndisrc takes ~2-5s on a healthy LAN to find a
        // broadcast + receive first frame. Beyond 8s the source likely doesn't
        // exist and we'd rather fail fast than hang the operator UI.
        let sink = pipeline
            .sink_element()
            .ok_or_else(|| anyhow!("pipeline has no sink element"))?;
        let video_pad = sink
            .static_pad("video_0")
            .ok_or_else(|| anyhow!("whepserversink has no video_0 sink pad"))?;
        let mut watcher = pipeline.state_watcher();
        let caps_ready = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                // Bail out if the pipeline errored (e.g. NDI source not found
                // and ndisrc emitted ERROR on the bus).
                if let crate::pipeline::PipelineState::Errored(ref e) = *watcher.borrow_and_update()
                {
                    return Err(anyhow!("pipeline errored: {e}"));
                }
                if video_pad.current_caps().is_some() {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        match caps_ready {
            Ok(Ok(())) => {
                // pipeline.state_watcher() and self.spawn_supervisor must
                // run before pipeline is moved into ActiveSource on the
                // active.insert line below.
                let watcher = pipeline.state_watcher();
                let supervisor =
                    self.spawn_supervisor(source_id.to_string(), ndi_name.to_string(), watcher);
                active.insert(
                    source_id.to_string(),
                    ActiveSource {
                        pipeline,
                        supervisor: Some(supervisor),
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
                    "NDI source '{ndi_name}' did not deliver any frame within 8s; \
                     ndisrc could not connect or the broadcaster is silent"
                ))
            }
        }
    }

    /// Spawn the supervisor task for one source. Returns the JoinHandle so
    /// callers can store it in `ActiveSource.supervisor` and abort on stop.
    ///
    /// The supervisor:
    /// - Subscribes to the pipeline's state watcher
    /// - On Errored/Stopped, requests a rebuild via `rebuild_pipeline`
    /// - Rate-limits to 1 rebuild / 2s; exponentially backs off after 5
    ///   consecutive failures
    /// - Re-subscribes to the FRESH state watcher after each successful
    ///   rebuild (the old watcher's pipeline was dropped)
    /// - Exits when the watcher closes (pipeline dropped) OR the task is
    ///   externally aborted via `stop_pipeline`
    fn spawn_supervisor(
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
                                state.mark_rebuild_succeeded();
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
    async fn state_watcher_for(
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
    async fn rebuild_pipeline(&self, source_id: &str, ndi_name: &str) -> Result<()> {
        let mut active = self.active.lock().await;
        // Force-remove the dead entry. If somehow it has become healthy in
        // the meantime, leave it alone (idempotent).
        if let StateCheckOutcome::Idempotent = check_active_entry(&mut active, source_id).await {
            return Ok(());
        }

        let whep_url = format!("/ndi/whep/{}", source_id);
        let mut pipeline = NdiPipeline::build(ndi_name, whep_url)?;
        pipeline.start().await?;

        let sink = pipeline
            .sink_element()
            .ok_or_else(|| anyhow!("pipeline has no sink element"))?;
        let video_pad = sink
            .static_pad("video_0")
            .ok_or_else(|| anyhow!("whepserversink has no video_0 sink pad"))?;
        let mut watcher = pipeline.state_watcher();
        let caps_ready = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                if let crate::pipeline::PipelineState::Errored(ref e) = *watcher.borrow_and_update()
                {
                    return Err(anyhow!("pipeline errored: {e}"));
                }
                if video_pad.current_caps().is_some() {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        match caps_ready {
            Ok(Ok(())) => {
                active.insert(
                    source_id.to_string(),
                    ActiveSource {
                        pipeline,
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
                    "NDI source '{ndi_name}' did not deliver any frame within 8s on rebuild"
                ))
            }
        }
    }

    /// Stop one pipeline.
    pub async fn stop_pipeline(&self, source_id: &str) {
        let mut active = self.active.lock().await;
        if let Some(mut src) = active.remove(source_id) {
            if let Some(handle) = src.supervisor.take() {
                handle.abort();
            }
            src.pipeline.stop().await;
        }
    }

    /// Stop ALL pipelines.
    pub async fn stop_all(&self) {
        let mut active = self.active.lock().await;
        for (_, mut src) in active.drain() {
            if let Some(handle) = src.supervisor.take() {
                handle.abort();
            }
            src.pipeline.stop().await;
        }
    }

    /// Is the given source's pipeline currently active?
    pub async fn is_active(&self, source_id: &str) -> bool {
        self.active.lock().await.contains_key(source_id)
    }

    /// Snapshot of every active pipeline's current state.
    ///
    /// Returns one entry per source currently in the active map, as
    /// `(source_id, PipelineState)`. Used by `/healthz` (#333 item 7) so
    /// dashboards can detect activation failures within seconds instead of
    /// inferring from operator-reported 'red error' status.
    ///
    /// Bounded by a 200 ms lock-acquisition timeout (deep-review 🟡 #1):
    /// `start_pipeline` and `rebuild_pipeline` hold the same `active` mutex
    /// for up to 8 s during the caps-wait. Without the timeout, a `/healthz`
    /// request that races a pipeline start would block long enough to
    /// trip a 5 s LB health-check timeout — exactly the failure mode
    /// item 7 was supposed to expose. On timeout we return an empty vec
    /// and log a warning; the caller (LB / dashboard) sees "no pipelines"
    /// for one poll cycle, which is preferable to a hung probe.
    pub async fn pipeline_snapshots(&self) -> Vec<(String, PipelineState)> {
        match tokio::time::timeout(std::time::Duration::from_millis(200), self.active.lock()).await
        {
            Ok(guard) => guard
                .iter()
                .map(|(id, src)| (id.clone(), src.pipeline.state()))
                .collect(),
            Err(_) => {
                tracing::warn!(
                    "pipeline_snapshots lock acquisition timed out after 200 ms — \
                     likely contended with a long-running pipeline start/rebuild; \
                     returning empty snapshot so /healthz does not stall (#333 item 7)"
                );
                Vec::new()
            }
        }
    }

    /// Test-only: trigger an Errored state on the source's pipeline so
    /// the PipelineSupervisor reacts as it would for a real ndisrc fault.
    /// Returns `true` if the source was active (state injection succeeded),
    /// `false` if not (caller should map to 404).
    #[cfg(feature = "test-helpers")]
    pub async fn simulate_pipeline_error(&self, source_id: &str, msg: &str) -> bool {
        let active = self.active.lock().await;
        match active.get(source_id) {
            Some(src) => {
                src.pipeline.simulate_error_for_test(msg);
                true
            }
            None => false,
        }
    }

    /// Forward a WHEP HTTP exchange into the source's `whepserversink`
    /// signaller via `emit_by_name`. The signaller's Promise resolves with
    /// `{status: u32, headers: gst::Structure, body: glib::Bytes}`.
    pub async fn whep_signaller_call(&self, source_id: &str, op: WhepOp) -> Result<WhepReply> {
        let sink = {
            let active = self.active.lock().await;
            let src = active
                .get(source_id)
                .ok_or_else(|| anyhow!(SOURCE_NOT_ACTIVE_ERR))?;
            match src.pipeline.state() {
                PipelineState::Streaming | PipelineState::Starting => {}
                PipelineState::Stopped => return Err(anyhow!("pipeline stopped")),
                PipelineState::Errored(e) => return Err(anyhow!("pipeline errored: {e}")),
            }
            src.pipeline
                .sink_element()
                .ok_or_else(|| anyhow!("pipeline has no sink element"))?
        };

        // Do all signaller work (non-Send glib::Object) in a synchronous block
        // before any .await points so the async fn stays Send.
        let fut = {
            let signaller = sink
                .dynamic_cast_ref::<gstreamer::ChildProxy>()
                .ok_or_else(|| anyhow!("sink is not a ChildProxy"))?
                .child_by_name("signaller")
                .ok_or_else(|| anyhow!("no signaller child on whepserversink"))?;

            let (promise, fut) = gstreamer::Promise::new_future();
            match op {
                WhepOp::Post { id, body } => {
                    let bytes = glib::Bytes::from_owned(body);
                    signaller.emit_by_name::<()>("post", &[&id, &bytes, &promise]);
                }
                WhepOp::Patch { id, body, headers } => {
                    let bytes = glib::Bytes::from_owned(body);
                    let mut sb = gstreamer::Structure::builder("whep-signaller/headers");
                    for (k, v) in &headers {
                        sb = sb.field(k.as_str(), v);
                    }
                    signaller.emit_by_name::<()>("patch", &[&id, &bytes, &sb.build(), &promise]);
                }
                WhepOp::Delete { id } => {
                    signaller.emit_by_name::<()>("delete", &[&id, &promise]);
                }
            }
            // `signaller` (non-Send glib::Object) is dropped here before `fut.await`
            fut
        };

        let reply = fut
            .await
            .map_err(|e| anyhow!("whep signaller promise error: {:?}", e))
            .context("whep signaller promise dropped")?
            .ok_or_else(|| anyhow!("whep signaller returned no payload"))?;
        let status = reply
            .get::<u32>("status")
            .map_err(|e| anyhow!("missing status field: {e}"))? as u16;
        let headers = match reply.get::<gstreamer::Structure>("headers") {
            Ok(s) => s
                .iter()
                .filter_map(|(name, value)| {
                    value.get::<String>().ok().map(|v| (name.to_string(), v))
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        let body = reply
            .get::<glib::Bytes>("body")
            .ok()
            .map(|b| b.as_ref().to_vec());
        Ok(WhepReply {
            status,
            headers,
            body,
        })
    }
}

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
                pipeline: dead,
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
                pipeline: p,
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
                pipeline: p,
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
