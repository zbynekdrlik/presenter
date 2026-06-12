//! Pipeline construction: build the shared-encoder ENCODER pipeline
//! (`ndisrc → ndisrcdemux → videoconvert → videoscale → caps → raw_tee`,
//! fanning into TWO parallel encode branches — one per stream profile:
//! default `→ q_default → encoder → profile_caps → h264parse → enc_appsink`
//! (1280×720 H264) and compat `→ q_compat → videorate → videoconvert →
//! videoscale → compat_scale_caps → encoder_compat(vp8enc) →
//! enc_appsink_compat` (854×480@20 realtime VP8). Each appsink is wrapped by
//! a `gstreamer_utils::StreamProducer` which fans the encoded stream out to
//! the per-consumer `appsrc → rtph264pay|rtpvp8pay → webrtcbin` pipelines
//! built in `add_consumer` (see `consumers.rs`); a consumer selects its
//! producer via the WHEP `?profile=compat` query (see `StreamProfile`). The
//! compat branch exists because the weak stage TVs' MStar H264 OMX decoder
//! is vendor-broken (even an exactly-640×480 H264 stream dies after ~5s) —
//! but VDO.Ninja's libwebrtc VP8 has played smoothly on the SAME TVs for
//! years, so compat mirrors its realtime stream properties, above all
//! token-partitioned VP8 for multithreaded TV decode (see
//! `build_compat_vp8_encoder`).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_utils::StreamProducer;
use tokio::sync::watch;

use super::{NdiPipeline, PipelineState};

/// Stage-display-safe encode resolution. NDI sources can be 1080p/1440p/4K, but
/// stage displays are low-cost TVs and browsers negotiate a bounded H264 level —
/// encoding above this yields an undecodable stream (black). 720p 16:9 is
/// universally decodable and matches the 2.5 Mbps target. Sources are downscaled
/// (and letterboxed if not 16:9) to this before the encoder; see `build`.
const MAX_VIDEO_WIDTH: i32 = 1280;
const MAX_VIDEO_HEIGHT: i32 = 720;

/// Compat-profile resolution: 854×480 — 16:9 at 480p, so the picture fills
/// the full width (the abandoned 640×480 H264 attempt letterboxed 16:9 into
/// 4:3 AND still killed the TVs' vendor-broken MStar OMX decoder after ~5s).
/// VP8 has no decoder port-reconfig trap — libvpx software-decodes any size;
/// FLOOR-FINDING (2026-06-12): 480p@20 still stalled the Vestels (>10s
/// presentation gaps, watchdog resets) even on the lite plain-JS page, so
/// this probes the platform's actual ceiling from BELOW — 360p@15@500kbps
/// is the lane VDO.Ninja-era adaptive delivery would have degraded into.
const COMPAT_VIDEO_WIDTH: i32 = 640;
const COMPAT_VIDEO_HEIGHT: i32 = 360;
/// Compat-profile framerate: 20fps. The TVs decode VP8 in software — fewer,
/// cheaper frames beat 30fps with drops. `videorate(drop-only)` thins the
/// 30fps tee output down to this.
const COMPAT_FRAMERATE: i32 = 15;

/// Primary (720p) encoder bitrate in kbit/s.
const DEFAULT_BITRATE_KBPS: u32 = 2500;
/// Compat (480p VP8) encoder bitrate in bits/s — the weak-device budget
/// (vp8enc's `target-bitrate` is bits/sec, unlike the H264 encoders' kbps).
const COMPAT_TARGET_BITRATE_BPS: i32 = 500_000;

