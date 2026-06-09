//! Pipeline construction: build the shared-encoder fanout topology
//! (`ndisrc → ndisrcdemux → videoconvert → vah264enc → h264parse → tee`).
//! The per-consumer `rtph264pay → webrtcbin` branches are added off the tee
//! in `add_consumer` (see `consumers.rs`).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
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

        let ndisrc = gst::ElementFactory::make("ndisrc")
            .property("ndi-name", ndi_name)
            .build()
            .context("build ndisrc")?;
        let ndisrcdemux = gst::ElementFactory::make("ndisrcdemux")
            .name("demux")
            .build()
            .context("build ndisrcdemux")?;
        let videoconvert = gst::ElementFactory::make("videoconvert")
            .build()
            .context("build videoconvert")?;
        // Downscale to a stage-display-safe resolution BEFORE encoding. NDI
        // sources are commonly 1080p, 1440p, even 4K (Resolume SP-live here is
        // 2560×1440). Encoding at the source resolution (a) exceeds the H264
        // level the browser negotiates → the browser decodes ZERO frames →
        // black stage, and (b) is unplayable on the low-cost TVs used as stage
        // displays. Cap at 720p (16:9, universally decodable, matches the 2.5
        // Mbps target). `add-borders` letterboxes non-16:9 sources instead of
        // stretching; downstream of a smaller source it upscales harmlessly.
        let videoscale = gst::ElementFactory::make("videoscale")
            .property("add-borders", true)
            .build()
            .context("build videoscale")?;
        let scale_caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("video/x-raw")
                    // format=NV12 (4:2:0) is REQUIRED, not cosmetic: NDI sources
                    // are often 4:2:2 (UYVY, like Resolume here) or 4:4:4, and if
                    // the encoder input keeps that chroma, nvh264enc emits a
                    // High-4:2:2 / High-4:4:4 H264 profile that NO browser can
                    // decode (ontrack fires, framesDecoded stays 0 → black). Web
                    // browsers only decode 4:2:0. Forcing NV12 here makes the
                    // encoder emit a Main/High 4:2:0 stream every browser decodes.
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

        // Build the encoder with tuning applied at construction time.
        // (No encoder-setup signal — that was a webrtcsink concept. Here
        // we own the encoder element directly so we set properties now.)
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
            _ => {
                // hw_h264_encoder only returns the three above; defensive fallthrough.
            }
        }
        let encoder = encoder_builder.build().context("build encoder")?;

        // Parse the encoder's H264 elementary stream into AU-aligned frames so
        // every PER-CONSUMER rtph264pay (added in add_consumer) receives a
        // clean, properly-capped stream. `config-interval=-1` re-inserts
        // SPS/PPS before every IDR so a consumer that joins mid-stream decodes
        // at the next keyframe.
        //
        // The PAYLOADER is intentionally NOT here — it is per-consumer,
        // downstream of the tee, so each webrtcbin negotiates its own dynamic
        // RTP payload type with its browser. A single shared payloader can only
        // emit ONE pt and silently fails (connected, no frames) for any browser
        // that negotiates a different one — the #336 regression. The ENCODER
        // stays shared (one nvh264enc), preserving the fanout goal.
        let h264parse = gst::ElementFactory::make("h264parse")
            .name("h264parse")
            .property("config-interval", -1i32)
            .build()
            .context("build h264parse")?;
        let tee = gst::ElementFactory::make("tee")
            .name("tee")
            // Tee starts without any linked src pads; first consumer adds a branch.
            .property("allow-not-linked", true)
            .build()
            .context("build tee")?;

        pipeline
            .add_many([
                &ndisrc,
                &ndisrcdemux,
                &videoconvert,
                &videoscale,
                &scale_caps,
                &audio_fakesink,
                &encoder,
                &h264parse,
                &tee,
            ])
            .context("add elements")?;

        ndisrc.link(&ndisrcdemux).context("link ndisrc -> demux")?;
        // videoconvert → videoscale → capsfilter(≤720p) → encoder
        gst::Element::link_many([&videoconvert, &videoscale, &scale_caps, &encoder])
            .context("link videoconvert -> videoscale -> caps -> encoder")?;
        encoder
            .link(&h264parse)
            .context("link encoder -> h264parse")?;
        h264parse.link(&tee).context("link h264parse -> tee")?;

        // ndisrcdemux is a sometimes-pad element. Wire up dynamic pads:
        let videoconvert_clone = videoconvert.clone();
        let audio_fakesink_clone = audio_fakesink.clone();
        ndisrcdemux.connect_pad_added(move |_, pad| {
            let name = pad.name();
            if name == "video" {
                if let Some(sink_pad) = videoconvert_clone.static_pad("sink") {
                    let _ = pad.link(&sink_pad);
                }
            } else if name == "audio" {
                if let Some(sink_pad) = audio_fakesink_clone.static_pad("sink") {
                    let _ = pad.link(&sink_pad);
                }
            }
        });

        tracing::info!(
            encoder = encoder_name,
            %ndi_name,
            "pipeline built (shared-encoder fanout topology)"
        );

        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);

        Ok(Self {
            pipeline,
            whep_url,
            state_tx,
            state_rx,
            bus_watch: std::sync::Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            tee: Arc::new(tee),
        })
    }
}
