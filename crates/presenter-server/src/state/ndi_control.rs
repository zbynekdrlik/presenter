//! Hardware-boundary seam over the NDI manager surface the server invokes.
//!
//! # Why this seam exists (a HARDWARE-dependency boundary, not internal mocking)
//!
//! The real [`presenter_ndi::NdiManager`] talks to libndi + a GStreamer/NVENC
//! pipeline — physical, host-specific resources. `NdiManager::try_new()` returns
//! `None` on any host without libndi (the GitHub-hosted `Rust Tests` and
//! mutation runners have no libndi), so the `if let Some(manager) = …` block in
//! [`crate::state::AppState::activate_video_source`] — and therefore the #370
//! source-switch reap wiring inside it — is **unreachable by any libndi-free
//! unit test**. Without a seam, a refactor could silently delete the
//! `stop_other_pipelines(...)` reap call and reintroduce the #370 two-encoder
//! NVENC leak with every existing test still green.
//!
//! [`NdiManagerHandle`] is the seam: in production it is always
//! [`NdiManagerHandle::Real`], a zero-cost forwarder to the real `NdiManager`
//! (production behaviour is byte-for-byte unchanged). In `cfg(test)` it can also
//! be [`NdiManagerHandle::Fake`], a recording stand-in that lets a libndi-free
//! test assert the activation WIRING (does `activate_video_source` actually call
//! the reap after a successful `start_pipeline`?).
//!
//! Per `test-strictness.md`, the fake is acceptable **only** because it stands in
//! for the libndi/GPU hardware boundary — it does NOT mock internal server logic.
//! It exists purely to make the hardware-gated branch reachable on CI hosts that
//! physically lack the NDI SDK and an NVENC-capable GPU.

use std::sync::Arc;

use presenter_ndi::{NdiManager, PipelineStartError};

#[cfg(test)]
use std::sync::Mutex;

/// A handle to the NDI manager surface used by the server.
///
/// `Real` wraps the production [`NdiManager`] (the only variant that exists at
/// runtime). `Fake` is a `cfg(test)`-only recording stand-in for the
/// libndi/GPU hardware boundary, used to guard the #370 reap wiring.
///
/// `Clone` is cheap — every variant holds an `Arc`, so cloning a handle (as
/// happens whenever [`crate::state::AppState`] is cloned) is a refcount bump.
#[derive(Clone)]
pub(crate) enum NdiManagerHandle {
    /// Production variant — forwards every call to the real `NdiManager`.
    Real(Arc<NdiManager>),
    /// Test-only recording stand-in for the libndi/GPU hardware boundary.
    #[cfg(test)]
    Fake(Arc<FakeNdiControl>),
}

impl NdiManagerHandle {
    /// Forward to [`NdiManager::start_pipeline`].
    pub(crate) async fn start_pipeline(
        &self,
        source_id: &str,
        ndi_name: &str,
    ) -> Result<(), PipelineStartError> {
        match self {
            Self::Real(m) => m.start_pipeline(source_id, ndi_name).await,
            #[cfg(test)]
            Self::Fake(f) => f.start_pipeline(source_id, ndi_name).await,
        }
    }

    /// Forward to [`NdiManager::stop_pipeline`].
    pub(crate) async fn stop_pipeline(&self, source_id: &str) {
        match self {
            Self::Real(m) => m.stop_pipeline(source_id).await,
            #[cfg(test)]
            Self::Fake(f) => f.stop_pipeline(source_id).await,
        }
    }

    /// Forward to [`NdiManager::stop_other_pipelines`] — the #370 reap.
    pub(crate) async fn stop_other_pipelines(&self, keep_id: &str) {
        match self {
            Self::Real(m) => m.stop_other_pipelines(keep_id).await,
            #[cfg(test)]
            Self::Fake(f) => f.stop_other_pipelines(keep_id).await,
        }
    }

    /// Forward to [`NdiManager::stop_all`].
    pub(crate) async fn stop_all(&self) {
        match self {
            Self::Real(m) => m.stop_all().await,
            #[cfg(test)]
            Self::Fake(_) => unreachable!("FakeNdiControl::stop_all is never exercised"),
        }
    }

