//! Pipeline lifecycle: start (PLAYING transition + bus watch), stop/teardown,
//! and the small state-observation accessors.

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use tokio::sync::watch;

use super::{NdiPipeline, PipelineState};

impl NdiPipeline {
    /// Transition the pipeline to PLAYING. Returns immediately; the state
    /// watcher moves to `Streaming` once the PIPELINE element posts
    /// `StateChanged → Playing` on the bus.
    pub async fn start(&self) -> Result<()> {
        self.state_tx.send_replace(PipelineState::Starting);
        let pipeline = self.pipeline.clone();
        let state_tx = self.state_tx.clone();
        let pipeline_obj = pipeline.upcast_ref::<gst::Object>().clone();

        // Bus watch: drives the state transitions Starting → Streaming → Errored/Stopped.
        //
        // Live sources (ndisrc) skip `AsyncDone` — they go PAUSED → PLAYING
        // directly via `NoPreroll`. We watch `StateChanged` filtered to the
        // PIPELINE element itself and trip Streaming when it reaches PLAYING.
        // Element-level state changes are ignored — they fire earlier and would
        // race against encoder/tee setup.
        let bus = pipeline
            .bus()
            .ok_or_else(|| anyhow!("pipeline has no bus"))?;
        let pipeline_weak = pipeline.downgrade();
        *self.bus_watch.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(tokio::spawn(async move {
                let mut stream = bus.stream();
                use futures_util::StreamExt;
                while let Some(msg) = stream.next().await {
                    match msg.view() {
                        gst::MessageView::StateChanged(sc)
                            if sc.src() == Some(&pipeline_obj)
                                && sc.current() == gst::State::Playing =>
                        {
                            let _ = state_tx.send(PipelineState::Streaming);
                        }
                        gst::MessageView::Latency(_) => {
                            // GStreamer requires the APPLICATION to service
                            // Latency messages by redistributing latency
                            // (gst-launch and webrtcsink both do this).
                            if let Some(p) = pipeline_weak.upgrade() {
                                p.call_async(|p| {
                                    let _ = p.recalculate_latency();
                                });
                            }
                        }
                        gst::MessageView::AsyncDone(_) => {
                            // Harmless duplicate for live pipelines (the
                            // StateChanged branch above already fired); load-bearing
                            // for non-live test cases like videotestsrc.
                            let _ = state_tx.send(PipelineState::Streaming);
                        }
                        gst::MessageView::Error(err) => {
                            let detail = format!(
                                "{}: {}",
                                err.error(),
                                err.debug().unwrap_or_default().as_str()
                            );
                            tracing::error!(error = %detail, "pipeline error");
                            let _ = state_tx.send(PipelineState::Errored(detail));
                        }
                        gst::MessageView::Eos(_) => {
                            tracing::warn!("pipeline EOS received → state=Stopped");
                            let _ = state_tx.send(PipelineState::Stopped);
                        }
                        _ => {}
                    }
                }
            }));

        pipeline
            .set_state(gst::State::Playing)
            .context("failed to set pipeline PLAYING")?;
        Ok(())
    }

    /// Tear down the pipeline. Safe to call multiple times.
    pub async fn stop(&self) {
        self.teardown();
        let _ = self.state_tx.send(PipelineState::Stopped);
    }

    /// Synchronous teardown: release per-consumer state, set pipeline state to
    /// Null, and abort the bus-watch task.
    /// Shared between `stop()` and `Drop` so the invariant lives in one place.
    /// Idempotent — GStreamer ignores a duplicate Null transition.
    pub(super) fn teardown(&self) {
        // Tear down each consumer's OWN pipeline and unregister its appsrc from
        // the fanout so the encoder appsink callback stops pushing into it.
        // sessions is a tokio::sync::Mutex; try_lock in Drop avoids blocking.
        if let Ok(mut sessions) = self.sessions.try_lock() {
            // WhepSession::drop performs the full per-consumer teardown:
            // StreamProducer link disconnect, bus-task abort, consumer
            // pipeline → Null.
            sessions.clear();
        } else {
            // Lock contention during Drop is unusual. GStreamer will free the
            // elements when the bins drop anyway; leave a debug log rather than
            // spinning.
            tracing::debug!(
                "NdiPipeline teardown: sessions mutex contended; \
                 skipping explicit per-consumer cleanup (GStreamer will free on bin drop)"
            );
        }
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(h) = self
            .bus_watch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            h.abort();
        }
    }

    pub fn whep_url(&self) -> &str {
        &self.whep_url
    }

    pub fn state(&self) -> PipelineState {
        self.state_rx.borrow().clone()
    }

    pub fn state_watcher(&self) -> watch::Receiver<PipelineState> {
        self.state_rx.clone()
    }

    /// Test-only: force an `Errored` state transition without actually
    /// disturbing the underlying GStreamer pipeline. Used by the WHEP
    /// kill-pipeline test route to simulate an `ndisrc` "Internal data
    /// stream error" — the realistic failure mode that the production
    /// `PipelineSupervisor` is designed to recover from.
    ///
    /// The supervisor (still alive, still subscribed to this state
    /// channel) reacts to the Errored transition exactly as it would
    /// for a real ndisrc fault: rebuild the pipeline via
    /// `NdiManager::rebuild_pipeline`.
    #[cfg(feature = "test-helpers")]
    pub fn simulate_error_for_test(&self, msg: &str) {
        let _ = self.state_tx.send(PipelineState::Errored(msg.to_string()));
    }
}