impl NdiPipeline {
    /// Build but do not yet start the pipeline.
    ///
    /// `whep_url` is the axum route path (e.g. `/ndi/whep/<source_id>`) used
    /// as a logical key; the element does NOT bind its own HTTP port.
    pub fn build(ndi_name: &str, whep_url: String) -> Result<Self> {
        crate::init().context("gstreamer init failed")?;
        let encoder_name = crate::hw_h264_encoder().ok_or_else(|| {
            anyhow!(
                "no hardware H264 encoder registered; refusing to build pipeline \
                 (software H264 at 720p30 would melt the N100). \
                 Install Intel VA-API: sudo apt install gstreamer1.0-vaapi intel-media-va-driver-non-free \
                 OR NVIDIA NVENC: sudo apt install gstreamer1.0-plugins-bad with nvcodec support"
            )
        })?;

        let pipeline = gst::Pipeline::new();

        let (ndisrc, ndisrcdemux) = build_ndi_source(ndi_name)?;
        let (videoconvert, videoscale, scale_caps, audio_fakesink) = build_video_chain()?;
        let (raw_tee, q_default, q_compat) = build_raw_tee_and_queues()?;
        let encoder = build_encoder(encoder_name)?;
        let profile_caps = build_profile_caps()?;
        let (h264parse, appsink) = build_parse_and_sink()?;

        pipeline
            .add_many([
                &ndisrc,
                &ndisrcdemux,
                &videoconvert,
                &videoscale,
                &scale_caps,
                &audio_fakesink,
                &raw_tee,
                &q_default,
                &encoder,
                &profile_caps,
                &h264parse,
                appsink.upcast_ref::<gst::Element>(),
            ])
            .context("add elements")?;

        ndisrc.link(&ndisrcdemux).context("link ndisrc -> demux")?;
        // videoconvert → videoscale → capsfilter(NV12, ≤720p) → tee. Both
        // encode branches are linked HERE, before any state change — a tee
        // branch linked after PLAYING never forwards a buffer (sticky events).
        gst::Element::link_many([&videoconvert, &videoscale, &scale_caps, &raw_tee])
            .context("link videoconvert -> videoscale -> caps -> tee")?;
        // Branch A (default profile, 720p H264):
        gst::Element::link_many([&raw_tee, &q_default, &encoder, &profile_caps, &h264parse])
            .context("link tee -> q_default -> encoder -> profile_caps -> h264parse")?;
        h264parse
            .link(appsink.upcast_ref::<gst::Element>())
            .context("link h264parse -> appsink")?;
        // Branch B (compat profile, realtime VP8 854×480@20 — see
        // `add_compat_branch`): added and linked before any state change too.
        let appsink_compat = add_compat_branch(&pipeline, &raw_tee, &q_compat)?;

        // Wrap each appsink in a StreamProducer — the battle-tested fanout
        // from gstreamer-utils that webrtcsink itself uses: forwards full
        // SAMPLES (caps + segment + PTS preserved on the shared clock/base-
        // time), propagates producer latency to consumer appsrcs, gates new
        // consumers on a keyframe, and forwards browser PLIs upstream to the
        // branch's encoder. sync=false: forward every encoded frame
        // IMMEDIATELY — the default (sync=true) holds each frame to its clock
        // deadline (~40ms measured), correct for a rendering sink, wrong for
        // a relay.
        let settings = gstreamer_utils::streamproducer::ProducerSettings { sync: false };
        let producer = StreamProducer::with(&appsink, settings.clone());
        let producer_compat = StreamProducer::with(&appsink_compat, settings);

        connect_demux_pads(&ndisrcdemux, &videoconvert, &audio_fakesink);

        tracing::info!(
            encoder = encoder_name,
            %ndi_name,
            "pipeline built (720p H264 default + 854x480@20 realtime-VP8 compat, per-consumer-pipeline fanout)"
        );

        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);

        Ok(Self {
            pipeline,
            whep_url,
            state_tx,
            state_rx,
            bus_watch: std::sync::Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            producer,
            producer_compat,
        })
    }
}

/// Wire ndisrcdemux's sometimes-pads: video → videoconvert, audio → fakesink.
fn connect_demux_pads(
    ndisrcdemux: &gst::Element,
    videoconvert: &gst::Element,
    audio_fakesink: &gst::Element,
) {
    let videoconvert = videoconvert.clone();
    let audio_fakesink = audio_fakesink.clone();
    ndisrcdemux.connect_pad_added(move |_, pad| {
        let name = pad.name();
        if name == "video" {
            if let Some(sink_pad) = videoconvert.static_pad("sink") {
                let _ = pad.link(&sink_pad);
            }
        } else if name == "audio" {
            if let Some(sink_pad) = audio_fakesink.static_pad("sink") {
                let _ = pad.link(&sink_pad);
            }
        }
    });
}

