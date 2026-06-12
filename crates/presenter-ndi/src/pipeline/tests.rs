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
        // producer_vp8 mirrors production's parallel VP8 branch with another
        // detached appsink (no VP8 fanout is exercised by these tests).
        let appsink_vp8 = gst_app::AppSink::builder().build();
        Self {
            pipeline: gst::Pipeline::new(),
            whep_url: String::new(),
            state_tx,
            state_rx,
            bus_watch: std::sync::Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            producer: StreamProducer::from(&appsink),
            producer_vp8: StreamProducer::from(&appsink_vp8),
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
        // The topology stub keeps the H264-only structure (its tests assert
        // encoder-count/webrtcbin-count invariants, not the VP8 branch — that
        // branch is locked against the REAL `NdiPipeline::build` by
        // `pipeline_has_parallel_vp8_branch_with_low_latency_props`). The
        // struct field is filled from a detached appsink, like
        // `stopped_for_test`.
        let appsink_vp8 = gst_app::AppSink::builder().build();
        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);
        Ok(Self {
            pipeline,
            whep_url: String::new(),
            state_tx,
            state_rx,
            bus_watch: std::sync::Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            producer,
            producer_vp8: StreamProducer::from(&appsink_vp8),
        })
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
            ice_tx,
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

/// The H264 encoder factories `iterate_encoders` counts — mirrored here so
/// the #336 tests can ALSO prove consumer pipelines hold zero of them.
const H264_ENCODER_FACTORIES: [&str; 5] = [
    "vah264enc",
    "nvh264enc",
    "x264enc",
    "nvcudah264enc",
    "nvautogpuh264enc",
];

/// Count H264 encoder elements across every CONSUMER pipeline (the #336
/// invariant demands this is ZERO — encoders live ONLY in the encoder
/// pipeline, one per PROFILE, never per consumer).
async fn consumer_pipeline_encoder_count(pipeline: &NdiPipeline) -> usize {
    let sessions = pipeline.sessions.lock().await;
    sessions
        .values()
        .flat_map(|s| s.consumer_pipeline.iterate_elements().into_iter().flatten())
        .filter(|el| {
            el.factory()
                .is_some_and(|f| H264_ENCODER_FACTORIES.contains(&f.name().as_str()))
        })
        .count()
}

