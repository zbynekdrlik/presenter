//! Pipeline construction: build the shared-encoder ENCODER pipeline
//! (`ndisrc → ndisrcdemux → videoconvert → videoscale → caps → raw_tee`,
//! fanning into TWO parallel encode branches:
//! H264 `→ q_h264 → encoder → profile_caps → h264parse → enc_appsink` and
//! VP8 `→ q_vp8 → videoconvert → videoscale → caps(854×480) → vp8enc →
//! enc_appsink_vp8`). Each appsink is
//! wrapped by a `gstreamer_utils::StreamProducer` which fans the encoded
//! stream out to the per-consumer `appsrc → rtph264pay|rtpvp8pay → webrtcbin`
//! pipelines built in `add_consumer` (see `consumers.rs`). The VP8 branch
//! exists because the weak stage TVs' H264 OMX decoder is vendor-broken
//! (spec addendum 2) — those clients re-offer VP8-only and are served from
//! the parallel VP8 producer.

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
        let (raw_tee, q_h264, q_vp8) = build_raw_tee_and_queues()?;
        let encoder = build_encoder(encoder_name)?;
        let profile_caps = build_profile_caps()?;
        let (h264parse, appsink) = build_parse_and_sink()?;
        let (vp8_convert, vp8_scale, vp8_scale_caps, vp8enc, vp8_appsink) = build_vp8_branch()?;

        pipeline
            .add_many([
                &ndisrc,
                &ndisrcdemux,
                &videoconvert,
                &videoscale,
                &scale_caps,
                &audio_fakesink,
                &raw_tee,
                &q_h264,
                &encoder,
                &profile_caps,
                &h264parse,
                appsink.upcast_ref::<gst::Element>(),
                &q_vp8,
                &vp8_convert,
                &vp8_scale,
                &vp8_scale_caps,
                &vp8enc,
                vp8_appsink.upcast_ref::<gst::Element>(),
            ])
            .context("add elements")?;

        ndisrc.link(&ndisrcdemux).context("link ndisrc -> demux")?;
        // videoconvert → videoscale → capsfilter(NV12, ≤720p) → tee. Both
        // encode branches are linked HERE, before any state change — a tee
        // branch linked after PLAYING never forwards a buffer (sticky events).
        gst::Element::link_many([&videoconvert, &videoscale, &scale_caps, &raw_tee])
            .context("link videoconvert -> videoscale -> caps -> tee")?;
        // Branch A (H264, elements unchanged from the pre-tee topology):
        gst::Element::link_many([&raw_tee, &q_h264, &encoder, &profile_caps, &h264parse])
            .context("link tee -> q_h264 -> encoder -> profile_caps -> h264parse")?;
        h264parse
            .link(appsink.upcast_ref::<gst::Element>())
            .context("link h264parse -> appsink")?;
        // Branch B (VP8 compat profile): NV12→I420 convert feeds the 480p
        // downscale (854×480, weak-device budget) and then vp8enc (I420-only).
        gst::Element::link_many([
            &raw_tee,
            &q_vp8,
            &vp8_convert,
            &vp8_scale,
            &vp8_scale_caps,
            &vp8enc,
        ])
        .context("link tee -> q_vp8 -> videoconvert -> videoscale -> vp8_scale_caps -> vp8enc")?;
        vp8enc
            .link(vp8_appsink.upcast_ref::<gst::Element>())
            .context("link vp8enc -> vp8 appsink")?;

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
        let producer_vp8 = StreamProducer::with(&vp8_appsink, settings);

        connect_demux_pads(&ndisrcdemux, &videoconvert, &audio_fakesink);

        tracing::info!(
            encoder = encoder_name,
            %ndi_name,
            "pipeline built (shared-encoder H264 + parallel VP8 fallback, per-consumer-pipeline fanout)"
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
            producer_vp8,
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

/// Build the encoder pipeline's tail: `h264parse` (named "h264parse") and the
/// bounded appsink (named "enc_appsink") that the StreamProducer wraps.
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
/// The ENCODER stays shared (one nvh264enc), preserving the fanout goal.
///
/// appsink: the encoder pipeline ends in an appsink wrapped by StreamProducer
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

/// The H264 caps used on BOTH sides of the StreamProducer bridge: the encoder
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

/// The VP8 caps used on BOTH sides of the VP8 StreamProducer bridge: the VP8
/// encoder appsink's caps filter AND every VP8 consumer appsrc's initial
/// caps — the same always-match guarantee `consumer_h264_caps` gives the
/// H264 bridge.
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
        build_branch_queue("q_h264")?,
        build_branch_queue("q_vp8")?,
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

/// VP8 compat-profile resolution. The VP8 branch serves WEAK devices that
/// both (a) force this server to encode in SOFTWARE (vp8enc) and (b) decode
/// in software themselves (libvpx on broken-OMX Vestel TVs). Measured on
/// prod 2026-06-12 at 720p30: presenter-server hit 175% CPU / load 11.4 on
/// the 4-core N100 (periodic hiccups for ALL consumers, healthy H264 ones
/// included), while the Vestel TVs decoded real motion content at 0.3-1.7
/// fps with freezes. 854×480 (16:9 480p) is cheap on both ends; healthy
/// clients stay on the unchanged H264 720p branch. 854 is not mod-16, but
/// libvpx only requires even dimensions — verified end-to-end by the local
/// synthetic e2e (VP8 consumer decodes with frameWidth=854).
const VP8_VIDEO_WIDTH: i32 = 854;
const VP8_VIDEO_HEIGHT: i32 = 480;

/// Build the parallel VP8 fallback branch tail: `videoconvert → videoscale →
/// capsfilter("vp8_scale_caps") → vp8enc → appsink("enc_appsink_vp8")`.
/// Exists because the weak stage TVs' H264 OMX decoder is vendor-broken
/// (`OMX.MS.AVC.Decoder` port-reconfig failure → ~1 displayed frame per GOP,
/// spec addendum 2); those clients re-offer VP8-only and decode it in
/// software (libvpx), bypassing the broken OMX.
///
/// The `videoconvert` is REQUIRED: vp8enc's sink template accepts ONLY
/// `video/x-raw,format=I420` (verified via gst-inspect-1.0 1.24), while
/// `scale_caps` pins NV12 for the H264 hardware encoders — the convert is a
/// cheap NV12→I420 repack (both 4:2:0, chroma reshuffle only).
///
/// The `videoscale → vp8_scale_caps` pair downscales 720p → 854×480: this
/// branch is the COMPAT profile for weak devices, so it must be cheap to
/// encode (software, on the N100) AND cheap to decode (software, on the
/// TVs) — see `VP8_VIDEO_WIDTH`. `add-borders` letterboxes like the main
/// chain's videoscale.
///
/// vp8enc tuning (types verified via gst-inspect-1.0): `deadline=1` µs/frame
/// = libvpx realtime mode; `cpu-used=8` trades quality for encode speed;
/// `keyframe-max-dist=240` matches the H264 GOP (8s) — joins are served by
/// `request_keyframe`, not scheduled keyframe pulses; `target-bitrate` is in
/// bits/sec (1 Mbps — the 480p compat budget; 2 Mbps belonged to the
/// abandoned 720p VP8 profile).
fn build_vp8_branch() -> Result<(
    gst::Element,
    gst::Element,
    gst::Element,
    gst::Element,
    gst_app::AppSink,
)> {
    let vp8_convert = gst::ElementFactory::make("videoconvert")
        .build()
        .context("build vp8 videoconvert")?;
    let vp8_scale = gst::ElementFactory::make("videoscale")
        .property("add-borders", true)
        .build()
        .context("build vp8 videoscale")?;
    let vp8_scale_caps = gst::ElementFactory::make("capsfilter")
        .name("vp8_scale_caps")
        .property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("format", "I420")
                .field("width", VP8_VIDEO_WIDTH)
                .field("height", VP8_VIDEO_HEIGHT)
                .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
                .build(),
        )
        .build()
        .context("build vp8 scale capsfilter")?;
    let vp8enc = gst::ElementFactory::make("vp8enc")
        .name("encoder_vp8")
        .property("deadline", 1i64)
        .property("cpu-used", 8i32)
        .property("keyframe-max-dist", 240i32)
        .property("target-bitrate", 1_000_000i32)
        .build()
        .context("build vp8enc")?;
    // Same bounded relay appsink as the H264 branch: 5-frame backlog,
    // drop(true) keeps the newest; StreamProducer wraps it with sync=false.
    let vp8_appsink = gst_app::AppSink::builder()
        .name("enc_appsink_vp8")
        .caps(&consumer_vp8_caps())
        .max_buffers(5)
        .drop(true)
        .build();
    Ok((vp8_convert, vp8_scale, vp8_scale_caps, vp8enc, vp8_appsink))
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

/// Build the H264 encoder with tuning applied at construction time. `encoder_name`
/// is one of the three returned by `hw_h264_encoder()` (vah264enc / nvh264enc /
/// x264enc); the element is named "encoder" for later lookup.
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
                .property("bitrate", 2500u32);
        }
        "nvh264enc" => {
            encoder_builder = encoder_builder
                .property("gop-size", 240i32)
                .property("zerolatency", true)
                .property("bitrate", 2500u32);
        }
        "x264enc" => {
            encoder_builder = encoder_builder
                .property_from_str("tune", "zerolatency")
                .property_from_str("speed-preset", "superfast")
                .property("key-int-max", 240u32)
                .property("bitrate", 2500u32);
        }
        _ => {
            // hw_h264_encoder only returns the three above; defensive fallthrough.
        }
    }
    encoder_builder.build().context("build encoder")
}

/// Pin the encoder's H264 output to constrained-baseline. The encoders
/// default to High profile, which strict TV HW decoders (the Vestel stage
/// displays) reject for WebRTC — Chromium then swaps in NullVideoDecoder and
/// the stage shows black while RTP keeps flowing (live prod finding,
/// 2026-06-11). Constrained-baseline is WebRTC's universally-decodable
/// profile; the ~10-15% bitrate-efficiency loss at 2.5 Mbps 720p30 is
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