/// Build the NDI ingest pair: `ndisrc` (named "ndisrc") and `ndisrcdemux`
/// (named "demux").
///
/// timestamp-mode=receive-time: PTS purely from this server's clock at
/// frame arrival. The default ("auto") follows the NDI sender's
/// (Resolume's) timecode with windowed drift correction — accumulated
/// sender-clock drift is then corrected via DISCONT, which the browser
/// sees as "latency builds up, then the picture jumps". Receive-time
/// has ZERO sender-clock coupling; arrival jitter (10-60ms measured)
/// is absorbed by the browser's jitter buffer.
fn build_ndi_source(ndi_name: &str) -> Result<(gst::Element, gst::Element)> {
    let ndisrc = gst::ElementFactory::make("ndisrc")
        .name("ndisrc")
        .property("ndi-name", ndi_name)
        .property_from_str("timestamp-mode", "receive-time")
        .build()
        .context("build ndisrc")?;
    let ndisrcdemux = gst::ElementFactory::make("ndisrcdemux")
        .name("demux")
        .build()
        .context("build ndisrcdemux")?;
    Ok((ndisrc, ndisrcdemux))
}

/// Build the default branch's tail: `h264parse` and the bounded appsink that
/// the branch's StreamProducer wraps.
///
/// h264parse: parses the encoder's H264 elementary stream into AU-aligned
/// frames so every PER-CONSUMER rtph264pay (in its own pipeline) receives a
/// clean, properly-capped stream. `config-interval=-1` re-inserts SPS/PPS
/// before every IDR so a consumer that joins mid-stream gets an IDR
/// immediately via `consumers::request_keyframe` (GOP itself is 240 frames).
///
/// The PAYLOADER is intentionally NOT here — it is per-consumer, in the
/// consumer's own pipeline downstream of an appsrc, so each webrtcbin
/// negotiates its own dynamic RTP payload type with its browser. A single
/// shared payloader emits ONE pt and silently fails (connected, no frames)
/// for any browser that negotiates a different one — the #336 regression.
/// The ENCODERS stay shared (one per PROFILE, never per consumer),
/// preserving the fanout goal.
///
/// appsink: the branch ends in an appsink wrapped by StreamProducer
/// (the same fanout webrtcsink uses). The caps filter pins the bridge
/// format to byte-stream/AU H264 so every consumer appsrc is created
/// with caps that ALWAYS match what the producer forwards.
/// `max-buffers`+`drop` bound the appsink so a momentarily-slow fanout
/// can never back-pressure (and stall) the shared encoder. 5 frames
/// (~170ms) — a bigger backlog would replay stale frames late after a
/// transient stall (latency spike); drop(true) keeps the newest.
fn build_parse_and_sink() -> Result<(gst::Element, gst_app::AppSink)> {
    let h264parse = gst::ElementFactory::make("h264parse")
        .name("h264parse")
        .property("config-interval", -1i32)
        .build()
        .context("build h264parse")?;
    let appsink = gst_app::AppSink::builder()
        .name("enc_appsink")
        .caps(&consumer_h264_caps())
        .max_buffers(5)
        .drop(true)
        .build();
    Ok((h264parse, appsink))
}

/// The H264 caps used on BOTH sides of each StreamProducer bridge: the encoder
/// appsink's caps filter AND every consumer appsrc's initial caps. Pinning
/// byte-stream/AU on both sides guarantees they always match (h264parse
/// converts as needed; with config-interval=-1 the stream carries inline
/// SPS/PPS, so a consumer can start parsing at any IDR).
pub(super) fn consumer_h264_caps() -> gst::Caps {
    gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build()
}

/// The VP8 caps used on BOTH sides of the compat StreamProducer bridge: the
/// vp8enc appsink's caps filter AND every compat consumer appsrc's initial
/// caps — the same always-match guarantee `consumer_h264_caps` gives the
/// H264 bridge (vp8enc output needs no parser; rtpvp8pay takes it directly).
pub(super) fn consumer_vp8_caps() -> gst::Caps {
    gst::Caps::builder("video/x-vp8").build()
}