/// Regression test for #336: shared-encoder fanout — updated for the
/// two-profile pivot (2026-06-12).
///
/// The pre-fix pipeline ended in `whepserversink`, which (by gst-plugin-rs
/// 0.15 design) spawns one independent encoder per WHEP consumer. With
/// multiple stage-display browsers connecting to the same NDI source,
/// 3-4 encoders saturated the N100's iGPU VAAPI scheduler — the
/// 2026-05-24 production incident.
///
/// The ACTUAL #336 invariant is that encoders never multiply PER CONSUMER.
/// Since the compat-profile pivot the ENCODER pipeline holds EXACTLY TWO
/// H264 encoder elements BY DESIGN — "encoder" (720p default profile) and
/// "encoder_compat" (640×480 compat profile): one per PROFILE. Consumer
/// pipelines (appsrc → rtph264pay → webrtcbin, fed via StreamProducer)
/// contain ZERO encoders. This test asserts both: regardless of how many
/// consumers are added, the encoder pipeline yields exactly TWO encoder
/// elements, the consumer pipelines yield one webrtcbin each and NO encoder.
///
/// Runs on every CI host (no GPU/libndi required) — uses
/// `stopped_for_test_with_topology()` with `x264enc` (always available)
/// in lieu of real HW encoders.
#[tokio::test]
async fn pipeline_has_exactly_two_profile_encoders_for_n_consumers() {
    super::super::init().expect("gst init");
    let mut pipeline = NdiPipeline::stopped_for_test_with_topology("x264enc")
        .expect("test-only topology builder must succeed when x264enc registered");

    // Simulate N WHEP POSTs.
    for i in 0..4 {
        pipeline
            .add_consumer_stub(&format!("test-session-{i}"))
            .await
            .expect("add_consumer must succeed up to the soft cap");
    }

    let encoder_count = pipeline.iterate_encoders().count();
    let webrtcbin_count = pipeline.iterate_webrtcbins().len();
    let consumer_encoders = consumer_pipeline_encoder_count(&pipeline).await;

    assert_eq!(
        encoder_count, 2,
        "REGRESSION (#336, two-profile pivot): the ENCODER pipeline must have \
         EXACTLY TWO encoders — one per PROFILE (encoder + encoder_compat), \
         NEVER one per consumer; got {encoder_count} encoders for 4 consumers. \
         Encoder factories considered: {H264_ENCODER_FACTORIES:?}"
    );
    assert_eq!(
        consumer_encoders, 0,
        "REGRESSION (#336): consumer pipelines must contain ZERO encoders \
         (appsrc → rtph264pay → webrtcbin only); got {consumer_encoders}"
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

/// Spec #336 / Task 7: cleanup invariant. After N add_consumer +
/// M remove_consumer calls, exactly N-M consumer pipelines (one webrtcbin
/// each) remain, the encoder count stays at exactly 2 (one per PROFILE —
/// encoder + encoder_compat, never per consumer), and remove_consumer
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
        2,
        "encoder count must stay at 2 (one per profile) regardless of consumer churn",
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
        2,
        "encoder count must stay at 2 (one per profile) across removes",
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
        2,
        "both profile encoders must still be present (they're part of the encoder pipeline topology)",
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
    let (_appsrc, payloader, _webrtcbin) = super::consumers::build_consumer_elements(
        "test-agg",
        super::consumers::ConsumerCodec::H264 { pt: 102 },
    )
    .expect("consumer elements build");
    let value = payloader.property_value("aggregate-mode");
    let (_, enum_value) =
        gst::glib::EnumValue::from_value(&value).expect("aggregate-mode is an enum");
    assert_eq!(enum_value.nick(), "zero-latency");
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

/// Compat branch (spec addendum 2 pivot, 2026-06-12): the weak Vestel TVs'
/// MStar H264 OMX decoder (`OMX.MS.AVC.Decoder`) dies ONLY on output-port
/// reconfiguration — it default-inits at 640×480 and a 1280×720 stream
/// forces a reconfig that fails (`setParameter(ParamPortDefinition)
/// BadParameter`, codec torn down every GOP; logcat-proven). The abandoned
/// VP8 answer cost ~26fps WITH 37 freezes on the TVs (software decode) and
/// load ~10 on the prod N100 (software vp8enc). A stream that IS EXACTLY
/// 640×480 H264 avoids the port reconfig entirely → HW decode on the TV
/// (zero TV CPU) + GPU encode on the server (near-zero N100 CPU).
///
/// Locks the full compat branch shape against the REAL `NdiPipeline::build`:
/// 640×480 NV12 scale caps (NO videoconvert — the tee already carries NV12,
/// which every H264 encoder here accepts), a SECOND H264 encoder
/// ("encoder_compat", SAME factory as the primary so both stay hardware),
/// constrained-baseline pinning, a dedicated h264parse, and the same bounded
/// relay appsink contract as the primary branch (sync=false, 5 buffers,
/// byte-stream/AU H264 bridge caps).
#[test]
fn compat_branch_is_h264_640x480_with_relay_props() {
    super::super::init().unwrap();
    if super::super::hw_h264_encoder().is_none() {
        return;
    }
    let p = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();

    let scale_caps = p
        .pipeline
        .by_name("compat_scale_caps")
        .expect("capsfilter named compat_scale_caps in the compat branch");
    let caps = scale_caps.property::<gst::Caps>("caps");
    let s = caps.structure(0).expect("caps structure");
    assert_eq!(s.name(), "video/x-raw");
    assert_eq!(
        s.get::<&str>("format").expect("format field"),
        "NV12",
        "compat branch must stay NV12 — the tee output is already NV12 (no videoconvert)"
    );
    assert_eq!(
        s.get::<i32>("width").expect("width field"),
        640,
        "EXACTLY 640 — the MStar OMX default-init width; any other width forces the fatal port reconfig"
    );
    assert_eq!(
        s.get::<i32>("height").expect("height field"),
        480,
        "EXACTLY 480 — the MStar OMX default-init height"
    );

    let encoder = p.pipeline.by_name("encoder").expect("primary encoder");
    let compat = p
        .pipeline
        .by_name("encoder_compat")
        .expect("H264 encoder named encoder_compat in the compat branch");
    assert_eq!(
        compat.factory().map(|f| f.name()),
        encoder.factory().map(|f| f.name()),
        "compat encoder must use the SAME factory as the primary (hardware encode on both branches)"
    );

    let profile_caps = p
        .pipeline
        .by_name("compat_profile_caps")
        .expect("capsfilter named compat_profile_caps between encoder_compat and h264parse_compat");
    let caps = profile_caps.property::<gst::Caps>("caps");
    let s = caps.structure(0).expect("caps structure");
    assert_eq!(s.name(), "video/x-h264");
    assert_eq!(
        s.get::<&str>("profile").expect("profile field"),
        "constrained-baseline",
        "compat branch must pin constrained-baseline like the primary (strict TV decoders)"
    );

    let parse = p
        .pipeline
        .by_name("h264parse_compat")
        .expect("h264parse named h264parse_compat in the compat branch");
    assert_eq!(
        parse.property::<i32>("config-interval"),
        -1,
        "SPS/PPS before every IDR so compat joiners start decoding immediately"
    );

    let appsink = p
        .pipeline
        .by_name("enc_appsink_compat")
        .expect("appsink named enc_appsink_compat")
        .downcast::<gst_app::AppSink>()
        .expect("enc_appsink_compat is an AppSink");
    assert!(
        !appsink.property::<bool>("sync"),
        "compat producer appsink must be sync=false (relay, not renderer)"
    );
    assert_eq!(appsink.max_buffers(), 5, "compat backlog must be 5 frames");
    let sink_caps = appsink.caps().expect("enc_appsink_compat caps");
    assert_eq!(
        sink_caps.structure(0).expect("caps structure").name(),
        "video/x-h264",
        "compat bridge caps must be H264 (consumers parse it with the same rtph264pay path)"
    );

    assert_eq!(
        p.iterate_encoders().count(),
        2,
        "EXACTLY TWO H264 encoders by design — one per profile (encoder + encoder_compat)"
    );
}

/// Codec selection rule (spec addendum 2, deterministic): H264 whenever the
/// offer contains it — today's behavior, zero change for healthy clients
/// (Chrome's default offer lists VP8 first but ALWAYS includes H264).
#[test]
fn select_codec_prefers_h264_when_offer_has_both() {
    let offer = "v=0\r\n\
                 m=video 9 UDP/TLS/RTP/SAVPF 100 103\r\n\
                 a=rtpmap:100 VP8/90000\r\n\
                 a=rtpmap:103 H264/90000\r\n";
    assert_eq!(
        super::consumers::select_codec(offer),
        Some(super::consumers::ConsumerCodec::H264 { pt: 103 }),
        "H264 must win whenever the offer carries it, even when VP8 is listed first"
    );
}

/// VP8 is served ONLY when the offer carries NO H264 — exactly what the
/// fallback client produces via setCodecPreferences (VP8+rtx only).
#[test]
fn select_codec_falls_back_to_vp8_when_no_h264() {
    let offer = "v=0\r\n\
                 m=video 9 UDP/TLS/RTP/SAVPF 100 101\r\n\
                 a=rtpmap:100 VP8/90000\r\n\
                 a=rtpmap:101 rtx/90000\r\n";
    assert_eq!(
        super::consumers::select_codec(offer),
        Some(super::consumers::ConsumerCodec::Vp8 { pt: 100 }),
    );
}

/// Neither H264 nor VP8 in the offer → no codec to serve; add_consumer must
/// take the error path instead of building a pipeline the browser can never
/// decode (the old behavior preset rtph264pay pt=96 and NEVER worked).
#[test]
fn select_codec_returns_none_when_neither_codec_offered() {
    let offer = "v=0\r\n\
                 m=video 9 UDP/TLS/RTP/SAVPF 98\r\n\
                 a=rtpmap:98 VP9/90000\r\n";
    assert_eq!(super::consumers::select_codec(offer), None);
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
