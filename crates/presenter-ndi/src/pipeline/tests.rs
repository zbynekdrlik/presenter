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
use gstreamer_app as gst_app;
use gstreamer_utils::StreamProducer;
use tokio::sync::watch;

use super::build::consumer_h264_caps;
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
    /// state-channel pinned at `Stopped`, no whep_url, no bus_watch, and a
    /// StreamProducer over a detached appsink. Its public surface (`state()`,
    /// `stop()`, drop) behaves identically to a real-but-never-started
    /// pipeline.
    pub fn stopped_for_test() -> Self {
        // gstreamer::init() is idempotent and runs without plugins.
        let _ = gstreamer::init();
        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);
        let appsink = gst_app::AppSink::builder().build();
        Self {
            pipeline: gst::Pipeline::new(),
            whep_url: String::new(),
            state_tx,
            state_rx,
            bus_watch: std::sync::Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            producer: StreamProducer::from(&appsink),
        }
    }

    /// Test-only: force the observed state without going through the bus
    /// watch. Lets state-machine tests exercise Streaming/Errored branches
    /// without needing real GStreamer messaging.
    pub fn set_state_for_test(&mut self, state: PipelineState) {
        self.state_tx.send_replace(state);
    }

    /// Build a pipeline with the shared-encoder topology using `videotestsrc`
    /// in place of `ndisrc`/`ndisrcdemux`, so the test runs on hosts without
    /// gst-plugin-ndi registered. Mirrors production: the encoder pipeline
    /// ends in an appsink wrapped by a StreamProducer; consumers run in their
    /// OWN pipelines (see `add_consumer_stub`).
    ///
    /// Fails with an error if `encoder_name` is not registered.
    ///
    /// NOTE: this stub uses FIXED legacy tuning values (GOP 30, backlog 30,
    /// sync=true) for STRUCTURAL tests only — it does not track production
    /// tuning. The real values are locked by
    /// `pipeline_tuning_properties_are_low_latency` against `NdiPipeline::build`.
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
        let h264parse = gst::ElementFactory::make("h264parse")
            .name("h264parse")
            .property("config-interval", -1i32)
            .build()
            .context("h264parse")?;
        let appsink = gst_app::AppSink::builder()
            .name("enc_appsink")
            .caps(&consumer_h264_caps())
            .max_buffers(30u32)
            .drop(true)
            .build();
        pipeline
            .add_many([
                &src,
                &convert,
                &encoder,
                &h264parse,
                appsink.upcast_ref::<gst::Element>(),
            ])
            .context("add")?;
        gst::Element::link_many([&src, &convert, &encoder, &h264parse]).context("link")?;
        h264parse
            .link(appsink.upcast_ref::<gst::Element>())
            .context("link h264parse -> appsink")?;
        let producer = StreamProducer::from(&appsink);
        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);
        Ok(Self {
            pipeline,
            whep_url: String::new(),
            state_tx,
            state_rx,
            bus_watch: std::sync::Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            producer,
        })
    }

    /// Sync stub: add a consumer WITHOUT SDP exchange (tests only). Builds the
    /// production consumer topology — a SEPARATE pipeline with
    /// `appsrc → rtph264pay → webrtcbin` — connects its appsrc to the
    /// StreamProducer, and stores the session. Enforces the same cap as
    /// production via `MAX_CONSUMERS_PER_SOURCE`.
    ///
    /// Must be called from within a tokio runtime (the per-pipeline bus-watch
    /// stand-in task is spawned on the current runtime).
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
        let appsrc = gst_app::AppSrc::builder()
            .name(format!("src_{session_id}"))
            .caps(&consumer_h264_caps())
            .build();
        let payloader = gst::ElementFactory::make("rtph264pay")
            .name(format!("pay_{session_id}"))
            .property("config-interval", -1i32)
            .build()
            .context("rtph264pay (test)")?;
        let webrtcbin = gst::ElementFactory::make("webrtcbin")
            .name(session_id)
            .build()
            .context("webrtcbin (test)")?;
        let consumer_pipeline = gst::Pipeline::with_name(&format!("consumer_{session_id}"));
        consumer_pipeline
            .add_many([appsrc.upcast_ref::<gst::Element>(), &payloader, &webrtcbin])
            .context("consumer pipeline add (test)")?;
        appsrc
            .upcast_ref::<gst::Element>()
            .link(&payloader)
            .context("link appsrc -> rtph264pay (test)")?;
        payloader
            .link(&webrtcbin)
            .context("link rtph264pay -> webrtcbin (test)")?;
        let link = self
            .producer
            .add_consumer(&appsrc)
            .map_err(|e| anyhow!("StreamProducer::add_consumer (test): {e}"))?;
        let (ice_tx, _ice_rx) = tokio::sync::mpsc::unbounded_channel();
        let session = WhepSession {
            session_id: session_id.to_string(),
            consumer_pipeline,
            webrtcbin,
            link,
            // Stand-in for the production bus watch; aborted on Drop.
            bus_task: tokio::spawn(async {}),
            connection_state: Arc::new(std::sync::Mutex::new(WhepConnectionState::New)),
            ice_tx,
        };
        self.sessions
            .try_lock()
            .expect("sessions mutex poisoned in test")
            .insert(session_id.to_string(), session);
        Ok(())
    }

    /// Sync stub: remove a consumer (tests only). Idempotent. Dropping the
    /// session disconnects the producer link, aborts the bus task, and nulls
    /// the consumer pipeline — the same path production `remove_consumer`
    /// takes.
    pub fn remove_consumer_stub(&mut self, session_id: &str) -> Result<()> {
        let Some(session) = self
            .sessions
            .try_lock()
            .expect("sessions mutex poisoned in test")
            .remove(session_id)
        else {
            return Ok(());
        };
        drop(session);
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

/// Low-latency regression locks (2026-06-11 design): every knob below was a
/// measured latency/stability mechanism — see
/// docs/superpowers/specs/2026-06-11-ndi-low-latency-design.md.
#[test]
fn pipeline_tuning_properties_are_low_latency() {
    super::super::init().unwrap();
    if super::super::hw_h264_encoder().is_none() {
        return;
    }
    let p = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();

    // 1. PTS from server receive time — zero sender-clock coupling, no drift
    //    DISCONT jumps ("lag builds, then jumps").
    let ndisrc = p
        .pipeline
        .by_name("ndisrc")
        .expect("ndisrc element must be named 'ndisrc'");
    assert_eq!(
        ndisrc.property::<gstndi::TimestampMode>("timestamp-mode"),
        gstndi::TimestampMode::ReceiveTime,
        "ndisrc must use pure receive-time timestamps"
    );

    // 2. Relay forwards frames immediately (sync=false saves ~40ms measured);
    //    small bounded backlog (5 frames, drop=true).
    let appsink = p
        .pipeline
        .by_name("enc_appsink")
        .expect("appsink named enc_appsink")
        .downcast::<gst_app::AppSink>()
        .expect("enc_appsink is an AppSink");
    assert!(
        !appsink.property::<bool>("sync"),
        "producer appsink must be sync=false (StreamProducer::with ProducerSettings)"
    );
    assert_eq!(appsink.max_buffers(), 5, "appsink backlog must be 5 frames");

    // 3. GOP 240 (8s): no 1s IDR pulses; joins use force-keyunit instead.
    let encoder = p.pipeline.by_name("encoder").expect("encoder named");
    let factory = encoder.factory().expect("factory").name().to_string();
    let gop: i64 = match factory.as_str() {
        "nvh264enc" => encoder.property::<i32>("gop-size") as i64,
        "vah264enc" | "x264enc" => encoder.property::<u32>("key-int-max") as i64,
        other => panic!("unexpected encoder factory {other}"),
    };
    assert_eq!(gop, 240, "GOP must be 240 frames");
}

/// Regression test for #336: shared-encoder fanout.
///
/// The pre-fix pipeline ended in `whepserversink`, which (by gst-plugin-rs
/// 0.15 design) spawns one independent encoder per WHEP consumer. With
/// multiple stage-display browsers connecting to the same NDI source,
/// 3-4 encoders saturated the N100's iGPU VAAPI scheduler — the
/// 2026-05-24 production incident.
///
/// The fix keeps ONE shared encoder in the encoder pipeline; consumers run in
/// their OWN pipelines (appsrc → rtph264pay → webrtcbin) fed via
/// StreamProducer, which contain NO encoder. This test asserts the
/// load-bearing invariant: regardless of how many consumers are added, the
/// ENCODER pipeline yields EXACTLY ONE encoder element and the consumer
/// pipelines yield one webrtcbin each.
///
/// Runs on every CI host (no GPU/libndi required) — uses
/// `stopped_for_test_with_topology()` with `x264enc` (always available)
/// in lieu of real HW encoders.
#[tokio::test]
async fn pipeline_has_single_encoder_for_n_consumers() {
    super::super::init().expect("gst init");
    let mut pipeline = NdiPipeline::stopped_for_test_with_topology("x264enc")
        .expect("test-only topology builder must succeed when x264enc registered");

    // Simulate N WHEP POSTs.
    for i in 0..4 {
        pipeline
            .add_consumer_stub(&format!("test-session-{i}"))
            .expect("add_consumer must succeed up to the soft cap");
    }

    let encoder_factories = ["vah264enc", "nvh264enc", "x264enc"];
    let encoder_count = pipeline.iterate_encoders().count();
    let webrtcbin_count = pipeline.iterate_webrtcbins().len();

    assert_eq!(
        encoder_count, 1,
        "REGRESSION (#336): the ENCODER pipeline must have EXACTLY ONE encoder \
         for N consumers; got {encoder_count} encoders for 4 consumers. \
         Encoder factories considered: {encoder_factories:?}"
    );
    assert_eq!(
        webrtcbin_count, 4,
        "one webrtcbin per consumer pipeline; got {webrtcbin_count} for 4 consumers"
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
/// M remove_consumer calls, exactly N-M consumer pipelines (one webrtcbin
/// each) remain, the encoder count stays at exactly 1, and remove_consumer
/// on an unknown session is idempotent.
///
/// Catches the regression class where a leaked consumer pipeline or a
/// dangling StreamProducer link accumulates on every disconnect — would
/// exhaust GPU/socket budgets after a busy service.
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
        pipeline.iterate_webrtcbins().len(),
        5,
        "5 add_consumer calls must yield 5 consumer pipelines",
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
        pipeline.iterate_webrtcbins().len(),
        2,
        "5 add - 3 remove must leave exactly 2 consumer pipelines",
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
        pipeline.iterate_webrtcbins().len(),
        0,
        "5 add - 5 remove must leave 0 consumer pipelines",
    );
    assert_eq!(
        pipeline.iterate_encoders().count(),
        1,
        "encoder must still be present (it's part of the encoder pipeline topology)",
    );

    // Remove non-existent session must be idempotent.
    pipeline
        .remove_consumer_stub("session-does-not-exist")
        .expect("remove_consumer_stub must be idempotent on unknown session");
}
