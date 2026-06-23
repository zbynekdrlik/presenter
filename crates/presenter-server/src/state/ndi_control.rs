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

    /// Forward to [`NdiManager::discover_sources`].
    pub(crate) fn discover_sources(
        &self,
        timeout_ms: u32,
    ) -> anyhow::Result<Vec<presenter_ndi::discovery::NdiSourceInfo>> {
        match self {
            Self::Real(m) => m.discover_sources(timeout_ms),
            #[cfg(test)]
            Self::Fake(_) => unreachable!("FakeNdiControl::discover_sources is never exercised"),
        }
    }

    /// Forward to [`NdiManager::pipeline_snapshots`].
    pub(crate) async fn pipeline_snapshots(
        &self,
    ) -> Vec<(String, presenter_ndi::pipeline::PipelineState)> {
        match self {
            Self::Real(m) => m.pipeline_snapshots().await,
            #[cfg(test)]
            Self::Fake(_) => unreachable!("FakeNdiControl::pipeline_snapshots is never exercised"),
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
            Self::Fake(_) => unreachable!("FakeNdiControl::whep_signaller_call is never exercised"),
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

    /// The ordered sequence of calls recorded so far.
    pub(crate) fn calls(&self) -> Vec<NdiCall> {
        self.calls.lock().expect("calls lock").clone()
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
}
