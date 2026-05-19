//! Per-source GStreamer pipeline owning ndisrc + vah264enc + whepserversink.
//!
//! Each `NdiPipeline` instance corresponds to ONE active NDI source. The
//! pipeline is built lazily on `start`, torn down on `stop`. Subscribers
//! (browser WHEP connections) reach `whepserversink` via the axum WHEP shim
//! which bridges HTTP signalling into the element's signaller via
//! `emit_by_name`.

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use tokio::sync::watch;

/// Pipeline lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineState {
    /// Built but not yet PLAYING (waiting for ASYNC_DONE).
    Starting,
    /// PLAYING — WHEP endpoint is live and accepting subscribers.
    Streaming,
    /// Tearing down or torn down.
    Stopped,
    /// Error state — pipeline failed and must be recreated.
    Errored(String),
}

/// Owns one GStreamer pipeline for one NDI source.
pub struct NdiPipeline {
    /// Underlying GStreamer pipeline.
    pipeline: gst::Pipeline,
    /// WHEP URL that subscribers (browsers) POST to.
    whep_url: String,
    /// State observer for the manager / WS event emitter.
    state_tx: watch::Sender<PipelineState>,
    state_rx: watch::Receiver<PipelineState>,
    /// Bus watch task handle so we can cancel on Drop.
    bus_watch: Option<tokio::task::JoinHandle<()>>,
}

impl NdiPipeline {
    /// Build but do not yet start the pipeline.
    ///
    /// `whep_url` is the axum route path (e.g. `/ndi/whep/<source_id>`) used
    /// as a logical key; the element does NOT bind its own HTTP port.
    pub fn build(ndi_name: &str, whep_url: String) -> Result<Self> {
        super::init().context("gstreamer init failed")?;
        if !super::vah264enc_available() {
            return Err(anyhow!(
                "vah264enc not available; refusing to build pipeline (would fall back to software H264 \
                 which melts the N100). Install with: \
                 sudo apt install gstreamer1.0-vaapi intel-media-va-driver-non-free"
            ));
        }

        let desc = format!(
            "ndisrc ndi-name=\"{ndi_name}\" ! \
             ndisrcdemux name=demux \
             demux.video ! videoconvert ! \
               vah264enc bitrate=2000 key-int-max=60 rate-control=cbr ! \
               video/x-h264,profile=baseline ! \
               sink.video_0 \
             demux.audio ! audioconvert ! audioresample ! \
               opusenc bitrate=64000 ! \
               sink.audio_0 \
             whepserversink name=sink"
        );

        let pipeline = gst::parse::launch(&desc)
            .with_context(|| format!("failed to build pipeline for '{ndi_name}'"))?;
        let pipeline = pipeline
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("parse::launch returned non-Pipeline element"))?;

        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);

        Ok(Self {
            pipeline,
            whep_url,
            state_tx,
            state_rx,
            bus_watch: None,
        })
    }

    /// Transition the pipeline to PLAYING. Returns immediately; the state
    /// watcher will move to `Streaming` once ASYNC_DONE is received.
    pub async fn start(&mut self) -> Result<()> {
        self.state_tx.send_replace(PipelineState::Starting);
        let pipeline = self.pipeline.clone();
        let state_tx = self.state_tx.clone();

        // Bus watch: drives the state transitions Starting → Streaming → Errored/Stopped.
        let bus = pipeline.bus().ok_or_else(|| anyhow!("pipeline has no bus"))?;
        self.bus_watch = Some(tokio::spawn(async move {
            let mut stream = bus.stream();
            use futures_util::StreamExt;
            while let Some(msg) = stream.next().await {
                match msg.view() {
                    gst::MessageView::AsyncDone(_) => {
                        let _ = state_tx.send(PipelineState::Streaming);
                    }
                    gst::MessageView::Error(err) => {
                        let detail = format!("{}: {}", err.error(), err.debug().unwrap_or_default().as_str());
                        tracing::error!(error = %detail, "pipeline error");
                        let _ = state_tx.send(PipelineState::Errored(detail));
                    }
                    gst::MessageView::Eos(_) => {
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
    pub async fn stop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(h) = self.bus_watch.take() {
            h.abort();
        }
        let _ = self.state_tx.send(PipelineState::Stopped);
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

    /// Returns a clone of the `whepserversink` element so the WHEP HTTP shim
    /// can reach its `signaller` child and emit_by_name SDP exchanges.
    pub fn sink_element(&self) -> Option<gst::Element> {
        self.pipeline.by_name("sink")
    }
}

impl Drop for NdiPipeline {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(h) = self.bus_watch.take() {
            h.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_fails_when_vah264enc_missing() {
        // We can't actually un-install vah264enc, but we can assert the precondition logic:
        // build() returns Err if vah264enc_available() returns false.
        super::super::init().unwrap();
        // Skip the "would fail" case if vah264enc IS available on this host (which it should be).
        if !super::super::vah264enc_available() {
            let result = NdiPipeline::build("SOMENAME", "http://localhost/whep".into());
            assert!(result.is_err());
            assert!(result.err().unwrap().to_string().contains("vah264enc not available"));
        }
    }

    #[test]
    fn build_returns_ok_for_valid_pipeline_when_plugins_present() {
        super::super::init().unwrap();
        if !super::super::vah264enc_available() {
            // Skipped — Task 3 step 5 documents how to install VA-API.
            return;
        }
        // We can't actually start an NDI receive in a unit test (no live NDI source),
        // but parse::launch on the pipeline string should succeed when all elements are
        // registered.
        let result = NdiPipeline::build(
            "no-such-source",
            "http://127.0.0.1/whep".into(),
        );
        assert!(
            result.is_ok(),
            "pipeline build failed: {}",
            result.err().map(|e| e.to_string()).unwrap_or_default()
        );
        let p = result.unwrap();
        assert_eq!(p.state(), PipelineState::Stopped);
        assert_eq!(p.whep_url(), "http://127.0.0.1/whep");
    }

    #[test]
    fn state_transitions_start_at_stopped() {
        super::super::init().unwrap();
        if !super::super::vah264enc_available() {
            return;
        }
        let p = NdiPipeline::build(
            "no-such-source",
            "http://127.0.0.1/whep".into(),
        )
        .unwrap();
        assert_eq!(p.state(), PipelineState::Stopped);
    }
}
