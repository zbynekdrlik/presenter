//! #854 regression tests on a REAL GStreamer pipeline.
//!
//! CI cannot see the bug on its own: GitHub runners encode with `x264enc` and
//! dev2 with NVENC, both of which keep the input (receive-time) PTS, and the
//! synthetic NDI sender runs on the server's own clock. Prod's `vah264enc`
//! 1.24.2 instead stamps its output with a frame counter
//! (`pts = dts = start_pts + frame_duration × frame_count`,
//! gstvah264enc.c:3051), which runs ahead of the clock whenever the source is
//! faster than its declared rate.
//!
//! So these tests drive a live x264enc pipeline through a test-only probe
//! that re-stamps its output EXACTLY like vah264enc 1.24 does, with the source
//! delivering 40 fps against a 30 fps counter (the counter gains 333 ms per
//! second: PP's +48 ppm condition, compressed from hours to seconds). The
//! encoder side uses the production `profile_caps`, `h264parse` + appsink and
//! the production guard install; the consumer side is prod-shaped:
//! `StreamProducer` → appsrc (`configure_consumer`: live, leaky, 500 ms
//! `max-time`) on the encoder's clock + base time (`adopt_encoder_timeline`)
//! → `clocksync sync=true` (what webrtcbin 1.24 inserts on every input
//! stream) → fakesink. Unguarded, the clocksync holds the future-stamped
//! buffers and the 500 ms queue overflows once the lead passes 500 ms — the
//! PP SD1 failure. x264enc missing is a hard failure, never a skip.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_utils::StreamProducer;

use crate::pipeline::build::{build_parse_and_sink, build_profile_caps, consumer_h264_caps};
use crate::pipeline::consumers::{adopt_encoder_timeline, request_keyframe};
use crate::pipeline::encoder_timing::{self, EncoderTiming};

/// vah264enc's `frame_duration` for the nominal 30/1 caps rate.
const COUNTER_FRAME_NS: u64 = 1_000_000_000 / 30;

/// The encoder half: live source → x264enc → production profile caps →
/// production h264parse + appsink, guarded by the production install, wrapped
/// in a `StreamProducer` exactly like `NdiPipeline::build`.
struct EncoderSide {
    pipeline: gst::Pipeline,
    producer: StreamProducer,
    timing: Arc<EncoderTiming>,
}

fn build_encoder_side(fps: i32, emulate_vah264enc_1_24: bool) -> EncoderSide {
    crate::init().expect("gstreamer init");
    let pipeline = gst::Pipeline::new();
    // Prod pins the encoder pipeline to the system monotonic clock.
    pipeline.use_clock(Some(&gst::SystemClock::obtain()));
    let src = gst::ElementFactory::make("videotestsrc")
        .property("is-live", true)
        .build()
        .expect("videotestsrc");
    let raw_caps = gst::ElementFactory::make("capsfilter")
        .property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("format", "I420")
                .field("width", 320i32)
                .field("height", 240i32)
                .field("framerate", gst::Fraction::new(fps, 1))
                .build(),
        )
        .build()
        .expect("raw capsfilter");
    let encoder = gst::ElementFactory::make("x264enc")
        .name("encoder")
        .property_from_str("tune", "zerolatency")
        .property_from_str("speed-preset", "ultrafast")
        .property("key-int-max", 30u32)
        .build()
        .expect("x264enc must be installed (gstreamer1.0-plugins-ugly) — the #854 regression cannot run without it");
    let profile_caps = build_profile_caps().expect("production profile caps");
    let (h264parse, appsink) = build_parse_and_sink().expect("production h264parse + appsink");
    let sink_el = appsink.upcast_ref::<gst::Element>();
    pipeline
        .add_many([
            &src,
            &raw_caps,
            &encoder,
            &profile_caps,
            &h264parse,
            sink_el,
        ])
        .expect("add encoder-side elements");
    gst::Element::link_many([
        &src,
        &raw_caps,
        &encoder,
        &profile_caps,
        &h264parse,
        sink_el,
    ])
    .expect("link encoder side");
    if emulate_vah264enc_1_24 {
        // Added BEFORE the guard: probes run in the order they were added, so
        // the guard sees the frame-counter stamps, as on prod.
        install_frame_counter_stamping(&encoder);
    }
    let timing = encoder_timing::install(&encoder, sink_el, "encoder-timestamps-test");
    let producer = StreamProducer::with(
        &appsink,
        gstreamer_utils::streamproducer::ProducerSettings { sync: false },
    );
    EncoderSide {
        pipeline,
        producer,
        timing,
    }
}

/// Re-stamp every encoder output buffer as vah264enc 1.24.2 does:
/// `pts = dts = first_pts + frame_duration × n` (gstvah264enc.c:3051-3056 with
/// no B-frames), discarding the input PTS.
fn install_frame_counter_stamping(encoder: &gst::Element) {
    let pad = encoder.static_pad("src").expect("encoder src pad");
    let counter: Mutex<Option<(gst::ClockTime, u64)>> = Mutex::new(None);
    pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
        if let Some(buffer) = info.buffer_mut() {
            if let Some(pts) = buffer.pts() {
                let mut counter = counter.lock().expect("counter lock");
                let (first, n) = counter.get_or_insert((pts, 0));
                let stamped = *first + gst::ClockTime::from_nseconds(COUNTER_FRAME_NS * *n);
                *n += 1;
                let buf = buffer.make_mut();
                buf.set_pts(stamped);
                buf.set_dts(stamped);
            }
        }
        gst::PadProbeReturn::Ok
    })
    .expect("frame-counter probe added");
}

