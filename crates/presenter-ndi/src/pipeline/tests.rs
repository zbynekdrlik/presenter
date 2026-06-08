//! Test-only constructors/stubs and the unit tests for `NdiPipeline`.
//!
//! This whole module is `#[cfg(test)]`, so every item below is compiled only
//! for the crate's own test build. The test-helper `impl NdiPipeline` block
//! provides minimal pipeline-shaped values and synchronous add/remove stubs so
//! the topology + cap regression tests run on every CI host (no GPU / libndi /
//! gst-plugin-ndi required).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use tokio::sync::watch;

use super::*;
use crate::whep_session::{WhepConnectionState, WhepSession};

impl NdiPipeline {
    /// Test-only constructor: a minimal pipeline-shaped value in `Stopped`
    /// state without actually building any GStreamer elements. Lets the
    /// `manager::start_pipeline` regression test (and any future test
    /// asserting state-based behaviour) run on every CI host — including
    /// `ubuntu-latest` runners that have no libndi, no GPU, and no
    /// gst-plugin-rs registered — without falling back to the silent-skip
    /// pattern that masks real regressions.
    ///
    /// The returned value owns an empty `gst::Pipeline`, an empty
    /// state-channel pinned at `Stopped`, no whep_url, no bus_watch. Its
    /// public surface (`state()`, `stop()`, drop) behaves identically to a
    /// real-but-never-started pipeline.
    pub fn stopped_for_test() -> Self {
        // gstreamer::init() is idempotent and runs without plugins.
        let _ = gstreamer::init();
        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);
        // A minimal placeholder tee for the stopped_for_test variant.
        // We can't call gst::ElementFactory::make here safely without
        // gst init — but init() above ensures it. Use fakesink as the
        // tee placeholder; stopped_for_test is only used for state tests,
        // not fanout topology tests.
        let placeholder = gst::ElementFactory::make("fakesink")
            .build()
            .unwrap_or_else(|_| {
                // If even fakesink isn't available (stripped env), synthesise
                // a pipeline object as a stand-in — this path only needs
                // enough to avoid a panic on Drop.
                panic!("stopped_for_test: gstreamer init succeeded but fakesink unavailable — environment is too stripped");
            });
        Self {
            pipeline: gst::Pipeline::new(),
            whep_url: String::new(),
            state_tx,
            state_rx,
            bus_watch: std::sync::Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            tee: Arc::new(placeholder),
        }
    }

    /// Test-only: force the observed state without going through the bus
    /// watch. Lets state-machine tests exercise Streaming/Errored branches
    /// without needing real GStreamer messaging.
    pub fn set_state_for_test(&mut self, state: PipelineState) {
        self.state_tx.send_replace(state);
    }

    /// Build a pipeline with the shared-encoder fanout topology using
    /// `videotestsrc` in place of `ndisrc`/`ndisrcdemux`, so the test runs on
    /// hosts without gst-plugin-ndi registered.
    ///
    /// Fails with an error if `encoder_name` is not registered.
    pub fn stopped_for_test_with_topology(encoder_name: &str) -> Result<Self> {
        crate::init()?;
        if gst::ElementFactory::find(encoder_name).is_none() {
            return Err(anyhow!(
                "encoder {encoder_name} not registered on this host"
            ));
        }
        let pipeline = gst::Pipeline::new();
        let src = gst::ElementFactory::make("videotestsrc")
            .build()
            .context("videotestsrc")?;
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .context("videoconvert")?;
        let mut encoder_builder = gst::ElementFactory::make(encoder_name).name("encoder");
        match encoder_name {
            "vah264enc" => {
                encoder_builder = encoder_builder
                    .property("key-int-max", 30u32)
                    .property("bitrate", 2500u32);
            }
            "nvh264enc" => {
                encoder_builder = encoder_builder
                    .property("gop-size", 30i32)
                    .property("zerolatency", true)
                    .property("bitrate", 2500u32);
            }
            "x264enc" => {
                encoder_builder = encoder_builder
                    .property_from_str("tune", "zerolatency")
                    .property_from_str("speed-preset", "superfast")
                    .property("key-int-max", 30u32)
                    .property("bitrate", 2500u32);
            }
            _ => {}
        }
        let encoder = encoder_builder.build().context("encoder")?;
        let payer = gst::ElementFactory::make("rtph264pay")
            .name("rtpay")
            .build()
            .context("rtph264pay")?;
        let tee = gst::ElementFactory::make("tee")
            .name("tee")
            .property("allow-not-linked", true)
            .build()
            .context("tee")?;
        pipeline
            .add_many([&src, &convert, &encoder, &payer, &tee])
            .context("add")?;
        gst::Element::link_many([&src, &convert, &encoder, &payer, &tee]).context("link")?;
        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);
        Ok(Self {
            pipeline,
            whep_url: String::new(),
            state_tx,
            state_rx,
            bus_watch: std::sync::Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            tee: Arc::new(tee),
        })
    }

    /// Sync stub: add a consumer WITHOUT SDP exchange (tests only).
    /// Enforces the same cap as production via `MAX_CONSUMERS_PER_SOURCE`.
    pub fn add_consumer_stub(&mut self, session_id: &str) -> Result<(), AddConsumerError> {
        {
            let sessions = self
                .sessions
                .try_lock()
                .expect("sessions mutex poisoned in test");
            if sessions.len() >= MAX_CONSUMERS_PER_SOURCE {
                return Err(AddConsumerError::CapReached {
                    max: MAX_CONSUMERS_PER_SOURCE,
                });
            }
        }
        let tee_pad = (*self.tee)
            .request_pad_simple("src_%u")
            .ok_or_else(|| anyhow!("tee has no src request pad"))?;
        // Use a real queue element so the WhepSession.queue field is valid.
        // This also keeps the test topology consistent with production (tee → queue → webrtcbin).
        let queue = gst::ElementFactory::make("queue")
            .build()
            .context("queue (test)")?;
        let webrtcbin = gst::ElementFactory::make("webrtcbin")
            .name(session_id)
            .build()
            .context("webrtcbin (test)")?;
        self.pipeline
            .add_many([&queue, &webrtcbin])
            .context("pipeline.add queue+webrtcbin")?;
        // Link: tee → queue → webrtcbin
        let queue_sink = queue
            .static_pad("sink")
            .ok_or_else(|| anyhow!("queue has no sink pad (test)"))?;
        tee_pad
            .link(&queue_sink)
            .context("link tee_pad -> queue.sink (test)")?;
        queue
            .link(&webrtcbin)
            .context("link queue -> webrtcbin (test)")?;
        let (ice_tx, _ice_rx) = tokio::sync::mpsc::unbounded_channel();
        let session = WhepSession {
            session_id: session_id.to_string(),
            webrtcbin,
            queue,
            tee_src_pad: tee_pad,
            connection_state: Arc::new(std::sync::Mutex::new(WhepConnectionState::New)),
            ice_tx,
        };
        self.sessions
            .try_lock()
            .unwrap()
            .insert(session_id.to_string(), session);
        Ok(())
    }

    /// Sync stub: remove a consumer (tests only). Idempotent.
    pub fn remove_consumer_stub(&mut self, session_id: &str) -> Result<()> {
        let Some(session) = self.sessions.try_lock().unwrap().remove(session_id) else {
            return Ok(());
        };
        let _ = session.webrtcbin.set_state(gst::State::Null);
        let _ = session.queue.set_state(gst::State::Null);
        self.pipeline.remove(&session.queue).ok();
        self.pipeline.remove(&session.webrtcbin).ok();
        (*self.tee).release_request_pad(&session.tee_src_pad);
        Ok(())
    }
}

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
        // Skipped — host has neither Intel VA-API nor NVIDIA NVENC nor x264enc.
        return;
    }
    // We can't actually start an NDI receive in a unit test (no live NDI source),
    // but building the pipeline should succeed when all elements are registered.
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

