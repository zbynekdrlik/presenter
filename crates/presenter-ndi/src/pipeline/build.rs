//! Pipeline construction: build the shared-encoder ENCODER pipeline
//! (`ndisrc → ndisrcdemux → videoconvert → videoscale → caps → raw_tee`,
//! fanning into TWO parallel H264 encode branches — one per stream profile:
//! default `→ q_default → encoder → profile_caps → h264parse → enc_appsink`
//! (1280×720) and compat `→ q_compat → videoscale → compat_scale_caps →
//! encoder_compat → compat_profile_caps → h264parse_compat →
//! enc_appsink_compat` (640×480). Each appsink is wrapped by a
//! `gstreamer_utils::StreamProducer` which fans the encoded stream out to
//! the per-consumer `appsrc → rtph264pay → webrtcbin` pipelines built in
//! `add_consumer` (see `consumers.rs`); a consumer selects its producer via
//! the WHEP `?profile=compat` query (see `StreamProfile`). The compat branch
//! exists because the weak stage TVs' MStar H264 OMX decoder dies on output
//! port reconfiguration — it default-inits at 640×480, so ONLY an exactly-
//! 640×480 stream decodes in hardware there (spec addendum 2 pivot).

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

/// Compat-profile resolution: EXACTLY the MStar OMX decoder's default-init
/// port size. The weak Vestel stage TVs decode the first frame of each GOP,
/// then the OMX output-port reconfiguration (640×480 default → stream size)
/// fails at the vendor-firmware level (`setParameter(ParamPortDefinition)
/// BadParameter`, codec torn down and recreated every GOP — logcat-proven,
/// spec addendum 2). A stream that needs NO reconfig decodes in hardware.
/// 640 and 480 are mod-16 — clean macroblock alignment for every encoder.
const COMPAT_VIDEO_WIDTH: i32 = 640;
const COMPAT_VIDEO_HEIGHT: i32 = 480;

/// Primary (720p) encoder bitrate in kbit/s.
const DEFAULT_BITRATE_KBPS: u32 = 2500;
/// Compat (480p) encoder bitrate in kbit/s — the weak-device budget.
const COMPAT_BITRATE_KBPS: u32 = 900;

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
        let encoder = build_encoder(encoder_name, "encoder", DEFAULT_BITRATE_KBPS)?;
        let profile_caps = build_profile_caps("profile_caps")?;
        let (h264parse, appsink) = build_parse_and_sink("h264parse", "enc_appsink")?;
        let (compat_scale, compat_scale_caps) = build_compat_scale()?;
        let encoder_compat = build_encoder(encoder_name, "encoder_compat", COMPAT_BITRATE_KBPS)?;
        let compat_profile_caps = build_profile_caps("compat_profile_caps")?;
        let (h264parse_compat, appsink_compat) =
            build_parse_and_sink("h264parse_compat", "enc_appsink_compat")?;

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
                &q_compat,
                &compat_scale,
                &compat_scale_caps,
                &encoder_compat,
                &compat_profile_caps,
                &h264parse_compat,
                appsink_compat.upcast_ref::<gst::Element>(),
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
        // Branch B (compat profile, 640×480 H264): the tee output is already
        // NV12 (scale_caps pins it), which every encoder here accepts, so the
        // branch needs NO videoconvert — just the downscale to the OMX-safe
        // 640×480 (see `build_compat_scale`), then the same encode/parse tail
        // shape as branch A.
        gst::Element::link_many([
            &raw_tee,
            &q_compat,
            &compat_scale,
            &compat_scale_caps,
            &encoder_compat,
            &compat_profile_caps,
            &h264parse_compat,
        ])
        .context("link tee -> q_compat -> scale -> caps -> encoder_compat -> caps -> parse")?;
        h264parse_compat
            .link(appsink_compat.upcast_ref::<gst::Element>())
            .context("link h264parse_compat -> compat appsink")?;

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
            "pipeline built (two H264 profile branches — 720p default + 640x480 compat, per-consumer-pipeline fanout)"
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