/// Build the raw fanout point placed after `scale_caps`: a `tee` named
/// "raw_tee" plus one branch-isolation queue per encode branch. A tee blocks
/// ALL branches whenever ANY branch's downstream blocks, so each branch gets
/// a small LEAKY queue: a transient stall in one encoder can then never
/// stall the other encoder (or back-pressure the live NDI source) — it just
/// drops that branch's oldest raw frame, the correct realtime behavior.
fn build_raw_tee_and_queues() -> Result<(gst::Element, gst::Element, gst::Element)> {
    let raw_tee = gst::ElementFactory::make("tee")
        .name("raw_tee")
        .build()
        .context("build raw_tee")?;
    Ok((
        raw_tee,
        build_branch_queue("q_default")?,
        build_branch_queue("q_compat")?,
    ))
}

/// One branch-isolation queue (see `build_raw_tee_and_queues`): bounded to 5
/// raw frames (~165ms @30fps), byte/time limits disabled, leaky=downstream
/// (drop OLDEST on overflow — a realtime branch must never replay a backlog).
fn build_branch_queue(name: &str) -> Result<gst::Element> {
    gst::ElementFactory::make("queue")
        .name(name)
        .property("max-size-buffers", 5u32)
        .property("max-size-bytes", 0u32)
        .property("max-size-time", 0u64)
        .property_from_str("leaky", "downstream")
        .build()
        .with_context(|| format!("build queue {name}"))
}

/// Append the compat encode branch (realtime VP8, VDO.Ninja-style) to the
/// encoder pipeline and link it off `raw_tee`:
///
/// `raw_tee → q_compat → videorate(drop-only) → videoconvert(NV12→I420) →
/// videoscale(add-borders) → compat_scale_caps(I420 854×480 PAR 1/1 @20/1)
/// → vp8enc("encoder_compat") → enc_appsink_compat(video/x-vp8)`.
///
/// - videorate `drop-only=true`: thins the 30fps tee output to the 20fps the
///   caps pin by DROPPING frames — never duplicates to upconvert a slower
///   source (a realtime branch must not pad with copies).
/// - videoconvert: REQUIRED — vp8enc's sink accepts ONLY I420 (gst-inspect
///   verified) while the tee carries NV12 for the H264 hw encoder; this is a
///   cheap 4:2:0→4:2:0 chroma repack.
/// - videoscale `add-borders=true`: 720p 16:9 → 854×480 is aspect-preserving
///   (no borders in practice); non-16:9 sources letterbox instead of stretch.
/// - appsink: same bounded relay contract as the default branch — 5-frame
///   backlog, drop(true) keeps the newest, VP8 bridge caps
///   (`consumer_vp8_caps`) so every compat consumer appsrc always matches.
///
/// Returns the branch's appsink for its StreamProducer wrap. Linked at build
/// time, before any state change (a tee branch linked after PLAYING never
/// forwards a buffer — sticky events).
fn add_compat_branch(
    pipeline: &gst::Pipeline,
    raw_tee: &gst::Element,
    q_compat: &gst::Element,
) -> Result<gst_app::AppSink> {
    let videorate = gst::ElementFactory::make("videorate")
        .property("drop-only", true)
        .build()
        .context("build compat videorate")?;
    let videoconvert = gst::ElementFactory::make("videoconvert")
        .build()
        .context("build compat videoconvert")?;
    let videoscale = gst::ElementFactory::make("videoscale")
        .property("add-borders", true)
        .build()
        .context("build compat videoscale")?;
    let scale_caps = gst::ElementFactory::make("capsfilter")
        .name("compat_scale_caps")
        .property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("format", "I420")
                .field("width", COMPAT_VIDEO_WIDTH)
                .field("height", COMPAT_VIDEO_HEIGHT)
                .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
                .field("framerate", gst::Fraction::new(COMPAT_FRAMERATE, 1))
                .build(),
        )
        .build()
        .context("build compat scale capsfilter")?;
    let encoder_compat = build_compat_vp8_encoder()?;
    let appsink = gst_app::AppSink::builder()
        .name("enc_appsink_compat")
        .caps(&consumer_vp8_caps())
        .max_buffers(5)
        .drop(true)
        .build();
    pipeline
        .add_many([
            q_compat,
            &videorate,
            &videoconvert,
            &videoscale,
            &scale_caps,
            &encoder_compat,
            appsink.upcast_ref::<gst::Element>(),
        ])
        .context("add compat branch elements")?;
    gst::Element::link_many([
        raw_tee,
        q_compat,
        &videorate,
        &videoconvert,
        &videoscale,
        &scale_caps,
        &encoder_compat,
    ])
    .context("link tee -> q_compat -> rate -> convert -> scale -> caps -> vp8enc")?;
    encoder_compat
        .link(appsink.upcast_ref::<gst::Element>())
        .context("link vp8enc -> compat appsink")?;
    Ok(appsink)
}