    /// Forward to [`NdiManager::discover_sources`] — best-effort, empty when blind.
    pub(crate) fn discover_sources(
        &self,
        timeout_ms: u32,
    ) -> anyhow::Result<Vec<presenter_ndi::discovery::NdiSourceInfo>> {
        match self {
            Self::Real(m) => m.discover_sources(timeout_ms),
            #[cfg(test)]
            Self::Fake(f) => Ok(f.discovery_snapshot().unwrap_or_default()),
        }
    }

    /// Forward to [`NdiManager::discovery_snapshot`] — `None` when the finder has never
    /// completed a scan, which the #546 status join must NOT read as an empty network.
    pub(crate) fn discovery_snapshot(
        &self,
    ) -> Option<Vec<presenter_ndi::discovery::NdiSourceInfo>> {
        match self {
            Self::Real(m) => m.discovery_snapshot(),
            #[cfg(test)]
            Self::Fake(f) => f.discovery_snapshot(),
        }
    }

    /// Forward to [`NdiManager::pipeline_snapshots`].
    pub(crate) async fn pipeline_snapshots(
        &self,
    ) -> Vec<(String, presenter_ndi::pipeline::PipelineState)> {
        match self {
            Self::Real(m) => m.pipeline_snapshots().await,
            #[cfg(test)]
            Self::Fake(f) => f.pipeline_snapshots().unwrap_or_default(),
        }
    }

    /// Forward to [`NdiManager::pipeline_snapshots_checked`] — `None` when the
    /// manager's lock could not be taken (it is busy starting a pipeline), which the
    /// #546 status join must NOT read as "no pipelines".
    pub(crate) async fn pipeline_snapshots_checked(
        &self,
    ) -> Option<Vec<(String, presenter_ndi::pipeline::PipelineState)>> {
        match self {
            Self::Real(m) => m.pipeline_snapshots_checked().await,
            #[cfg(test)]
            Self::Fake(f) => f.pipeline_snapshots(),
        }
    }

    /// Forward to [`NdiManager::pipeline_snapshot`].
    pub(crate) async fn pipeline_snapshot(
        &self,
        source_id: &str,
    ) -> Option<presenter_ndi::PipelineSnapshot> {
        match self {
            Self::Real(m) => m.pipeline_snapshot(source_id).await,
            #[cfg(test)]
            Self::Fake(_) => unreachable!("FakeNdiControl::pipeline_snapshot is never exercised"),
        }
    }

    /// Forward to [`NdiManager::whep_signaller_call`].
    pub(crate) async fn whep_signaller_call(
        &self,
        source_id: &str,
        op: presenter_ndi::manager::WhepOp,
    ) -> anyhow::Result<presenter_ndi::manager::WhepReply> {
        match self {
            Self::Real(m) => m.whep_signaller_call(source_id, op).await,
            #[cfg(test)]
            Self::Fake(f) => f.whep_signaller_call(source_id, op).await,
        }
    }

    /// Forward to [`NdiManager::simulate_pipeline_error`] (test-helpers feature).
    #[cfg(feature = "test-helpers")]
    pub(crate) async fn simulate_pipeline_error(&self, source_id: &str, msg: &str) -> bool {
        match self {
            Self::Real(m) => m.simulate_pipeline_error(source_id, msg).await,
            #[cfg(test)]
            Self::Fake(_) => {
                unreachable!("FakeNdiControl::simulate_pipeline_error is never exercised")
            }
        }
    }
}

