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
        // We still probe for an HW encoder up-front so we can refuse-loudly
        // when neither vah264enc nor nvh264enc is registered. `whepserversink`
        // instantiates its OWN encoder internally based on GStreamer element
        // rank, so we don't pre-encode here — but if no HW encoder is
        // available it would fall back to software (x264enc), and software
        // 720p30 H264 melts the N100. The check below catches that case.
        let _ = super::hw_h264_encoder().ok_or_else(|| {
            anyhow!(
                "no hardware H264 encoder registered; refusing to build pipeline \
                 (software H264 at 720p30 would melt the N100). \
                 Install Intel VA-API: sudo apt install gstreamer1.0-vaapi intel-media-va-driver-non-free \
                 OR NVIDIA NVENC: sudo apt install gstreamer1.0-plugins-bad with nvcodec support"
            )
        })?;

        // CRITICAL: `whepserversink` (which is `webrtcsink` with the WHEP
        // signaller) takes RAW input. It picks an encoder internally and
        // manages bitrate, GOP, and codec negotiation with the browser. Do
        // NOT insert nvh264enc / vah264enc / h264parse upstream of it — the
        // pre-encoded H264 was being rejected by webrtcsink's internal
        // h264parse with "broken/invalid nal Type: 1 Slice, Size: 8". See
        // the official gst-plugin-webrtc-0.15.2/examples/whepserver.rs which
        // links videotestsrc → whepserversink directly.
        //
        // `video-caps=video/x-h264` constrains the codec offer to H264 (over
        // VP8/VP9) so encoder rank selection lands on nvh264enc/vah264enc
        // automatically. Browser-side WebRTC clients all support H264.
        //
        // Audio handling: ndisrcdemux ALWAYS exposes both a `video` and an
        // `audio` source pad. If the NDI broadcaster sends no audio (e.g.
        // Resolume, video-only OBS scenes), the audio pad never delivers a
        // CAPS event and webrtcsink panics on `in_caps.unwrap()` while
        // iterating registered streams. The audio branch terminates in a
        // `fakesink` with `async=false sync=false` so an absent audio stream
        // doesn't block preroll, and the audio pad is NOT linked into
        // whepserversink — so the WHEP offer is video-only, which is what
        // most live cameras need anyway. Re-enable audio per-source via a
        // settings flag once we have an NDI source that actually carries it.
        // `host-addr` overrides whepserversink's internal warp server bind.
        // The default `http://127.0.0.1:9090` collides with prior restart
        // processes AND with other dev2 services that already use 9090.
        // We pick a per-pipeline port in the 19090-19999 range derived from
        // a hash of the WHEP URL — stable across pipeline restarts for the
        // same source (so a crash-loop doesn't ping-pong ports), unique
        // across concurrent sources (so multiple NDI pipelines coexist).
        //
        // The bound port is NEVER reached externally — presenter's axum WHEP
        // shim bridges into the signaller via `emit_by_name("post"|"patch"|
        // "delete", ...)` directly. The warp server runs because the
        // signaller's lifecycle requires it; this just gets it out of our way.
        let port: u16 = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            whep_url.hash(&mut h);
            19090 + (h.finish() % 900) as u16
        };
        // host-addr AND video-caps are both set programmatically below
        // because parse::launch syntax can't reliably tokenize URL values
        // (`://`) and Caps values; the parser silently keeps the defaults
        // (warp on 9090, all codecs offered).
        let desc = format!(
            "ndisrc ndi-name=\"{ndi_name}\" ! \
             ndisrcdemux name=demux \
             demux.video ! videoconvert ! sink.video_0 \
             demux.audio ! fakesink async=false sync=false \
             whepserversink name=sink"
        );

        let pipeline = gst::parse::launch(&desc)
            .with_context(|| format!("failed to build pipeline for '{ndi_name}'"))?;
        let pipeline = pipeline
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("parse::launch returned non-Pipeline element"))?;

        // Constrain the codec offer to H264 only.
        //
        // Without this, webrtcsink defaults to offering ALL codecs (VP8,
        // VP9, H264 — see imp.rs:540 `video_caps: Codecs::video_codecs()`).
        // webrtcbin's first-choice in the SDP answer is VP8; webrtcsink then
        // tries to instantiate `vp8enc` (or `nvvp8enc`), neither of which
        // is registered on the N100/dev2 stack, so the m=video line goes
        // `a=inactive` and the browser <video> sits at videoWidth=0 forever
        // (black screen). Forcing `video/x-h264` makes the SDP answer offer
        // only H264 → encoder selection lands on nvh264enc/vah264enc which
        // we already have.
        let h264_caps = gstreamer::Caps::builder("video/x-h264").build();
        let sink = pipeline
            .by_name("sink")
            .ok_or_else(|| anyhow!("pipeline has no sink element"))?;
        sink.set_property("video-caps", &h264_caps);

        // Override the default whepserversink warp HTTP bind port.
        // The `host-addr` property lives on the `signaller` CHILD of the
        // sink element. We use `ChildProxyExtManual::set_child_property`
        // which does the `child::property` lookup itself — `set_property`
        // directly on the sink/child-proxy doesn't recurse (it strips the
        // `signaller::` prefix and looks up `host-addr` on the sink, which
        // doesn't have it).
        use gstreamer::prelude::ChildProxyExtManual;
        let child_proxy = sink
            .dynamic_cast_ref::<gstreamer::ChildProxy>()
            .ok_or_else(|| anyhow!("sink is not a ChildProxy"))?;
        child_proxy.set_child_property("signaller::host-addr", format!("http://127.0.0.1:{port}"));
        tracing::info!(port, %ndi_name, "whepserversink video-caps=H264 + host-addr set");

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
    /// watcher moves to `Streaming` once the PIPELINE element posts
    /// `StateChanged → Playing` on the bus.
    pub async fn start(&mut self) -> Result<()> {
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
        // race against webrtcsink's codec discovery.
        let bus = pipeline
            .bus()
            .ok_or_else(|| anyhow!("pipeline has no bus"))?;
        self.bus_watch = Some(tokio::spawn(async move {
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
        self.teardown();
        let _ = self.state_tx.send(PipelineState::Stopped);
    }

    /// Synchronous teardown: set state to Null and abort the bus-watch task.
    /// Shared between `stop()` and `Drop` so the invariant lives in one place.
    /// Idempotent — GStreamer ignores a duplicate Null transition.
    fn teardown(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(h) = self.bus_watch.take() {
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

    /// Returns a clone of the `whepserversink` element so the WHEP HTTP shim
    /// can reach its `signaller` child and emit_by_name SDP exchanges.
    pub fn sink_element(&self) -> Option<gst::Element> {
        self.pipeline.by_name("sink")
    }
}

impl Drop for NdiPipeline {
    fn drop(&mut self) {
        self.teardown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_fails_when_no_hw_h264_encoder() {
        // We can't actually un-install the encoders, but we can assert the precondition logic:
        // build() returns Err if hw_h264_encoder() returns None.
        super::super::init().unwrap();
        if super::super::hw_h264_encoder().is_none() {
            let result = NdiPipeline::build("SOMENAME", "http://localhost/whep".into());
            assert!(result.is_err());
            assert!(result
                .err()
                .unwrap()
                .to_string()
                .contains("no hardware H264 encoder"));
        }
    }

    #[test]
    fn build_returns_ok_for_valid_pipeline_when_plugins_present() {
        super::super::init().unwrap();
        if super::super::hw_h264_encoder().is_none() {
            // Skipped — host has neither Intel VA-API nor NVIDIA NVENC.
            return;
        }
        // We can't actually start an NDI receive in a unit test (no live NDI source),
        // but parse::launch on the pipeline string should succeed when all elements are
        // registered.
        let result = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into());
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
        if super::super::hw_h264_encoder().is_none() {
            return;
        }
        let p = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();
        assert_eq!(p.state(), PipelineState::Stopped);
    }

    /// Regression test for the bug surfaced 2026-05-20: the WHEP SDP answer
    /// was offering `a=rtpmap:96 VP8/90000 ... a=inactive` because the
    /// `whepserversink` `video-caps` property wasn't being applied (passed
    /// as a quoted string via parse::launch syntax). webrtcsink defaulted
    /// to offering all codecs, browser picked VP8, and the encoder couldn't
    /// instantiate `vp8enc` (we only have HW H264 encoders installed) — so
    /// the m=video line went `a=inactive` and the browser <video> sat at
    /// `videoWidth=0 readyState=0` forever (black screen).
    ///
    /// Asserts the `video-caps` GObject property is the gst::Caps we set
    /// programmatically after parse::launch, not the unconstrained default.
    #[test]
    fn video_caps_property_restricts_offered_codec_to_h264() {
        super::super::init().unwrap();
        if super::super::hw_h264_encoder().is_none() {
            // Pipeline can't even build without an HW encoder; nothing to
            // assert here.
            return;
        }
        let pipeline =
            NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();
        let sink = pipeline.sink_element().expect("sink element present");
        let actual_caps: gst::Caps = sink.property("video-caps");
        // The default whepserversink video-caps is the union of all video
        // codecs — its caps string contains "video/x-vp8" (or similar).
        // After our explicit override it must be exactly H264.
        let actual_str = actual_caps.to_string();
        assert!(
            actual_str.contains("video/x-h264"),
            "video-caps should contain video/x-h264; got: {actual_str}"
        );
        assert!(
            !actual_str.contains("video/x-vp8") && !actual_str.contains("video/x-vp9"),
            "video-caps should NOT contain VP8/VP9 (HW VP encoders absent → \
             would force 'a=inactive' in the WHEP SDP answer); got: {actual_str}"
        );
    }
}
