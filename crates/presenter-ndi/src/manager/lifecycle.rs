//! `NdiManager` lifecycle + control surface: construction, source discovery,
//! starting and stopping pipelines, and active-map membership queries. Split
//! out of the manager god-file (#357).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::sync::Mutex;

use crate::discovery;
use crate::ndi_sdk::NdiLib;
use crate::pipeline::NdiPipeline;

use super::activation::{finalize_reservation, wait_for_streaming, Finalize, WaitOutcome};
use super::{check_active_entry, ActiveSource, NdiManager, StateCheckOutcome};

/// Outcome classification for [`NdiManager::start_pipeline`] failures.
///
/// Lets the caller distinguish an EXPECTED "the source is configured but its
/// broadcaster is silent / not producing" condition from a GENUINE pipeline
/// failure, so the stage view can show a calm "waiting for source" placeholder
/// for the former and a red error overlay only for the latter (#448). Before
/// this, both collapsed into one `anyhow::Error` and a configured-but-OFF source
/// was painted as a red "NDI pipeline failed: … broadcaster is silent" error.
#[derive(Debug)]
pub enum PipelineStartError {
    /// The pipeline built and started, but the NDI source did not begin
    /// streaming within the budget — the broadcaster is silent / not producing
    /// (e.g. Resolume output is off). An expected, non-error state.
    SourceSilent { ndi_name: String },
    /// A genuine failure to build/start/run the pipeline (encoder build failure,
    /// GStreamer element error, etc.).
    Failed(anyhow::Error),
    /// The in-flight `Starting` reservation was SUPERSEDED mid-wait (#741): a
    /// concurrent `stop`/deactivate removed it, or an activate-switch replaced it
    /// with a different pipeline. Not a failure — the caller must return `Ok`
    /// WITHOUT publishing a stage status (the concurrent op owns the outcome).
    Superseded,
}

impl std::fmt::Display for PipelineStartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineStartError::SourceSilent { ndi_name } => write!(
                f,
                "NDI source '{ndi_name}' is not producing — broadcaster silent or off"
            ),
            PipelineStartError::Failed(e) => write!(f, "{e}"),
            PipelineStartError::Superseded => write!(
                f,
                "pipeline start superseded by a concurrent activation change"
            ),
        }
    }
}

impl std::error::Error for PipelineStartError {}