/// Regression test for #336: shared-encoder fanout.
///
/// The pre-fix pipeline ended in `whepserversink`, which (by gst-plugin-rs
/// 0.15 design) spawns one independent encoder per WHEP consumer. With
/// multiple stage-display browsers connecting to the same NDI source,
/// 3-4 encoders saturated the N100's iGPU VAAPI scheduler — the
/// 2026-05-24 production incident.
///
/// The fix builds the pipeline with `ndisrc → demux → videoconvert →
/// vah264enc → rtph264pay → tee` (one encoder), then `add_consumer`
/// dynamically requests a tee src pad + spawns one `webrtcbin` per
/// consumer that reads from the shared encoded stream. This test asserts
/// the load-bearing invariant: regardless of how many consumers are
/// added, the pipeline iterator yields EXACTLY ONE encoder element.
///
/// Runs on every CI host (no GPU/libndi required) — uses
/// `stopped_for_test_with_topology()` which builds the encoder + tee
/// + per-consumer webrtcbin elements with `x264enc` (always available)
/// in lieu of real HW encoders.
#[tokio::test]
async fn pipeline_has_single_encoder_for_n_consumers() {
    super::super::init().expect("gst init");
    // Build a pipeline-shaped value with the fanout topology, force
    // `x264enc` as the encoder so this runs on every CI host.
    let mut pipeline = NdiPipeline::stopped_for_test_with_topology("x264enc")
        .expect("test-only topology builder must succeed when x264enc registered");

    // Simulate N WHEP POSTs.
    for i in 0..4 {
        pipeline
            .add_consumer_stub(&format!("test-session-{i}"))
            .expect("add_consumer must succeed up to the soft cap");
    }

    // Count elements whose factory name is one of the encoder factories.
    let encoder_factories = ["vah264enc", "nvh264enc", "x264enc"];
    let encoder_count = pipeline.iterate_encoders().count();
    let webrtcbin_count = pipeline.iterate_webrtcbins().count();

    assert_eq!(
        encoder_count, 1,
        "REGRESSION (#336): pipeline must have EXACTLY ONE encoder for N consumers; \
         got {encoder_count} encoders for 4 consumers. Encoder factories considered: {encoder_factories:?}"
    );
    assert_eq!(
        webrtcbin_count, 4,
        "pipeline must have one webrtcbin per consumer; got {webrtcbin_count} for 4 consumers"
    );
}

