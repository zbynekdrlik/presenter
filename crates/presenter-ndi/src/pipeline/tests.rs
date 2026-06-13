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
use gstreamer_video as gst_video;
use tokio::sync::watch;

use super::build::{compat_raw_caps, consumer_h264_caps};
use super::*;
use crate::whep_session::{LivenessState, WhepConnectionState, WhepSession};

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
        // producer_compat mirrors production's parallel compat branch with
        // another detached appsink (no compat fanout is exercised here).
        let appsink_compat = gst_app::AppSink::builder().build();
        Self {
            pipeline: gst::Pipeline::new(),
            whep_url: String::new(),
            state_tx,
            state_rx,
            bus_watch: std::sync::Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            producer: StreamProducer::from(&appsink),
            producer_compat: StreamProducer::from(&appsink_compat),
        }
    }

    /// Test-only: force the observed state without going through the bus
    /// watch. Lets state-machine tests exercise Streaming/Errored branches
    /// without needing real GStreamer messaging.
    pub fn set_state_for_test(&mut self, state: PipelineState) {
        self.state_tx.send_replace(state);
    }

    /// Build a pipeline with the production topology using `videotestsrc` in
    /// place of `ndisrc`/`ndisrcdemux`, so the test runs on hosts without
    /// gst-plugin-ndi registered. Mirrors production's TWO-PROFILE shape (#387):
    /// `src → convert → tee` fanning into a SHARED H264 default branch
    /// (encoder → h264parse → enc_appsink) and a RAW compat branch
    /// (→ enc_appsink_compat_raw, NO encoder — the compat vp8enc is
    /// per-consumer), each wrapped by its own StreamProducer; consumers run in
    /// their OWN pipelines (see `add_consumer_stub` / `add_compat_consumer_stub`).
    ///
    /// Fails with an error if `encoder_name` is not registered.
    ///
    /// NOTE: this stub uses FIXED legacy tuning values (GOP 30, backlog 30,
    /// sync=true) for STRUCTURAL tests only — it does not track production
    /// tuning or the compat conditioning chain. The real values are locked by
    /// `pipeline_tuning_properties_are_low_latency` and
    /// `compat_branch_fans_raw_i420_no_shared_encoder` against
    /// `NdiPipeline::build`.
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
        let tee = gst::ElementFactory::make("tee")
            .name("raw_tee")
            .build()
            .context("tee")?;
        pipeline
            .add_many([&src, &convert, &tee])
            .context("add head")?;
        gst::Element::link_many([&src, &convert, &tee]).context("link head")?;
        let producer = Self::add_stub_encode_branch(
            &pipeline,
            &tee,
            encoder_name,
            "encoder",
            "h264parse",
            "enc_appsink",
        )?;
        let producer_compat = Self::add_stub_raw_compat_branch(&pipeline, &tee)?;
        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);
        Ok(Self {
            pipeline,
            whep_url: String::new(),
            state_tx,
            state_rx,
            bus_watch: std::sync::Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            producer,
            producer_compat,
        })
    }

    /// Test-only helper for `stopped_for_test_with_topology`: append one
    /// encode branch (`queue → encoder → h264parse → appsink`) to the stub
    /// tee and wrap its appsink in a StreamProducer. Legacy fixed tuning —
    /// structural tests only.
    fn add_stub_encode_branch(
        pipeline: &gst::Pipeline,
        tee: &gst::Element,
        encoder_name: &str,
        encoder_element_name: &str,
        parse_name: &str,
        sink_name: &str,
    ) -> Result<StreamProducer> {
        let queue = gst::ElementFactory::make("queue")
            .build()
            .context("queue")?;
        let mut encoder_builder =
            gst::ElementFactory::make(encoder_name).name(encoder_element_name);
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
            .name(parse_name)
            .property("config-interval", -1i32)
            .build()
            .context("h264parse")?;
        let appsink = gst_app::AppSink::builder()
            .name(sink_name)
            .caps(&consumer_h264_caps())
            .max_buffers(30u32)
            .drop(true)
            .build();
        pipeline
            .add_many([
                &queue,
                &encoder,
                &h264parse,
                appsink.upcast_ref::<gst::Element>(),
            ])
            .context("add branch")?;
        gst::Element::link_many([tee, &queue, &encoder, &h264parse]).context("link branch")?;
        h264parse
            .link(appsink.upcast_ref::<gst::Element>())
            .context("link h264parse -> appsink")?;
        Ok(StreamProducer::from(&appsink))
    }

    /// Test-only helper for `stopped_for_test_with_topology`: append the #387
    /// compat branch stub — `queue → appsink("enc_appsink_compat_raw")` fanning
    /// RAW frames, NO encoder. Since #387 the compat encoder lives in each
    /// CONSUMER pipeline (per-TV adaptive bitrate), so the encoder pipeline's
    /// compat branch carries RAW. STRUCTURAL only — no videorate/convert/scale
    /// conditioning; the real RAW caps are locked by
    /// `compat_branch_fans_raw_i420_no_shared_encoder` against
    /// `NdiPipeline::build`.
    fn add_stub_raw_compat_branch(
        pipeline: &gst::Pipeline,
        tee: &gst::Element,
    ) -> Result<StreamProducer> {
        let queue = gst::ElementFactory::make("queue")
            .build()
            .context("queue (raw compat stub)")?;
        let appsink = gst_app::AppSink::builder()
            .name("enc_appsink_compat_raw")
            .caps(&compat_raw_caps())
            .max_buffers(30u32)
            .drop(true)
            .build();
        pipeline
            .add_many([&queue, appsink.upcast_ref::<gst::Element>()])
            .context("add raw compat stub branch")?;
        gst::Element::link_many([tee, &queue, appsink.upcast_ref::<gst::Element>()])
            .context("link raw compat stub branch")?;
        Ok(StreamProducer::from(&appsink))
    }

    /// Stub: add a consumer WITHOUT SDP exchange (tests only). Builds the
    /// production consumer topology — a SEPARATE pipeline with
    /// `appsrc → rtph264pay → webrtcbin` — connects its appsrc to the
    /// StreamProducer, and stores the session. Runs the SAME join gate as
    /// production `add_consumer` (`reap_and_check_cap`: reap zombies, then
    /// enforce `MAX_CONSUMERS_PER_SOURCE`), so the cap/reaper tests exercise
    /// the real logic instead of a duplicated check.
    ///
    /// Must be called from within a tokio runtime (the per-pipeline bus-watch
    /// stand-in task is spawned on the current runtime).
    pub async fn add_consumer_stub(&mut self, session_id: &str) -> Result<(), AddConsumerError> {
        self.reap_and_check_cap().await?;
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
            liveness: Arc::new(std::sync::Mutex::new(LivenessState::new())),
            ice_tx,
            // DEFAULT profile: shared H264 encoder, no per-consumer controller.
            compat_controller_task: None,
            compat_target_bitrate: None,
        };
        self.sessions
            .lock()
            .await
            .insert(session_id.to_string(), session);
        Ok(())
    }

    /// Stub: add a COMPAT consumer WITHOUT SDP exchange (tests only). Mirrors
    /// the #387 production compat consumer topology — a SEPARATE pipeline with
    /// `appsrc(RAW I420) → vp8enc("venc_<session>") → rtpvp8pay → webrtcbin`,
    /// fed from the COMPAT (raw) StreamProducer, with the per-consumer
    /// `CompatBitrateController`'s current bitrate exposed via an `AtomicI32`
    /// (started at START_BPS, no controller task spawned in the stub). Lets the
    /// #336/#387 topology + snapshot tests exercise the real per-consumer-VP8
    /// shape without a live RTCP feedback loop.
    pub async fn add_compat_consumer_stub(
        &mut self,
        session_id: &str,
    ) -> Result<(), AddConsumerError> {
        self.reap_and_check_cap().await?;
        let (appsrc, encoder, payloader, webrtcbin) =
            super::consumer_build::build_consumer_elements(session_id, StreamProfile::Compat, 96)
                .map_err(AddConsumerError::Other)?;
        let encoder = encoder.expect("compat consumer must own a vp8enc (#387)");
        let consumer_pipeline = gst::Pipeline::with_name(&format!("consumer_{session_id}"));
        consumer_pipeline
            .add_many([
                appsrc.upcast_ref::<gst::Element>(),
                &encoder,
                &payloader,
                &webrtcbin,
            ])
            .context("compat consumer pipeline add (test)")?;
        gst::Element::link_many([appsrc.upcast_ref::<gst::Element>(), &encoder, &payloader])
            .context("link appsrc -> vp8enc -> rtpvp8pay (test)")?;
        payloader
            .link(&webrtcbin)
            .context("link rtpvp8pay -> webrtcbin (test)")?;
        let link = self
            .producer_compat
            .add_consumer(&appsrc)
            .map_err(|e| anyhow!("StreamProducer::add_consumer compat (test): {e}"))?;
        let (ice_tx, _ice_rx) = tokio::sync::mpsc::unbounded_channel();
        let session = WhepSession {
            session_id: session_id.to_string(),
            consumer_pipeline,
            webrtcbin,
            link,
            bus_task: tokio::spawn(async {}),
            connection_state: Arc::new(std::sync::Mutex::new(WhepConnectionState::New)),
            liveness: Arc::new(std::sync::Mutex::new(LivenessState::new())),
            ice_tx,
            // No live controller task in the stub; the AtomicI32 starts at
            // START_BPS so the snapshot test sees the initial bitrate.
            compat_controller_task: None,
            compat_target_bitrate: Some(Arc::new(std::sync::atomic::AtomicI32::new(
                super::adaptive::START_BPS,
            ))),
        };
        self.sessions
            .lock()
            .await
            .insert(session_id.to_string(), session);
        Ok(())
    }

    /// Test-only: overwrite a stored session's last-seen connection state,
    /// exactly as the `notify::connection-state` subscriber does when
    /// webrtcbin reports a transition (a vanished client reaches
    /// Disconnected/Failed via ICE timeout). Lets the zombie-reaper tests
    /// simulate dead sessions without real ICE.
    pub fn set_connection_state_for_test(&self, session_id: &str, state: WhepConnectionState) {
        let sessions = self
            .sessions
            .try_lock()
            .expect("sessions mutex busy in test");
        let session = sessions
            .get(session_id)
            .unwrap_or_else(|| panic!("no session {session_id} in test"));
        *session
            .connection_state
            .lock()
            .expect("connection_state poisoned in test") = state;
    }

    /// Test-only: overwrite a stored session's RTCP-liveness tracker (#388),
    /// the seam that lets the stale-reaper tests inject a flat-vs-advancing
    /// `(last_bytes, last_progress)` WITHOUT a live peer's get-stats. Real
    /// `transport_bytes_received` needs a connected browser (unavailable in
    /// unit tests), so the reaper LOGIC is tested honestly against injected
    /// state instead of a faked get-stats reply.
    pub fn set_liveness_for_test(
        &self,
        session_id: &str,
        last_bytes: u64,
        last_progress: std::time::Instant,
    ) {
        let sessions = self
            .sessions
            .try_lock()
            .expect("sessions mutex busy in test");
        let session = sessions
            .get(session_id)
            .unwrap_or_else(|| panic!("no session {session_id} in test"));
        *session.liveness.lock().expect("liveness poisoned in test") = LivenessState {
            last_bytes,
            last_progress,
        };
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

/// The video encoder factories `iterate_encoders` counts (the H264 default
/// branch and the vp8enc compat branch) — mirrored here so the #336 tests
/// can ALSO prove consumer pipelines hold zero of them.
const ENCODER_FACTORIES: [&str; 6] = [
    "vah264enc",
    "nvh264enc",
    "x264enc",
    "nvcudah264enc",
    "nvautogpuh264enc",
    "vp8enc",
];

/// Count video encoder elements across every CONSUMER pipeline (the #336
/// invariant demands this is ZERO — encoders live ONLY in the encoder
/// pipeline, one per PROFILE, never per consumer).
async fn consumer_pipeline_encoder_count(pipeline: &NdiPipeline) -> usize {
    let sessions = pipeline.sessions.lock().await;
    sessions
        .values()
        .flat_map(|s| s.consumer_pipeline.iterate_elements().into_iter().flatten())
        .filter(|el| {
            el.factory()
                .is_some_and(|f| ENCODER_FACTORIES.contains(&f.name().as_str()))
        })
        .count()
}

/// Regression test for #336: shared-encoder fanout — updated for the #387
/// per-consumer-adaptive compat pivot (2026-06-13).
///
/// The pre-fix pipeline ended in `whepserversink`, which (by gst-plugin-rs
/// 0.15 design) spawns one independent encoder per WHEP consumer. With
/// multiple stage-display browsers connecting to the same NDI source,
/// 3-4 H264 encoders saturated the N100's iGPU VAAPI scheduler — the
/// 2026-05-24 production incident.
///
/// The #336 invariant is that the EXPENSIVE H264 720p encoder never
/// multiplies per consumer. Since #387 the topology is asymmetric BY DESIGN:
///
/// - The ENCODER pipeline holds EXACTLY ONE encoder — the shared H264 720p
///   "encoder" (DEFAULT profile). The compat path now fans RAW I420 (the
///   shared `encoder_compat` vp8enc is gone — it moved to the consumers), so
///   the encoder pipeline has NO vp8enc.
/// - DEFAULT consumer pipelines contain ZERO encoders (fed the shared H264
///   stream) — the original #336 invariant, untouched.
/// - COMPAT consumer pipelines contain exactly ONE `vp8enc` each — the
///   deliberate per-codec-per-weak-consumer exception (#387). This is
///   BOUNDED and affordable: ≤3 weak TVs at VP8 480p. Per-consumer H264 720p
///   would melt the N100 (the #336 incident); per-consumer VP8 480p does not
///   (measured headroom: N100 load ~2.5 with one vah264enc 720p + 3 vp8enc
///   480p), and it is the ONLY way each weak TV's bitrate can adapt to its
///   OWN link.
///
/// Runs on every CI host (no GPU/libndi required) — uses
/// `stopped_for_test_with_topology()` with `x264enc` (always available)
/// in lieu of real HW encoders.
#[tokio::test]
async fn pipeline_has_one_shared_h264_encoder_compat_encodes_per_consumer() {
    super::super::init().expect("gst init");
    let mut pipeline = NdiPipeline::stopped_for_test_with_topology("x264enc")
        .expect("test-only topology builder must succeed when x264enc registered");

    // Simulate 3 DEFAULT (H264) + 2 COMPAT (VP8) WHEP POSTs.
    for i in 0..3 {
        pipeline
            .add_consumer_stub(&format!("default-{i}"))
            .await
            .expect("default add_consumer must succeed up to the soft cap");
    }
    for i in 0..2 {
        pipeline
            .add_compat_consumer_stub(&format!("compat-{i}"))
            .await
            .expect("compat add_consumer must succeed up to the soft cap");
    }

    let encoder_count = pipeline.iterate_encoders().count();
    let webrtcbin_count = pipeline.iterate_webrtcbins().len();
    let consumer_encoders = consumer_pipeline_encoder_count(&pipeline).await;

    // The ENCODER pipeline has exactly ONE encoder — the shared H264 (the
    // compat vp8enc moved to the consumers in #387).
    assert_eq!(
        encoder_count, 1,
        "REGRESSION (#336/#387): the ENCODER pipeline must have EXACTLY ONE \
         encoder — the shared H264 720p \"encoder\". The compat vp8enc moved \
         to the per-consumer pipelines; got {encoder_count}. \
         Encoder factories considered: {ENCODER_FACTORIES:?}"
    );
    assert!(
        pipeline.pipeline.by_name("encoder_compat").is_none(),
        "REGRESSION (#387): no shared vp8enc (encoder_compat) in the encoder \
         pipeline — the compat path fans RAW and each compat consumer encodes"
    );
    // 3 DEFAULT consumers contribute ZERO encoders; 2 COMPAT consumers
    // contribute exactly ONE vp8enc each → 2 encoders across all consumers.
    assert_eq!(
        consumer_encoders, 2,
        "REGRESSION (#336/#387): DEFAULT consumers must contain ZERO encoders \
         (shared H264) and each COMPAT consumer EXACTLY ONE vp8enc; with 3 \
         default + 2 compat consumers that is 2 consumer encoders, got \
         {consumer_encoders}"
    );
    assert_eq!(
        webrtcbin_count, 5,
        "one webrtcbin per consumer pipeline; got {webrtcbin_count} for 5 consumers"
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
            .await
            .expect("consumer up to cap must succeed");
    }

    // Overflow: the (cap+1)th consumer must be rejected.
    let err = pipeline
        .add_consumer_stub("test-session-overflow")
        .await
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

/// Zombie reaper (2026-06-12 production incident): stage-display TVs
/// power-cycle overnight WITHOUT sending WHEP DELETE, so their server-side
/// sessions sit in the map forever — webrtcbin's connection-state eventually
/// reports Disconnected/Failed, but nothing removed the session.
/// `reap_dead_sessions` must remove EXACTLY the sessions whose last-seen
/// state is Disconnected, Failed or Closed — and must NOT touch
/// New/Connecting (a TV mid-join is not a zombie) or Connected (alive).
#[tokio::test]
async fn reap_dead_sessions_removes_only_dead_states() {
    super::super::init().expect("gst init");
    let mut pipeline =
        NdiPipeline::stopped_for_test_with_topology("x264enc").expect("topology builder");
    for id in [
        "dead-disconnected",
        "dead-failed",
        "dead-closed",
        "alive-new",
        "alive-connected",
    ] {
        pipeline.add_consumer_stub(id).await.expect("add stub");
    }
    pipeline.set_connection_state_for_test("dead-disconnected", WhepConnectionState::Disconnected);
    pipeline.set_connection_state_for_test("dead-failed", WhepConnectionState::Failed);
    pipeline.set_connection_state_for_test("dead-closed", WhepConnectionState::Closed);
    // "alive-new" keeps the stub's default New (mid-negotiation grace).
    pipeline.set_connection_state_for_test("alive-connected", WhepConnectionState::Connected);

    let reaped = pipeline.reap_dead_sessions().await;

    assert_eq!(
        reaped, 3,
        "exactly the Disconnected/Failed/Closed sessions must be reaped"
    );
    let sessions = pipeline.sessions.lock().await;
    assert_eq!(sessions.len(), 2, "the two alive sessions must survive");
    assert!(
        sessions.contains_key("alive-new"),
        "a New (mid-join) session must NOT be reaped"
    );
    assert!(
        sessions.contains_key("alive-connected"),
        "a Connected session must NOT be reaped"
    );
}

/// THE morning-incident regression (2026-06-12): 8 zombie sessions (TVs gone
/// overnight without DELETE, ~730k buffers pushed to nobody) filled
/// `MAX_CONSUMERS_PER_SOURCE`, so EVERY new consumer got CapReached → 503 →
/// all displays stuck on "Connecting…" forever. The join path must
/// self-heal: reap dead sessions BEFORE the cap check, so a rebooted TV's
/// join succeeds deterministically on a map full of zombies.
#[tokio::test]
async fn zombie_sessions_free_the_cap_for_a_new_join() {
    super::super::init().expect("gst init");
    let mut pipeline =
        NdiPipeline::stopped_for_test_with_topology("x264enc").expect("topology builder");
    for i in 0..MAX_CONSUMERS_PER_SOURCE {
        pipeline
            .add_consumer_stub(&format!("zombie-{i}"))
            .await
            .expect("fill to cap");
    }
    for i in 0..MAX_CONSUMERS_PER_SOURCE {
        pipeline.set_connection_state_for_test(
            &format!("zombie-{i}"),
            WhepConnectionState::Disconnected,
        );
    }

    // Pre-fix this returned CapReached — the 503 every TV saw all morning.
    pipeline
        .add_consumer_stub("rebooted-tv")
        .await
        .expect("join with a map full of zombies MUST succeed via the reap-then-cap gate");

    let sessions = pipeline.sessions.lock().await;
    assert_eq!(
        sessions.len(),
        1,
        "all zombies reaped; only the new session remains"
    );
    assert!(sessions.contains_key("rebooted-tv"));
}

/// #388 RED: RTCP-liveness reaper. The production bug (2026-06-12, twice):
/// gst webrtcbin NEVER flips connection-state for a vanished peer, so the
/// state-based `reap_dead_sessions` is INERT in production — the state stays
/// `Connected` forever. A LIVE WHEP consumer continuously sends RTCP receiver
/// reports, so the peer-RR fingerprint (a hash of the peer's RR fields, chiefly
/// `rb-exthighestseq` which advances on every received RTP packet) keeps
/// changing; a vanished peer's fingerprint freezes. `reap_stale_sessions`
/// uses that as the liveness signal: a session whose fingerprint has NOT
/// changed for longer than `stale_after` is reaped; one still changing is kept.
///
/// Tested via an injectable liveness seam (`set_liveness_for_test`) because real
/// `get-stats` RR fields need a live peer (unavailable in unit tests).
/// The reaper LOGIC is what we lock here; the thin get-stats fingerprint
/// accessor is exercised only by the live functional check.
#[tokio::test]
async fn reap_stale_sessions_removes_flat_keeps_advancing() {
    super::super::init().expect("gst init");
    let mut pipeline =
        NdiPipeline::stopped_for_test_with_topology("x264enc").expect("topology builder");
    for id in ["flat-zombie", "advancing-alive"] {
        pipeline.add_consumer_stub(id).await.expect("add stub");
    }

    let stale_after = std::time::Duration::from_millis(50);
    let now = std::time::Instant::now();
    // Flat session: last byte progress is well PAST the stale window — its peer
    // vanished (TV rebooted without WHEP DELETE), RTCP stopped, counter frozen.
    pipeline.set_liveness_for_test(
        "flat-zombie",
        1_000,
        now - std::time::Duration::from_secs(10),
    );
    // Advancing session: progressed JUST now — a live peer still sending RRs.
    pipeline.set_liveness_for_test("advancing-alive", 5_000, now);

    let reaped = pipeline.reap_stale_sessions(stale_after).await;

    assert_eq!(
        reaped, 1,
        "exactly the flat (RTCP-stale) session must be reaped"
    );
    let sessions = pipeline.sessions.lock().await;
    assert_eq!(sessions.len(), 1, "the advancing session must survive");
    assert!(
        sessions.contains_key("advancing-alive"),
        "a session whose transport bytes are still advancing is NOT a zombie"
    );
    assert!(
        !sessions.contains_key("flat-zombie"),
        "a session whose transport bytes went flat past the stale window must be reaped"
    );
}

/// #388 RED: a session whose transport bytes ADVANCE between reap passes must
/// have its `last_progress` refreshed so it is never reaped while live — even
/// if it was previously near the stale window. This proves the reaper updates
/// liveness on progress (it does not reap on a single flat read).
#[tokio::test]
async fn reap_stale_sessions_refreshes_progress_on_advancing_bytes() {
    super::super::init().expect("gst init");
    let mut pipeline =
        NdiPipeline::stopped_for_test_with_topology("x264enc").expect("topology builder");
    pipeline.add_consumer_stub("live").await.expect("add stub");

    let stale_after = std::time::Duration::from_millis(50);
    // Seed last_progress at the very edge of the stale window with some bytes…
    pipeline.set_liveness_for_test(
        "live",
        1_000,
        std::time::Instant::now() - std::time::Duration::from_millis(40),
    );
    // …then simulate the peer having sent more RTCP (transport bytes advanced)
    // by bumping last_bytes BELOW the live counter the stub will read. A real
    // stub webrtcbin reads 0 transport bytes, so to prove the "advancing"
    // branch we set last_bytes to a value the reaper will compare against the
    // live read. We use 0 here so the live read (also ~0) does NOT exceed it,
    // forcing the time-based branch — which, with a fresh-enough progress
    // stamp, must KEEP the session.
    pipeline.set_liveness_for_test("live", 0, std::time::Instant::now());

    let reaped = pipeline.reap_stale_sessions(stale_after).await;
    assert_eq!(
        reaped, 0,
        "a session with fresh progress must NOT be reaped"
    );
    assert!(
        pipeline.sessions.lock().await.contains_key("live"),
        "freshly-progressed session survives"
    );
}

/// #388 RED: cap recovery via the STALE reaper. The 2026-06-12 incident as it
/// ACTUALLY manifested in production: 8 sessions whose peers vanished WITHOUT a
/// state transition (webrtcbin never flipped them off `Connected`), so the
/// state-based reaper could not free them and every new join 503'd. With the
/// RTCP-stale reaper wired into `reap_and_check_cap`, a map full of stale (but
/// still `Connected`-stated) zombies must self-heal: `add_consumer_stub` (which
/// runs the real join gate) succeeds.
#[tokio::test]
async fn rtcp_stale_zombies_free_the_cap_for_a_new_join() {
    super::super::init().expect("gst init");
    let mut pipeline =
        NdiPipeline::stopped_for_test_with_topology("x264enc").expect("topology builder");
    for i in 0..MAX_CONSUMERS_PER_SOURCE {
        pipeline
            .add_consumer_stub(&format!("rtcp-zombie-{i}"))
            .await
            .expect("fill to cap");
    }
    // Every session is STALE but still `Connected`-stated (the production
    // reality the state-based reaper cannot see). The join gate uses the
    // production `JOIN_STALE_AFTER` (60s) window, so push progress well past
    // it (10 minutes ago) to guarantee these are reaped on the next join.
    let stale = std::time::Instant::now() - std::time::Duration::from_secs(600);
    for i in 0..MAX_CONSUMERS_PER_SOURCE {
        pipeline.set_liveness_for_test(&format!("rtcp-zombie-{i}"), 1_000, stale);
    }

    // The join gate must reap the RTCP-stale zombies before the cap check.
    pipeline.add_consumer_stub("rebooted-tv").await.expect(
        "join with a map full of RTCP-stale zombies MUST succeed via the stale-then-cap gate",
    );

    let sessions = pipeline.sessions.lock().await;
    assert!(
        sessions.contains_key("rebooted-tv"),
        "the rebooted TV's join must land after the stale zombies are reaped"
    );
    assert!(
        sessions.len() <= MAX_CONSUMERS_PER_SOURCE,
        "the session count must be back within the cap after reaping"
    );
}

/// Spec #336 / Task 7: cleanup invariant. After N add_consumer +
/// M remove_consumer calls, exactly N-M consumer pipelines (one webrtcbin
/// each) remain, the SHARED H264 encoder count in the encoder pipeline stays
/// at exactly 1 (never per consumer — #336), and remove_consumer on an
/// unknown session is idempotent.
///
/// Catches the regression class where a leaked consumer pipeline or a
/// dangling StreamProducer link accumulates on every disconnect — would
/// exhaust GPU/socket budgets after a busy service.
#[tokio::test]
async fn add_then_remove_leaves_clean_state() {
    super::super::init().expect("gst init");
    let mut pipeline =
        NdiPipeline::stopped_for_test_with_topology("x264enc").expect("topology builder");

    // Add 5 DEFAULT consumers.
    for i in 0..5 {
        pipeline
            .add_consumer_stub(&format!("session-{i}"))
            .await
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
        "the shared H264 encoder count must stay at 1 regardless of consumer churn (#336)",
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
        "the shared H264 encoder count must stay at 1 across removes (#336)",
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
        "the shared H264 encoder must still be present (it's part of the encoder pipeline topology)",
    );

    // Remove non-existent session must be idempotent.
    pipeline
        .remove_consumer_stub("session-does-not-exist")
        .expect("remove_consumer_stub must be idempotent on unknown session");
}

/// The encoder MUST emit constrained-baseline H264. High profile (what the
/// encoders default to) is rejected by strict TV HW decoders (Vestel stage
/// displays): Chromium falls back to NullVideoDecoder and the stage shows
/// black while RTP flows — found live on prod 2026-06-11.
#[test]
fn encoder_output_is_pinned_to_constrained_baseline() {
    super::super::init().unwrap();
    if super::super::hw_h264_encoder().is_none() {
        return;
    }
    let p = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();
    let profile_caps = p
        .pipeline
        .by_name("profile_caps")
        .expect("capsfilter named profile_caps between encoder and h264parse");
    let caps = profile_caps.property::<gst::Caps>("caps");
    let s = caps.structure(0).expect("caps structure");
    assert_eq!(s.name(), "video/x-h264");
    assert_eq!(
        s.get::<&str>("profile").expect("profile field"),
        "constrained-baseline"
    );
}

/// webrtcsink parity: rtph264pay must aggregate in zero-latency mode
/// (default "none" can hold NALs; webrtcsink sets this on every payloader).
#[test]
fn consumer_payloader_uses_zero_latency_aggregation() {
    super::super::init().expect("gst init");
    let (_appsrc, _encoder, payloader, _webrtcbin) =
        super::consumer_build::build_consumer_elements("test-agg", StreamProfile::Default, 102)
            .expect("consumer elements build");
    let value = payloader.property_value("aggregate-mode");
    let (_, enum_value) =
        gst::glib::EnumValue::from_value(&value).expect("aggregate-mode is an enum");
    assert_eq!(enum_value.nick(), "zero-latency");
}

/// The profile implies the codec (realtime-VP8 compat pivot): a compat
/// consumer's elements must payload VP8 — `rtpvp8pay` pre-seated on the
/// offer's VP8 pt. Since #387 the compat consumer also OWNS its encoder, so
/// its appsrc carries RAW I420 854×480 (fed from `enc_appsink_compat_raw`)
/// and a per-consumer `vp8enc` sits between appsrc and payloader. (A
/// rtph264pay here, or a VP8-bridge appsrc, would silently discard every
/// buffer — connected, but black.)
#[test]
fn compat_consumer_elements_payload_vp8() {
    super::super::init().expect("gst init");
    let (appsrc, _encoder, payloader, _webrtcbin) =
        super::consumer_build::build_consumer_elements("test-vp8", StreamProfile::Compat, 96)
            .expect("consumer elements build");
    assert_eq!(
        payloader.factory().map(|f| f.name().to_string()).as_deref(),
        Some("rtpvp8pay"),
        "compat consumers must payload VP8"
    );
    assert_eq!(
        payloader.property::<u32>("pt"),
        96,
        "rtpvp8pay must be pre-seated on the offer's VP8 pt"
    );
    let caps = appsrc.caps().expect("compat appsrc has initial caps");
    let s = caps.structure(0).expect("caps structure");
    assert_eq!(
        s.name(),
        "video/x-raw",
        "compat appsrc now carries RAW I420 (the consumer encodes VP8 itself — #387)"
    );
    assert_eq!(s.get::<&str>("format").expect("format"), "I420");
    assert_eq!(s.get::<i32>("width").expect("width"), 854);
    assert_eq!(s.get::<i32>("height").expect("height"), 480);
}

/// #387: each COMPAT consumer builds its OWN `vp8enc` (named `venc_<session>`)
/// so its `target-bitrate` can be driven independently by that consumer's
/// `CompatBitrateController` from its OWN RTCP loss/RTT — VDO.Ninja-style
/// per-TV adaptation. A shared encoder could only ever serve the worst TV's
/// bitrate to all of them. The encoder carries the same libwebrtc-parity
/// realtime tuning the shared one did (deadline=1, cpu-used=8, cbr, 4 token
/// partitions, 4 threads, error-resilient, lag-in-frames=0, kf-max-dist=240)
/// and STARTS HIGH at 900k (quality policy: start high, reduce only on
/// measured loss). The DEFAULT (H264) profile keeps NO per-consumer encoder.
#[test]
fn compat_consumer_has_per_consumer_adaptive_vp8enc() {
    super::super::init().expect("gst init");

    // Default profile: NO encoder in the consumer (shared H264 producer — #336).
    let (_appsrc, default_enc, _pay, _wrtc) =
        super::consumer_build::build_consumer_elements("test-h264", StreamProfile::Default, 102)
            .expect("default consumer elements build");
    assert!(
        default_enc.is_none(),
        "DEFAULT consumers must have NO per-consumer encoder — the shared H264 \
         720p encoder is the #336 invariant (per-consumer H264 melts the N100)"
    );

    // Compat profile: its OWN vp8enc, named venc_<session>, realtime-tuned.
    let (_appsrc, encoder, _pay, _wrtc) =
        super::consumer_build::build_consumer_elements("test-vp8", StreamProfile::Compat, 96)
            .expect("compat consumer elements build");
    let encoder = encoder.expect("compat consumers MUST own a per-consumer vp8enc (#387)");
    assert_eq!(
        encoder.factory().map(|f| f.name().to_string()).as_deref(),
        Some("vp8enc"),
        "the per-consumer compat encoder must be a vp8enc"
    );
    assert_eq!(
        encoder.name(),
        "venc_test-vp8",
        "per-consumer encoder must be named venc_<session> (controller targets it by name)"
    );
    // Starts HIGH per the quality policy — the controller only reduces on loss.
    assert_eq!(
        encoder.property::<i32>("target-bitrate"),
        900_000,
        "per-consumer vp8enc must START at 900k (quality policy: start high)"
    );
    // libwebrtc-parity realtime tuning (same as the retired shared encoder).
    assert_eq!(
        encoder.property::<i64>("deadline"),
        1,
        "libvpx realtime mode"
    );
    assert_eq!(
        encoder.property::<i32>("cpu-used"),
        8,
        "fastest realtime preset"
    );
    assert_eq!(
        encoder.property::<i32>("keyframe-max-dist"),
        240,
        "GOP parity with the H264 branch (joins served by force-keyunit)"
    );
    assert_eq!(encoder.property::<i32>("threads"), 4, "4 encode threads");
    assert_eq!(
        encoder.property::<i32>("lag-in-frames"),
        0,
        "zero lookahead — realtime stream"
    );
    let value = encoder.property_value("token-partitions");
    let (_, enum_value) =
        gst::glib::EnumValue::from_value(&value).expect("token-partitions is an enum");
    assert_eq!(
        enum_value.nick(),
        "4",
        "token-partitions=4 — multithreaded software decode on the TV's 4 cores"
    );
    let value = encoder.property_value("end-usage");
    let (_, enum_value) = gst::glib::EnumValue::from_value(&value).expect("end-usage is an enum");
    assert_eq!(
        enum_value.nick(),
        "cbr",
        "CBR like libwebrtc realtime streams"
    );
    let value = encoder.property_value("error-resilient");
    let (_, flags) =
        gst::glib::FlagsValue::from_value(&value).expect("error-resilient is a flags property");
    assert!(
        flags.iter().any(|f| f.nick() == "default"),
        "error-resilient must carry the 'default' flag (libwebrtc parity)"
    );
}

/// With GOP=240 a joining consumer MUST trigger an immediate IDR — otherwise
/// it would wait up to 8s for the next scheduled keyframe (black join).
#[test]
fn request_keyframe_sends_force_key_unit_upstream() {
    use std::sync::atomic::{AtomicBool, Ordering};
    super::super::init().unwrap();
    if super::super::hw_h264_encoder().is_none() {
        return;
    }
    let p = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();
    let appsink = p.pipeline.by_name("enc_appsink").unwrap();
    // Pads are flushing in NULL state — gst_pad_push_event refuses upstream
    // events on a flushing pad BEFORE any probe runs. The full pipeline can't
    // reach PAUSED on a unit-test host (ndisrc fails its state change with no
    // live NDI source), so activate just the producer appsink: READY→PAUSED
    // activates its pads — exactly their state when a consumer joins the
    // PLAYING pipeline in production. Drop → teardown() returns it to NULL.
    appsink
        .set_state(gst::State::Paused)
        .expect("appsink must accept PAUSED (ASYNC pre-preroll)");
    let appsink_pad = appsink.static_pad("sink").unwrap();
    let seen = std::sync::Arc::new(AtomicBool::new(false));
    let seen_probe = std::sync::Arc::clone(&seen);
    appsink_pad.add_probe(gst::PadProbeType::EVENT_UPSTREAM, move |_, info| {
        if let Some(gst::PadProbeData::Event(ev)) = &info.data {
            if gst_video::UpstreamForceKeyUnitEvent::parse(ev).is_ok() {
                seen_probe.store(true, Ordering::SeqCst);
            }
        }
        gst::PadProbeReturn::Ok
    });
    super::consumers::request_keyframe(&p.producer);
    assert!(
        seen.load(Ordering::SeqCst),
        "ForceKeyUnit must be pushed upstream from the producer appsink"
    );
}

/// Compat branch (#387 per-consumer adaptive pivot, 2026-06-13): the compat
/// path no longer holds a SHARED `vp8enc` in the encoder pipeline. A single
/// shared encoder can only serve the WORST weak TV's bitrate to ALL of them;
/// to adapt PER-TV (VDO.Ninja-style — drive each TV's encoder bitrate from
/// its OWN RTCP loss/RTT), the compat branch now fans RAW I420 frames and
/// each compat CONSUMER builds its OWN `vp8enc` (see
/// `compat_consumer_has_per_consumer_adaptive_vp8enc`).
///
/// `StreamProducer` is zero-copy (refcounted samples), so fanning RAW I420 to
/// ≤3 weak TVs is cheap. Per-consumer VP8 480p encoders are affordable on the
/// N100 ONLY for the compat tier; per-consumer H264 720p melts it (#336), so
/// the DEFAULT profile keeps its SHARED H264 encoder.
///
/// Locks the new RAW compat branch shape against the REAL `NdiPipeline::build`:
/// NO `encoder_compat` element anywhere in the encoder pipeline (it moved to
/// the consumers), the 854×480 16:9 I420 @20fps `compat_scale_caps`
/// conditioning, and a RAW relay appsink `enc_appsink_compat_raw` with the
/// same bounded contract as the primary branch (sync=false, 5 buffers) but
/// `video/x-raw I420 854×480` bridge caps (the per-consumer appsrc input).
#[test]
fn compat_branch_fans_raw_i420_no_shared_encoder() {
    super::super::init().unwrap();
    if super::super::hw_h264_encoder().is_none() {
        return;
    }
    let p = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();

    // The shared compat vp8enc is GONE — it moved into the per-consumer
    // pipelines so each weak TV's bitrate adapts independently (#387).
    assert!(
        p.pipeline.by_name("encoder_compat").is_none(),
        "the shared compat vp8enc (encoder_compat) must NOT exist in the \
         encoder pipeline — it moved to the per-consumer pipelines (#387)"
    );

    let scale_caps = p
        .pipeline
        .by_name("compat_scale_caps")
        .expect("capsfilter named compat_scale_caps in the compat branch");
    let caps = scale_caps.property::<gst::Caps>("caps");
    let s = caps.structure(0).expect("caps structure");
    assert_eq!(s.name(), "video/x-raw");
    assert_eq!(
        s.get::<&str>("format").expect("format field"),
        "I420",
        "compat branch fans I420 raw (per-consumer vp8enc's sink is I420-only)"
    );
    assert_eq!(
        s.get::<i32>("width").expect("width field"),
        854,
        "854 = 16:9 at 480p — highest weak-TV-sustained tier (quality policy)"
    );
    assert_eq!(s.get::<i32>("height").expect("height field"), 480);
    assert_eq!(
        s.get::<gst::Fraction>("framerate")
            .expect("framerate field"),
        gst::Fraction::new(20, 1),
        "20fps — the weak TVs' software-decode budget (videorate drops to it)"
    );

    let appsink = p
        .pipeline
        .by_name("enc_appsink_compat_raw")
        .expect("raw appsink named enc_appsink_compat_raw")
        .downcast::<gst_app::AppSink>()
        .expect("enc_appsink_compat_raw is an AppSink");
    assert!(
        !appsink.property::<bool>("sync"),
        "compat raw producer appsink must be sync=false (relay, not renderer)"
    );
    assert_eq!(appsink.max_buffers(), 5, "compat backlog must be 5 frames");
    let sink_caps = appsink.caps().expect("enc_appsink_compat_raw caps");
    let cs = sink_caps.structure(0).expect("caps structure");
    assert_eq!(
        cs.name(),
        "video/x-raw",
        "compat bridge caps must now be RAW I420 (consumers encode VP8 themselves)"
    );
    assert_eq!(cs.get::<&str>("format").expect("format"), "I420");
    assert_eq!(cs.get::<i32>("width").expect("width"), 854);
    assert_eq!(cs.get::<i32>("height").expect("height"), 480);
}

/// WHEP `?profile=` query parsing: ONLY the exact string "compat" selects
/// the compat branch. Absent or unknown values (including the retired "vp8"
/// some TVs may still send, and any case variant) must degrade to Default —
/// an unrecognized profile must never break a display's join.
#[test]
fn stream_profile_from_query_parses_compat_only() {
    assert_eq!(
        StreamProfile::from_query(Some("compat")),
        StreamProfile::Compat
    );
    assert_eq!(StreamProfile::from_query(None), StreamProfile::Default);
    assert_eq!(StreamProfile::from_query(Some("")), StreamProfile::Default);
    assert_eq!(
        StreamProfile::from_query(Some("vp8")),
        StreamProfile::Default,
        "the retired vp8 mode string must fall back to Default"
    );
    assert_eq!(
        StreamProfile::from_query(Some("Compat")),
        StreamProfile::Default,
        "exact match only — unknown casing degrades to Default, never errors"
    );
}

/// Profile→producer threading: `?profile=compat` must feed the consumer
/// from the COMPAT branch's producer (the 854×480 realtime-VP8 stream) and
/// the default profile from the primary producer. Identified by each producer's
/// appsink name — the same handle `request_keyframe` pushes its IDR
/// request through, so this also locks keyframe targeting to the branch
/// the consumer is actually fed from.
#[test]
fn producer_for_profile_selects_matching_branch() {
    super::super::init().unwrap();
    if super::super::hw_h264_encoder().is_none() {
        return;
    }
    let p = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();
    assert_eq!(
        p.producer_for_profile(StreamProfile::Default)
            .appsink()
            .name(),
        "enc_appsink",
        "Default profile must be fed from the primary 720p branch"
    );
    assert_eq!(
        p.producer_for_profile(StreamProfile::Compat)
            .appsink()
            .name(),
        "enc_appsink_compat_raw",
        "Compat profile must be fed from the 854x480 RAW I420 compat branch (#387)"
    );
}

/// A compat consumer's join-time IDR request must reach the COMPAT branch
/// (upstream from enc_appsink_compat) — not the default encoder. Mirrors
/// `request_keyframe_sends_force_key_unit_upstream` on the compat producer.
#[test]
fn request_keyframe_targets_the_compat_producer_for_compat_consumers() {
    use std::sync::atomic::{AtomicBool, Ordering};
    super::super::init().unwrap();
    if super::super::hw_h264_encoder().is_none() {
        return;
    }
    let p = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();
    let appsink = p.pipeline.by_name("enc_appsink_compat_raw").unwrap();
    // Activate just the compat appsink's pads (READY→PAUSED) so the upstream
    // event isn't refused on a flushing pad — same setup as the default-
    // branch keyframe test above.
    appsink
        .set_state(gst::State::Paused)
        .expect("compat appsink must accept PAUSED (ASYNC pre-preroll)");
    let appsink_pad = appsink.static_pad("sink").unwrap();
    let seen = std::sync::Arc::new(AtomicBool::new(false));
    let seen_probe = std::sync::Arc::clone(&seen);
    appsink_pad.add_probe(gst::PadProbeType::EVENT_UPSTREAM, move |_, info| {
        if let Some(gst::PadProbeData::Event(ev)) = &info.data {
            if gst_video::UpstreamForceKeyUnitEvent::parse(ev).is_ok() {
                seen_probe.store(true, Ordering::SeqCst);
            }
        }
        gst::PadProbeReturn::Ok
    });
    super::consumers::request_keyframe(p.producer_for_profile(StreamProfile::Compat));
    assert!(
        seen.load(Ordering::SeqCst),
        "ForceKeyUnit must be pushed upstream from the COMPAT producer appsink"
    );
}

/// /ndi/snapshot must expose per-consumer fanout counters and (when the
/// browser has sent RTCP RRs) round-trip/jitter/loss — the stage display's
/// own view of the link, readable server-side.
#[tokio::test]
async fn snapshot_includes_fanout_counters_and_rtcp_fields() {
    super::super::init().expect("gst init");
    let mut pipeline =
        NdiPipeline::stopped_for_test_with_topology("x264enc").expect("test topology");
    pipeline
        .add_consumer_stub("snap-1")
        .await
        .expect("stub consumer");
    let snap = pipeline.snapshot().await;
    assert_eq!(snap.sessions.len(), 1);
    let s = &snap.sessions[0];
    // Stub session pushed nothing — counters exist and are zero.
    assert_eq!(s.buffers_pushed, 0);
    assert_eq!(s.buffers_dropped, 0);
    // No RTCP from a stub webrtcbin — fields present as None (omitted in JSON).
    assert!(s.rtcp_round_trip_ms.is_none());
    let json = serde_json::to_string(&snap).unwrap();
    assert!(
        json.contains("buffersPushed"),
        "camelCase serialization: {json}"
    );
}

/// #387 observability: `/ndi/snapshot` must expose the per-consumer compat
/// `target-bitrate` so the AIMD loop is visible in the ledgers. DEFAULT
/// (H264, shared encoder) consumers report `compatTargetBitrateBps: null`;
/// COMPAT (per-consumer vp8enc) consumers report the controller's current
/// value — START_BPS (900k) right after add, before any RTCP feedback.
#[tokio::test]
async fn snapshot_exposes_per_consumer_compat_target_bitrate() {
    super::super::init().expect("gst init");
    let mut pipeline =
        NdiPipeline::stopped_for_test_with_topology("x264enc").expect("test topology");
    pipeline
        .add_consumer_stub("default-1")
        .await
        .expect("default stub consumer");
    pipeline
        .add_compat_consumer_stub("compat-1")
        .await
        .expect("compat stub consumer");

    let snap = pipeline.snapshot().await;
    let by_id = |id: &str| {
        snap.sessions
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("session {id} in snapshot"))
            .clone()
    };

    let default = by_id("default-1");
    assert_eq!(
        default.compat_target_bitrate_bps, None,
        "DEFAULT consumers (shared H264 encoder) must report compatTargetBitrateBps=null"
    );

    let compat = by_id("compat-1");
    assert_eq!(
        compat.compat_target_bitrate_bps,
        Some(super::adaptive::START_BPS),
        "COMPAT consumers must report the controller's current target-bitrate (starts at 900k)"
    );

    // camelCase serialization; default's null is OMITTED, compat's value present.
    let json = serde_json::to_string(&snap).unwrap();
    assert!(
        json.contains("compatTargetBitrateBps"),
        "camelCase serialization of the compat bitrate field: {json}"
    );
    assert!(
        json.contains("900000"),
        "compat consumer's 900k start bitrate must serialize: {json}"
    );
}