impl From<anyhow::Error> for PipelineStartError {
    fn from(e: anyhow::Error) -> Self {
        PipelineStartError::Failed(e)
    }
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
            snapshot_contention_streak: std::sync::atomic::AtomicU32::new(0),
        })
    }

    pub fn is_available(&self) -> bool {
        true
    }

    /// Best-effort list of NDI sources — empty both when nothing is broadcasting AND when
    /// the finder has not looked. `GET /ndi/sources` uses this.
    pub fn discover_sources(&self, _timeout_ms: u32) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// What the finder has actually SEEN — `None` when it has never completed a scan
    /// (it failed to start, or has not finished its first ~5 s pass since startup).
    ///
    /// Anything that shows the answer to a human must use this, not
    /// [`Self::discover_sources`]: an empty list from a finder that never looked is
    /// blindness, and reporting it as an empty network makes the server accuse every
    /// sending machine at the site of being switched off (#546).
    pub fn discovery_snapshot(&self) -> Option<Vec<discovery::NdiSourceInfo>> {
        self.source_list.snapshot()
    }

    /// Start a pipeline for the given source.
    ///
    /// `source_id` = UUID from the `video_sources` DB row (used as the WHEP URL key).
    /// `ndi_name` = NDI broadcaster name (e.g. "STREAM-SNV (stream)").
    ///
    /// Returns only AFTER the pipeline has transitioned to `Streaming` — i.e.
    /// the GStreamer bus has emitted `StateChanged → Playing` for the pipeline
    /// element. For the shared-encoder topology (#336), this means ndisrc is
    /// alive and ndisrcdemux has begun delivering frames; the encoder + tee
    /// will start producing H264 buffers shortly after. Downstream webrtcbin
    /// consumers attach lazily via `add_consumer`; they do not require encoder
    /// caps at attach time (SDP exchange happens independently).
    ///
    /// An 8-second timeout caps the wait — long enough for ndisrc to find the
    /// source on a healthy LAN, short enough that a missing/dead broadcaster
    /// reports back quickly to the operator.
    pub async fn start_pipeline(
        self: &std::sync::Arc<Self>,
        source_id: &str,
        ndi_name: &str,
    ) -> std::result::Result<(), PipelineStartError> {
        // Phase 1 — RESERVE under the lock (fast: check + build + start + insert a
        // `Starting` entry). The lock is RELEASED before the ~8 s streaming-ready
        // wait (#741), so status polls / stop / reap / WHEP no longer stall behind
        // an activation. An early `return` inside this block leaves the guard
        // scope, so the guard never spans the wait.
        let reserved: std::sync::Arc<NdiPipeline> = {
            let mut active = self.active.lock().await;

            // Operator-reactivation path: snapshot a dead entry's supervisor
            // BEFORE check_active_entry removes the entry, so we can abort it
            // below (else it double-watches the fresh pipeline — deep-review 🔵 #3,
            // PR #340). Safe to `.take()` under the lock.
            let prior_supervisor: Option<tokio::task::JoinHandle<()>> = active
                .get_mut(source_id)
                .filter(|entry| {
                    matches!(
                        entry.pipeline.state(),
                        crate::pipeline::PipelineState::Stopped
                            | crate::pipeline::PipelineState::Errored(_)
                    )
                })
                .and_then(|entry| entry.supervisor.take());

            if let StateCheckOutcome::Idempotent = check_active_entry(&mut active, source_id).await
            {
                debug_assert!(prior_supervisor.is_none());
                // Healthy entry. If it is still STARTING, it is an in-flight
                // reservation from a CONCURRENT start for this source (#741) —
                // observer-join it rather than build a second pipeline; otherwise
                // it is Streaming → a true idempotent no-op.
                if let Some(entry) = active.get(source_id) {
                    if matches!(
                        entry.pipeline.state(),
                        crate::pipeline::PipelineState::Starting
                    ) {
                        let in_flight = std::sync::Arc::clone(&entry.pipeline);
                        drop(active);
                        return Self::observe_in_flight(in_flight, ndi_name).await;
                    }
                }
                return Ok(());
            }
            // The entry was dead → check_active_entry removed it. Abort the prior
            // supervisor so it doesn't double-watch the pipeline we build now.
            if let Some(handle) = prior_supervisor {
                handle.abort();
            }

            let whep_url = format!("/ndi/whep/{}", source_id);
            let pipeline = NdiPipeline::build(ndi_name, whep_url)?;
            pipeline.start().await?;
            let arc = std::sync::Arc::new(pipeline);
            // Insert the reservation in `Starting`. It (a) blocks a concurrent
            // start(A) from double-building (that start observer-joins this Arc)
            // and (b) makes the status reader show `Starting` (→ Connecting,
            // #546-safe) during the unlocked wait below.
            active.insert(
                source_id.to_string(),
                ActiveSource {
                    pipeline: std::sync::Arc::clone(&arc),
                    supervisor: None,
                },
            );
            arc
            // `active` guard dropped here — the wait below runs UNLOCKED.
        };

        // Phases 2+3 run in a DETACHED task (#741 review 🟡): a cancelled caller —
        // an axum activate-handler future dropped on client disconnect, a shutdown,
        // a `select!` — must NOT be able to orphan the `Starting` reservation. The
        // spawned task finalizes it regardless; awaiting the handle preserves the
        // "returns only after Streaming" contract, and if the caller IS dropped the
        // task keeps running and cleans up on its own.
        let manager = std::sync::Arc::clone(self);
        let sid = source_id.to_string();
        let name = ndi_name.to_string();
        match tokio::spawn(async move { manager.finalize_start(sid, name, reserved).await }).await {
            Ok(result) => result,
            Err(join_err) => Err(PipelineStartError::Failed(anyhow!(
                "start_pipeline finalize task panicked: {join_err}"
            ))),
        }
    }

    /// Phases 2+3 of `start_pipeline` (#741), run in a DETACHED task for
    /// cancellation-safety: wait (unlocked) for Streaming, then finalize under the
    /// lock. On `Promote` the supervisor is spawned AND attached inside the SAME
    /// critical section as the `Arc::ptr_eq` ownership re-check (#741 review 🟡) — so
    /// a `stop`/switch that lands in the gap cannot leave an unsupervised pipeline:
    /// the supervisor is attached atomically with the confirmation the slot is still
    /// ours (`spawn_supervisor` is a non-blocking `tokio::spawn`, so holding the lock
    /// across it costs nothing).
    async fn finalize_start(
        self: std::sync::Arc<Self>,
        source_id: String,
        ndi_name: String,
        reserved: std::sync::Arc<NdiPipeline>,
    ) -> std::result::Result<(), PipelineStartError> {
        let outcome =
            wait_for_streaming(reserved.state_watcher(), std::time::Duration::from_secs(8)).await;
        match finalize_reservation(&self.active, &source_id, &reserved, outcome).await {
            Finalize::Promote => {
                let mut active = self.active.lock().await;
                match active.get_mut(&source_id) {
                    Some(slot) if std::sync::Arc::ptr_eq(&slot.pipeline, &reserved) => {
                        let supervisor = self.spawn_supervisor(
                            source_id.clone(),
                            ndi_name.clone(),
                            reserved.state_watcher(),
                        );
                        slot.supervisor = Some(supervisor);
                        Ok(())
                    }
                    _ => {
                        // Slot vanished/replaced (a concurrent stop/switch) between
                        // finalize and here — stop our now-orphaned pipeline; publish
                        // nothing.
                        drop(active);
                        reserved.stop().await;
                        Err(PipelineStartError::Superseded)
                    }
                }
            }
            Finalize::Removed(WaitOutcome::Errored(e)) => {
                Err(PipelineStartError::Failed(anyhow!("pipeline errored: {e}")))
            }
            // Stopped/TimedOut → broadcaster silent / not producing (#448): a
            // neutral "waiting for source" placeholder, not a red error.
            Finalize::Removed(_) => Err(PipelineStartError::SourceSilent { ndi_name }),
            Finalize::Superseded => Err(PipelineStartError::Superseded),
        }
    }

    /// Observer-join an in-flight `Starting` reservation created by a CONCURRENT
    /// start for the same source (#741). We did NOT reserve it, so we never touch
    /// the active map — we only wait for it and report its outcome, preserving the
    /// "start_pipeline returns only after Streaming" contract for this caller too.
    ///
    /// Caveat (#741 review 🔵): when the observed reservation is a supervisor REBUILD
    /// that then dies/times out, this returns `Superseded`/`SourceSilent` and the
    /// caller (`activate_video_source`) publishes nothing new, so a stale stage status
    /// can persist until the 30 s auto-reconnect ticker re-drives activation. Accepted
    /// — the owner (the rebuild's supervisor) owns that source's real recovery.
    async fn observe_in_flight(
        in_flight: std::sync::Arc<NdiPipeline>,
        ndi_name: &str,
    ) -> std::result::Result<(), PipelineStartError> {
        match wait_for_streaming(in_flight.state_watcher(), std::time::Duration::from_secs(8)).await
        {
            WaitOutcome::Streaming => Ok(()),
            WaitOutcome::Errored(e) => {
                Err(PipelineStartError::Failed(anyhow!("pipeline errored: {e}")))
            }
            // The owner tore the reservation down (deactivate/reset) — publish nothing.
            WaitOutcome::Stopped => Err(PipelineStartError::Superseded),
            WaitOutcome::TimedOut => Err(PipelineStartError::SourceSilent {
                ndi_name: ndi_name.to_string(),
            }),
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

    /// Stop every active pipeline EXCEPT the one for `keep_id`.
    ///
    /// #370: called from the activate-switch path. Switching the active video
    /// source (deactivate A → activate B) used to start B's pipeline while
    /// leaving A's pipeline + its `nvh264enc` encoder streaming forever — the
    /// DB flipped A's row to `is_active=false` but the manager was never told.
    /// Two source pipelines (= two NVENC encoders) then accumulated after every
    /// switch. Reaping the orphaned siblings here keeps exactly ONE source
    /// pipeline running per the single-active-source invariant.
    pub async fn stop_other_pipelines(&self, keep_id: &str) {
        let mut active = self.active.lock().await;
        super::retain_only_active(&mut active, keep_id).await;
    }

    /// Stop ALL pipelines.
    pub async fn stop_all(&self) {
        let mut active = self.active.lock().await;
        for (_, src) in active.drain() {
            if let Some(handle) = src.supervisor {
                handle.abort();
            }
            src.pipeline.stop().await;
        }
    }

    /// Is the given source's pipeline currently active?
    pub async fn is_active(&self, source_id: &str) -> bool {
        self.active.lock().await.contains_key(source_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Kills the surviving Display mutant (lifecycle.rs:38 — `fmt` replaced with
    // `Ok(Default::default())`, i.e. empty output). Both arms of
    // `PipelineStartError`'s Display must produce the documented, non-empty text:
    // the SourceSilent arm carries the human-facing "not producing / broadcaster"
    // wording plus the ndi_name; the Failed arm transparently forwards the inner
    // error's Display. A pure constructor + Display assertion — no NDI SDK needed.
    #[test]
    fn source_silent_display_names_source_and_explains_not_producing() {
        let ndi_name = "RESOLUME-SNV (cg-obs)";
        let msg = PipelineStartError::SourceSilent {
            ndi_name: ndi_name.into(),
        }
        .to_string();
        assert!(
            msg.contains("not producing"),
            "SourceSilent Display must explain the source is not producing; got {msg:?}",
        );
        assert!(
            msg.contains("broadcaster"),
            "SourceSilent Display must mention the broadcaster; got {msg:?}",
        );
        assert!(
            msg.contains(ndi_name),
            "SourceSilent Display must name the NDI source; got {msg:?}",
        );
    }

    #[test]
    fn failed_display_forwards_inner_error_text() {
        let msg = PipelineStartError::Failed(anyhow!("boom")).to_string();
        assert_eq!(
            msg, "boom",
            "Failed Display must forward the inner error's text verbatim",
        );
    }

    // #741: pin the Superseded Display so the `fmt → Ok(Default::default())`
    // (empty-output) mutant is killed on this arm too.
    #[test]
    fn superseded_display_is_non_empty_and_explains_the_race() {
        let msg = PipelineStartError::Superseded.to_string();
        assert!(
            msg.contains("superseded"),
            "Superseded Display must name the superseded condition; got {msg:?}",
        );
        assert!(!msg.is_empty(), "Superseded Display must not be empty",);
    }
}