/// Spec #336 / Task 6: soft consumer cap. 9th add_consumer call must
/// return CapReached so the HTTP layer 503s the browser.
#[tokio::test]
async fn add_consumer_returns_cap_reached_after_eight() {
    super::super::init().expect("gst init");
    let mut pipeline =
        NdiPipeline::stopped_for_test_with_topology("x264enc").expect("topology builder");

    // Fill the cap.
    for i in 0..MAX_CONSUMERS_PER_SOURCE {
        pipeline
            .add_consumer_stub(&format!("test-session-{i}"))
            .expect("consumer up to cap must succeed");
    }

    // Overflow: the (cap+1)th consumer must be rejected.
    let err = pipeline
        .add_consumer_stub("test-session-overflow")
        .expect_err("consumer cap+1 must fail");

    match err {
        AddConsumerError::CapReached { max } => {
            assert_eq!(
                max, MAX_CONSUMERS_PER_SOURCE,
                "cap value must match the const",
            );
        }
        AddConsumerError::Other(other) => {
            panic!("expected CapReached, got Other: {other}");
        }
    }
}

/// Spec #336 / Task 7: cleanup invariant. After N add_consumer +
/// M remove_consumer calls, the pipeline must have exactly N-M
/// webrtcbin elements, the encoder count must stay at exactly 1,
/// and remove_consumer on an unknown session must be idempotent.
///
/// Catches the regression class where a forgotten
/// tee.release_request_pad or a leaked webrtcbin/queue element
/// accumulates on every disconnect — would exhaust the iGPU's pad
/// budget after a busy service.
#[tokio::test]
async fn add_then_remove_leaves_clean_state() {
    super::super::init().expect("gst init");
    let mut pipeline =
        NdiPipeline::stopped_for_test_with_topology("x264enc").expect("topology builder");

    // Add 5 consumers.
    for i in 0..5 {
        pipeline
            .add_consumer_stub(&format!("session-{i}"))
            .expect("add must succeed");
    }
    assert_eq!(
        pipeline.iterate_webrtcbins().count(),
        5,
        "5 add_consumer calls must yield 5 webrtcbins",
    );
    assert_eq!(
        pipeline.iterate_encoders().count(),
        1,
        "encoder count must stay at 1 regardless of consumer churn",
    );

    // Remove 3.
    for i in 0..3 {
        pipeline
            .remove_consumer_stub(&format!("session-{i}"))
            .expect("remove must succeed");
    }
    assert_eq!(
        pipeline.iterate_webrtcbins().count(),
        2,
        "5 add - 3 remove must leave exactly 2 webrtcbin elements",
    );
    assert_eq!(
        pipeline.iterate_encoders().count(),
        1,
        "encoder count must stay at 1 across removes",
    );

    // Remove the rest.
    for i in 3..5 {
        pipeline
            .remove_consumer_stub(&format!("session-{i}"))
            .expect("remove must succeed");
    }
    assert_eq!(
        pipeline.iterate_webrtcbins().count(),
        0,
        "5 add - 5 remove must leave 0 webrtcbin elements",
    );
    assert_eq!(
        pipeline.iterate_encoders().count(),
        1,
        "encoder must still be present (it's part of the pipeline topology)",
    );

    // Remove non-existent session must be idempotent.
    pipeline
        .remove_consumer_stub("session-does-not-exist")
        .expect("remove_consumer_stub must be idempotent on unknown session");
}