/// Build the compat branch's `vp8enc` ("encoder_compat") with libwebrtc-
/// parity realtime tuning. VDO.Ninja (browser-to-browser libwebrtc VP8) has
/// played smoothly for YEARS on the same weak Vestel TVs that freeze on a
/// default-tuned vp8enc stream — the previous VP8 attempt (deadline=1
/// cpu-used=8 alone) hit ~26fps with freezes because gst vp8enc's default
/// emits a SINGLE token partition, forcing the TV to decode tokens on ONE of
/// its 4 cores. Property types verified via gst-inspect-1.0 and locked by
/// `compat_branch_is_realtime_vp8`:
///
/// - `deadline=1` (µs/frame, Integer64): libvpx realtime mode.
/// - `cpu-used=8`: fastest realtime encode preset (quality ↓, speed ↑).
/// - `end-usage=cbr` + `target-bitrate=900_000` (bits/sec): constant-bitrate
///   like libwebrtc's rate controller — no VBR bursts to choke the TV.
/// - `keyframe-max-dist=240`: GOP parity with the H264 branch; joins are
///   served by `request_keyframe`, not scheduled keyframe pulses.
/// - `token-partitions="4"` (enum nick for FOUR partitions): THE key delta —
///   partitioned token coding lets the TV's libvpx decoder spread entropy
///   decode across its 4 cores, exactly what libwebrtc emits.
/// - `threads=4`: one encode thread per partition on the server.
/// - `error-resilient=default` (flags): frames stay decodable after loss,
///   libwebrtc parity for realtime streams.
/// - `lag-in-frames=0`: zero lookahead — no encoder-side frame delay.
fn build_compat_vp8_encoder() -> Result<gst::Element> {
    gst::ElementFactory::make("vp8enc")
        .name("encoder_compat")
        .property("deadline", 1i64)
        .property("cpu-used", 8i32)
        .property_from_str("end-usage", "cbr")
        .property("target-bitrate", COMPAT_TARGET_BITRATE_BPS)
        .property("keyframe-max-dist", 240i32)
        .property_from_str("token-partitions", "4")
        .property("threads", 4i32)
        .property_from_str("error-resilient", "default")
        .property("lag-in-frames", 0i32)
        .build()
        .context("build vp8enc encoder_compat")
}

