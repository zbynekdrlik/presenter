//! Pipeline construction: build the shared-encoder ENCODER pipeline
//! (`ndisrc → ndisrcdemux → videoconvert → videoscale → caps → raw_tee →
//! q_default → encoder → profile_caps → h264parse → enc_appsink`,
//! 1280×720 H264). The appsink is wrapped by a
//! `gstreamer_utils::StreamProducer` which fans the encoded stream out to the
//! per-consumer `appsrc → rtph264pay → webrtcbin` pipelines built in
//! `add_consumer` (see `consumers.rs`). ONE shared hardware-H264 encoder
//! serves every consumer.

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

/// Primary (720p) encoder bitrate in kbit/s.
const DEFAULT_BITRATE_KBPS: u32 = 2500;

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

        // Force the steady system monotonic clock for the ENCODER pipeline (and
        // therefore every consumer pipeline, which shares this clock + base-time
        // for its RTP/RTCP timing). Left to auto-select, GStreamer can slave to a
        // clock PROVIDED by ndisrc (the NDI sender's clock) and recalibrate it
        // periodically; a ~20s clock correction shifts the RTP/RTCP timing of
        // EVERY consumer at the SAME instant, which Chrome sees as a synchronized
        // playout resync — a ~400ms render pause every ~20s on ALL TVs at once
        // (the "naraz" hitch). A fixed system clock never recalibrates;
        // receive-time PTS still stamps from this clock at frame arrival.
        let sysclock = gst::SystemClock::obtain();
        pipeline.use_clock(Some(&sysclock));
        tracing::info!("encoder pipeline pinned to system monotonic clock (no NDI clock slaving)");

        let (ndisrc, ndisrcdemux) = build_ndi_source(ndi_name)?;
        let (videoconvert, videoscale, scale_caps, audio_fakesink) = build_video_chain()?;
        let (raw_tee, q_default) = build_raw_tee_and_queues()?;
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
        // videoconvert → videoscale → capsfilter(NV12, ≤720p) → tee. The
        // encode branch is linked HERE, before any state change — a tee
        // branch linked after PLAYING never forwards a buffer (sticky events).
        gst::Element::link_many([&videoconvert, &videoscale, &scale_caps, &raw_tee])
            .context("link videoconvert -> videoscale -> caps -> tee")?;
        // 720p H264 encode branch:
        gst::Element::link_many([&raw_tee, &q_default, &encoder, &profile_caps, &h264parse])
            .context("link tee -> q_default -> encoder -> profile_caps -> h264parse")?;
        h264parse
            .link(appsink.upcast_ref::<gst::Element>())
            .context("link h264parse -> appsink")?;
        // Wrap the appsink in a StreamProducer — the battle-tested fanout
        // from gstreamer-utils that webrtcsink itself uses: forwards full
        // SAMPLES (caps + segment + PTS preserved on the shared clock/base-
        // time), propagates producer latency to consumer appsrcs, gates new
        // consumers on a keyframe, and forwards browser PLIs upstream to the
        // encoder. sync=false: forward every encoded frame IMMEDIATELY — the
        // default (sync=true) holds each frame to its clock deadline (~40ms
        // measured), correct for a rendering sink, wrong for a relay.
        let settings = gstreamer_utils::streamproducer::ProducerSettings { sync: false };
        let producer = StreamProducer::with(&appsink, settings);

        connect_demux_pads(&ndisrcdemux, &videoconvert, &audio_fakesink);

        // #509 (T0): read-only ingest-timing probe on the raw NDI frames
        // (videoconvert's static sink pad, fed by the demux video pad). With
        // ndisrc timestamp-mode=receive-time, buffer PTS = this server's clock
        // at frame arrival, so the probe logs the camera→NDI→server delivery
        // cadence — the signal that localizes the "lags/jumps after a while".
        if let Some(sink) = videoconvert.static_pad("sink") {
            super::ingest_timing::install_probe(&sink, ndi_name);
        }

        tracing::info!(
            encoder = encoder_name,
            %ndi_name,
            "pipeline built (720p H264 shared encoder, per-consumer-pipeline fanout)"
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
/// immediately via `consumers::request_keyframe` (GOP itself is 60 frames).
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

/// Build the raw fanout point placed after `scale_caps`: a `tee` named
/// "raw_tee" plus the encode branch's isolation queue. A tee blocks ALL
/// branches whenever ANY branch's downstream blocks, so the branch gets a
/// small LEAKY queue: a transient encoder stall can then never back-pressure
/// the live NDI source — it just drops the branch's oldest raw frame, the
/// correct realtime behavior. (The `tee` is retained as the StreamProducer's
/// fanout is per-consumer downstream of the appsink, not per encode branch.)
fn build_raw_tee_and_queues() -> Result<(gst::Element, gst::Element)> {
    let raw_tee = gst::ElementFactory::make("tee")
        .name("raw_tee")
        .build()
        .context("build raw_tee")?;
    Ok((raw_tee, build_branch_queue("q_default")?))
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
            // key-int-max=60 (2s GOP @30fps): no 1s IDR pulses — large
            // keyframes made low-end TVs choppy and inflated their jitter
            // buffers. Consumer joins get an immediate IDR via force-keyunit
            // (see consumers::request_keyframe); loss recovery stays PLI-driven.
            // target-usage=6: faster encode on the prod N100 (default 4).
            encoder_builder = encoder_builder
                .property("key-int-max", 60u32)
                .property("target-usage", 6u32)
                .property("bitrate", DEFAULT_BITRATE_KBPS);
        }
        "nvh264enc" => {
            encoder_builder = encoder_builder
                .property("gop-size", 60i32)
                .property("zerolatency", true)
                .property("bitrate", DEFAULT_BITRATE_KBPS);
        }
        "x264enc" => {
            encoder_builder = encoder_builder
                .property_from_str("tune", "zerolatency")
                .property_from_str("speed-preset", "superfast")
                .property("key-int-max", 60u32)
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