/// Build one encode branch's tail: `h264parse` and the bounded appsink that
/// the branch's StreamProducer wraps. Both profile branches share this shape;
/// `parse_name`/`sink_name` are "h264parse"/"enc_appsink" for the default
/// branch and "h264parse_compat"/"enc_appsink_compat" for the compat branch.
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
fn build_parse_and_sink(
    parse_name: &str,
    sink_name: &str,
) -> Result<(gst::Element, gst_app::AppSink)> {
    let h264parse = gst::ElementFactory::make("h264parse")
        .name(parse_name)
        .property("config-interval", -1i32)
        .build()
        .with_context(|| format!("build {parse_name}"))?;
    let appsink = gst_app::AppSink::builder()
        .name(sink_name)
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

/// Build the compat branch's downscale pair: `videoscale(add-borders) →
/// capsfilter("compat_scale_caps", NV12 640×480 PAR 1/1)`.
///
/// The resolution MUST be exactly `COMPAT_VIDEO_WIDTH`×`COMPAT_VIDEO_HEIGHT`
/// — the MStar OMX decoder's default-init port size; any other size forces
/// the fatal output-port reconfiguration (see the const's doc). Format stays
/// NV12: the tee output is already NV12 (the main chain's `scale_caps` pins
/// it) and every H264 encoder we use accepts NV12 directly, so unlike the
/// abandoned VP8 branch (I420-only vp8enc) NO videoconvert is needed.
/// `add-borders` letterboxes the 16:9 720p input into 4:3 640×480 instead
/// of stretching, like the main chain's videoscale.
fn build_compat_scale() -> Result<(gst::Element, gst::Element)> {
    let compat_scale = gst::ElementFactory::make("videoscale")
        .property("add-borders", true)
        .build()
        .context("build compat videoscale")?;
    let compat_scale_caps = gst::ElementFactory::make("capsfilter")
        .name("compat_scale_caps")
        .property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("format", "NV12")
                .field("width", COMPAT_VIDEO_WIDTH)
                .field("height", COMPAT_VIDEO_HEIGHT)
                .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
                .build(),
        )
        .build()
        .context("build compat scale capsfilter")?;
    Ok((compat_scale, compat_scale_caps))
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

/// Build one H264 encoder with tuning applied at construction time.
/// `encoder_name` is one of the three returned by `hw_h264_encoder()`
/// (vah264enc / nvh264enc / x264enc); `element_name` is "encoder" for the
/// default 720p branch and "encoder_compat" for the 640×480 compat branch —
/// SAME factory on both, only the bitrate differs (`bitrate_kbps`).
fn build_encoder(
    encoder_name: &str,
    element_name: &str,
    bitrate_kbps: u32,
) -> Result<gst::Element> {
    let mut encoder_builder = gst::ElementFactory::make(encoder_name).name(element_name);
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
                .property("bitrate", bitrate_kbps);
        }
        "nvh264enc" => {
            encoder_builder = encoder_builder
                .property("gop-size", 240i32)
                .property("zerolatency", true)
                .property("bitrate", bitrate_kbps);
        }
        "x264enc" => {
            encoder_builder = encoder_builder
                .property_from_str("tune", "zerolatency")
                .property_from_str("speed-preset", "superfast")
                .property("key-int-max", 240u32)
                .property("bitrate", bitrate_kbps);
        }
        _ => {
            // hw_h264_encoder only returns the three above; defensive fallthrough.
        }
    }
    encoder_builder
        .build()
        .with_context(|| format!("build {element_name} ({encoder_name})"))
}

/// Pin an encoder's H264 output to constrained-baseline (one capsfilter per
/// branch: "profile_caps" / "compat_profile_caps"). The encoders default to
/// High profile, which strict TV HW decoders (the Vestel stage displays)
/// reject for WebRTC — Chromium then swaps in NullVideoDecoder and the stage
/// shows black while RTP keeps flowing (live prod finding, 2026-06-11).
/// Constrained-baseline is WebRTC's universally-decodable profile; the
/// ~10-15% bitrate-efficiency loss is invisible, the compatibility is
/// mandatory.
fn build_profile_caps(name: &str) -> Result<gst::Element> {
    gst::ElementFactory::make("capsfilter")
        .name(name)
        .property(
            "caps",
            gst::Caps::builder("video/x-h264")
                .field("profile", "constrained-baseline")
                .build(),
        )
        .build()
        .with_context(|| format!("build profile capsfilter {name}"))
}