/// Build the raw-video conditioning chain placed between the demux and the
/// encoder: `videoconvert → videoscale → capsfilter(NV12, ≤720p)` plus the
/// audio `fakesink`. Returns `(videoconvert, videoscale, scale_caps, audio_fakesink)`.
fn build_video_chain() -> Result<(gst::Element, gst::Element, gst::Element, gst::Element)> {
    let videoconvert = gst::ElementFactory::make("videoconvert")
        .build()
        .context("build videoconvert")?;
    // Downscale to a stage-display-safe resolution BEFORE encoding. NDI sources
    // are commonly 1080p, 1440p, even 4K (Resolume SP-live here is 2560×1440).
    // Encoding at the source resolution (a) exceeds the H264 level the browser
    // negotiates → the browser decodes ZERO frames → black stage, and (b) is
    // unplayable on the low-cost TVs used as stage displays. Cap at 720p (16:9,
    // universally decodable, matches the 2.5 Mbps target). `add-borders`
    // letterboxes non-16:9 sources instead of stretching; downstream of a
    // smaller source it upscales harmlessly.
    let videoscale = gst::ElementFactory::make("videoscale")
        .property("add-borders", true)
        .build()
        .context("build videoscale")?;
    let scale_caps = gst::ElementFactory::make("capsfilter")
        .property(
            "caps",
            gst::Caps::builder("video/x-raw")
                // format=NV12 (4:2:0) is REQUIRED, not cosmetic: NDI sources are
                // often 4:2:2 (UYVY, like Resolume here) or 4:4:4, and if the
                // encoder input keeps that chroma, nvh264enc emits a High-4:2:2 /
                // High-4:4:4 H264 profile that NO browser can decode (ontrack
                // fires, framesDecoded stays 0 → black). Web browsers only decode
                // 4:2:0. Forcing NV12 here makes the encoder emit a Main/High
                // 4:2:0 stream every browser decodes.
                .field("format", "NV12")
                .field("width", MAX_VIDEO_WIDTH)
                .field("height", MAX_VIDEO_HEIGHT)
                .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
                .build(),
        )
        .build()
        .context("build scale capsfilter")?;
    let audio_fakesink = gst::ElementFactory::make("fakesink")
        .property("async", false)
        .property("sync", false)
        .build()
        .context("build fakesink (audio)")?;
    Ok((videoconvert, videoscale, scale_caps, audio_fakesink))
}

/// Build the default branch's H264 encoder ("encoder") with tuning applied
/// at construction time. `encoder_name` is one of the three returned by
/// `hw_h264_encoder()` (vah264enc / nvh264enc / x264enc).
fn build_encoder(encoder_name: &str) -> Result<gst::Element> {
    let mut encoder_builder = gst::ElementFactory::make(encoder_name).name("encoder");
    match encoder_name {
        "vah264enc" => {
            // key-int-max=240 (8s GOP): no 1s IDR pulses — large keyframes
            // made low-end TVs choppy and inflated their jitter buffers.
            // Consumer joins get an immediate IDR via force-keyunit (see
            // consumers::request_keyframe); loss recovery stays PLI-driven.
            // target-usage=6: faster encode on the prod N100 (default 4).
            encoder_builder = encoder_builder
                .property("key-int-max", 240u32)
                .property("target-usage", 6u32)
                .property("bitrate", DEFAULT_BITRATE_KBPS);
        }
        "nvh264enc" => {
            encoder_builder = encoder_builder
                .property("gop-size", 240i32)
                .property("zerolatency", true)
                .property("bitrate", DEFAULT_BITRATE_KBPS);
        }
        "x264enc" => {
            encoder_builder = encoder_builder
                .property_from_str("tune", "zerolatency")
                .property_from_str("speed-preset", "superfast")
                .property("key-int-max", 240u32)
                .property("bitrate", DEFAULT_BITRATE_KBPS);
        }
        _ => {
            // hw_h264_encoder only returns the three above; defensive fallthrough.
        }
    }
    encoder_builder
        .build()
        .with_context(|| format!("build encoder ({encoder_name})"))
}

/// Pin the default encoder's H264 output to constrained-baseline (capsfilter
/// "profile_caps"). The encoders default to High profile, which strict TV HW
/// decoders (the Vestel stage displays) reject for WebRTC — Chromium then
/// swaps in NullVideoDecoder and the stage shows black while RTP keeps
/// flowing (live prod finding, 2026-06-11). Constrained-baseline is WebRTC's
/// universally-decodable profile; the ~10-15% bitrate-efficiency loss is
/// invisible, the compatibility is mandatory.
fn build_profile_caps() -> Result<gst::Element> {
    gst::ElementFactory::make("capsfilter")
        .name("profile_caps")
        .property(
            "caps",
            gst::Caps::builder("video/x-h264")
                .field("profile", "constrained-baseline")
                .build(),
        )
        .build()
        .context("build profile capsfilter")
}