/// The prod-shaped consumer: appsrc → clocksync(sync=true) → fakesink on the
/// encoder pipeline's clock + base time, PLAYING before it is connected.
fn build_consumer_side(encoder_pipeline: &gst::Pipeline) -> (gst::Pipeline, gst_app::AppSrc) {
    let pipeline = gst::Pipeline::new();
    let appsrc = gst_app::AppSrc::builder()
        .caps(&consumer_h264_caps())
        .build();
    StreamProducer::configure_consumer(&appsrc);
    let clocksync = gst::ElementFactory::make("clocksync")
        .property("sync", true)
        .build()
        .expect("clocksync (core elements)");
    let sink = gst::ElementFactory::make("fakesink")
        .property("sync", false)
        .property("async", false)
        .build()
        .expect("fakesink");
    let src_el = appsrc.upcast_ref::<gst::Element>();
    pipeline
        .add_many([src_el, &clocksync, &sink])
        .expect("add consumer elements");
    gst::Element::link_many([src_el, &clocksync, &sink]).expect("link consumer");
    adopt_encoder_timeline(
        &pipeline,
        encoder_pipeline.clock(),
        encoder_pipeline.base_time(),
        "encoder-timestamps-test",
    );
    pipeline
        .set_state(gst::State::Playing)
        .expect("consumer pipeline PLAYING");
    (pipeline, appsrc)
}

/// What one run measured.
struct RunResult {
    /// Buffers forwarded to the consumer after the join settled.
    pushed_after_settle: u64,
    /// Buffers dropped for the consumer after the join settled.
    dropped_after_settle: u64,
    /// Trailing-30s lead at the producer appsink, read before teardown.
    lead_30s: Option<(i64, i64)>,
}

/// Start the encoder, join one consumer, let the join settle (`settle` —
/// join-until-IDR drops land here), then measure for `measure`.
fn run_with_consumer(side: &EncoderSide, settle: Duration, measure: Duration) -> RunResult {
    side.pipeline
        .set_state(gst::State::Playing)
        .expect("encoder pipeline PLAYING");
    // Let the live source run and the base time settle before the join.
    std::thread::sleep(Duration::from_millis(300));
    let (consumer, appsrc) = build_consumer_side(&side.pipeline);
    let link = side
        .producer
        .add_consumer(&appsrc)
        .expect("connect the consumer");
    request_keyframe(&side.producer);
    std::thread::sleep(settle);
    let (pushed0, dropped0) = (link.pushed(), link.dropped());
    std::thread::sleep(measure);
    let (pushed1, dropped1) = (link.pushed(), link.dropped());
    let lead_30s = side.timing.lead_window_ms(Duration::from_secs(30));
    drop(link);
    let _ = consumer.set_state(gst::State::Null);
    let _ = side.pipeline.set_state(gst::State::Null);
    RunResult {
        pushed_after_settle: pushed1 - pushed0,
        dropped_after_settle: dropped1 - dropped0,
        lead_30s,
    }
}

/// THE #854 regression: a vah264enc-1.24-style frame counter on a source
/// faster than the counter must never turn into a consumer hold.
#[test]
fn frame_counter_stamping_never_becomes_a_consumer_hold() {
    let side = build_encoder_side(40, true);
    let run = run_with_consumer(&side, Duration::from_secs(1), Duration::from_millis(3_500));
    let snap = side.timing.snapshot();
    eprintln!(
        "[#854] pushed {} dropped {} after settle; lead30s {:?}; timing {:?}",
        run.pushed_after_settle, run.dropped_after_settle, run.lead_30s, snap
    );
    assert_eq!(
        run.dropped_after_settle, 0,
        "{} frames dropped after the join settled: the frame-counter stamps ran \
         ahead of the clock, clocksync held them and the 500 ms consumer queue \
         overflowed — the #854 PP SD1 failure",
        run.dropped_after_settle
    );
    assert!(
        run.pushed_after_settle >= 100,
        "only {} frames reached the consumer in 3.5 s at 40 fps",
        run.pushed_after_settle
    );
    let (_, lead_max) = run.lead_30s.expect("the lead meter saw frames");
    assert!(
        lead_max <= 5,
        "shared encoder output stamped {lead_max} ms ahead of the clock"
    );
    // The emulated PP condition really happened (the harness is not vacuous):
    // unguarded, these stamps would have been >= 500 ms in the future.
    assert!(snap.frames_pulled_back > 0, "guard never engaged: {snap:?}");
    assert!(
        snap.raw_lead_max_ms.unwrap_or(0) >= 500,
        "emulated counter lead never passed 500 ms: {snap:?}"
    );
}

/// On an encoder that keeps the input PTS (x264enc here; NVENC on dev2;
/// vah264enc >= 1.26) the guard must change nothing at all.
#[test]
fn guard_is_an_identity_when_the_encoder_keeps_pts() {
    let side = build_encoder_side(30, false);
    let run = run_with_consumer(
        &side,
        Duration::from_millis(500),
        Duration::from_millis(1_500),
    );
    let snap = side.timing.snapshot();
    eprintln!(
        "[#854] identity run: lead30s {:?}; timing {snap:?}",
        run.lead_30s
    );
    assert!(snap.installed, "guard + lead meter installed");
    assert!(snap.frames_seen >= 40, "frames seen: {snap:?}");
    assert_eq!(snap.frames_pulled_back, 0, "{snap:?}");
    assert_eq!(snap.frames_reanchored, 0, "{snap:?}");
    assert_eq!(snap.frames_unmeasured, 0, "{snap:?}");
    assert_eq!(snap.correction_ms, 0, "{snap:?}");
    let (_, lead_max) = run.lead_30s.expect("the lead meter saw frames");
    assert!(
        lead_max <= 0,
        "receive-time stamps are never ahead: {lead_max} ms"
    );
    assert_eq!(run.dropped_after_settle, 0, "a healthy link drops nothing");
}