/// Recording stand-in for the libndi/GPU hardware boundary.
///
/// Records the ordered sequence of activation-path calls so a libndi-free test
/// can assert the #370 reap WIRING in
/// [`crate::state::AppState::activate_video_source`]: after `start_pipeline`
/// returns `Ok`, the activation MUST call `stop_other_pipelines(new_id)`; on
/// `start_pipeline` `Err` it must NOT. `start_outcome` lets a test choose what
/// `start_pipeline` returns (Ok / silent-source Ok / hard Err).
#[cfg(test)]
#[derive(Default)]
pub(crate) struct FakeNdiControl {
    calls: Mutex<Vec<NdiCall>>,
    start_outcome: Mutex<StartOutcome>,
    /// What the NDI network "contains" — drives the #546 status join without libndi.
    discovered: Mutex<Vec<String>>,
    /// What the manager's pipeline map "holds", keyed by source id.
    snapshots: Mutex<Vec<(String, presenter_ndi::pipeline::PipelineState)>>,
    /// `true` = the manager's lock is held (it is busy starting a pipeline), so the
    /// snapshot map cannot be read at all — the real 200 ms-timeout path.
    snapshots_unreadable: Mutex<bool>,
    /// `true` = the finder has never completed a scan (it failed to start, or the server
    /// has only just booted), so this server cannot see the network at all (#546).
    finder_never_scanned: Mutex<bool>,
    /// What `whep_signaller_call` should return (#630 call-site wiring tests).
    whep_outcome: Mutex<Option<WhepOutcome>>,
}

/// What [`FakeNdiControl::whep_signaller_call`] should return.
///
/// Holds real [`presenter_ndi::manager::NdiSessionError`] variants (via
/// [`Self::to_result`]) rather than a bare string, so `map_signaller_error`'s
/// `downcast_ref` in `router/integrations/ndi_whep.rs` genuinely matches —
/// the fake exercises the SAME typed-error path a live `NdiManager` failure
/// would take, not a stand-in string comparison (#630). Only the variants the
/// #630 wiring tests actually need are here — extend when a new call site
/// needs a different outcome, per MVP philosophy (no speculative variants).
#[cfg(test)]
#[derive(Clone)]
pub(crate) enum WhepOutcome {
    /// `NdiSessionError::SourceNotActive`.
    SourceNotActive,
    /// `NdiSessionError::ConsumerCapReached { max }`.
    ConsumerCapReached { max: usize },
}

#[cfg(test)]
impl WhepOutcome {
    fn to_result(&self) -> anyhow::Result<presenter_ndi::manager::WhepReply> {
        use presenter_ndi::manager::NdiSessionError;
        match self {
            Self::SourceNotActive => Err(NdiSessionError::SourceNotActive.into()),
            Self::ConsumerCapReached { max } => {
                Err(NdiSessionError::ConsumerCapReached { max: *max }.into())
            }
        }
    }
}

/// One recorded call against [`FakeNdiControl`].
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NdiCall {
    /// `start_pipeline(source_id, ndi_name)`.
    StartPipeline { source_id: String, ndi_name: String },
    /// `stop_other_pipelines(keep_id)` — the #370 reap.
    StopOtherPipelines { keep_id: String },
}

/// What [`FakeNdiControl::start_pipeline`] should return.
#[cfg(test)]
#[derive(Default, Clone, Copy)]
pub(crate) enum StartOutcome {
    /// Pipeline reached Streaming — the success path (default).
    #[default]
    Ok,
    /// Broadcaster silent / not producing — an Ok-returning activation (#448).
    SilentSource,
    /// A genuine hard failure — activation returns Err.
    HardError,
}

#[cfg(test)]
impl FakeNdiControl {
    /// A fake whose `start_pipeline` returns the chosen outcome.
    pub(crate) fn with_outcome(outcome: StartOutcome) -> Arc<Self> {
        let fake = Self::default();
        *fake.start_outcome.lock().expect("start_outcome lock") = outcome;
        Arc::new(fake)
    }

    /// A fake whose `whep_signaller_call` returns the chosen outcome (#630).
    pub(crate) fn with_whep_outcome(outcome: WhepOutcome) -> Arc<Self> {
        let fake = Self::default();
        *fake.whep_outcome.lock().expect("whep_outcome lock") = Some(outcome);
        Arc::new(fake)
    }

    /// The ordered sequence of calls recorded so far.
    pub(crate) fn calls(&self) -> Vec<NdiCall> {
        self.calls.lock().expect("calls lock").clone()
    }

    /// Put these NDI names "on the network" (#546).
    pub(crate) fn set_discovered(&self, names: &[&str]) {
        *self.discovered.lock().expect("discovered lock") =
            names.iter().map(|n| (*n).to_string()).collect();
    }

    /// Put this pipeline in the manager's map (#546).
    pub(crate) fn set_pipeline(
        &self,
        source_id: &str,
        state: presenter_ndi::pipeline::PipelineState,
    ) {
        self.snapshots
            .lock()
            .expect("snapshots lock")
            .push((source_id.to_string(), state));
    }

    /// The finder has never completed a scan — the server is BLIND, which is a different
    /// fact from "the network is empty" (#546). This is the state a server is in when
    /// `NDIlib_find_create_v2` returned null (forever) or has just booted (transiently).
    pub(crate) fn finder_never_scanned(&self) {
        *self
            .finder_never_scanned
            .lock()
            .expect("finder_never_scanned lock") = true;
    }

    fn discovery_snapshot(&self) -> Option<Vec<presenter_ndi::discovery::NdiSourceInfo>> {
        if *self
            .finder_never_scanned
            .lock()
            .expect("finder_never_scanned lock")
        {
            return None;
        }
        Some(
            self.discovered
                .lock()
                .expect("discovered lock")
                .iter()
                .map(|name| presenter_ndi::discovery::NdiSourceInfo { name: name.clone() })
                .collect(),
        )
    }

    /// Simulate the manager being BUSY (mid `start_pipeline`): its lock is held, so the
    /// snapshot map cannot be read within the 200 ms budget (#546).
    pub(crate) fn set_snapshots_unreadable(&self) {
        *self
            .snapshots_unreadable
            .lock()
            .expect("snapshots_unreadable lock") = true;
    }

    fn pipeline_snapshots(&self) -> Option<Vec<(String, presenter_ndi::pipeline::PipelineState)>> {
        if *self
            .snapshots_unreadable
            .lock()
            .expect("snapshots_unreadable lock")
        {
            return None;
        }
        Some(self.snapshots.lock().expect("snapshots lock").clone())
    }

    /// Whether `stop_other_pipelines(keep_id)` was recorded with this id.
    pub(crate) fn reaped(&self, keep_id: &str) -> bool {
        self.calls()
            .iter()
            .any(|c| matches!(c, NdiCall::StopOtherPipelines { keep_id: k } if k == keep_id))
    }

    fn record(&self, call: NdiCall) {
        self.calls.lock().expect("calls lock").push(call);
    }

    async fn start_pipeline(
        &self,
        source_id: &str,
        ndi_name: &str,
    ) -> Result<(), PipelineStartError> {
        self.record(NdiCall::StartPipeline {
            source_id: source_id.to_string(),
            ndi_name: ndi_name.to_string(),
        });
        match *self.start_outcome.lock().expect("start_outcome lock") {
            StartOutcome::Ok => Ok(()),
            StartOutcome::SilentSource => Err(PipelineStartError::SourceSilent {
                ndi_name: ndi_name.to_string(),
            }),
            StartOutcome::HardError => Err(PipelineStartError::Failed(anyhow::anyhow!(
                "simulated start failure"
            ))),
        }
    }

    async fn stop_pipeline(&self, _source_id: &str) {
        // Not exercised by the wiring test; recorded for completeness.
    }

    async fn stop_other_pipelines(&self, keep_id: &str) {
        self.record(NdiCall::StopOtherPipelines {
            keep_id: keep_id.to_string(),
        });
    }

    /// #630: return the configured [`WhepOutcome`] — real typed
    /// `NdiSessionError` variants (or a generic error), so a router-level
    /// test can exercise the `.map_err(map_signaller_error)?` call-site
    /// wiring in `post_whep_session`/`patch_whep_session` deterministically,
    /// without a live NdiManager.
    async fn whep_signaller_call(
        &self,
        _source_id: &str,
        _op: presenter_ndi::manager::WhepOp,
    ) -> anyhow::Result<presenter_ndi::manager::WhepReply> {
        match self
            .whep_outcome
            .lock()
            .expect("whep_outcome lock")
            .as_ref()
        {
            Some(outcome) => outcome.to_result(),
            None => Err(anyhow::anyhow!(
                "FakeNdiControl: no whep_outcome configured — use with_whep_outcome"
            )),
        }
    }
}
