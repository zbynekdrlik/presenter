//! Pipeline construction: build the shared-encoder fanout topology
//! (`ndisrc → ndisrcdemux → videoconvert → vah264enc → rtph264pay → tee`).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use tokio::sync::watch;

use super::{NdiPipeline, PipelineState};

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

        let rtph264pay = gst::ElementFactory::make("rtph264pay")
            .name("rtpay")
            // Resend SPS/PPS with every IDR for fast browser recovery.
            .property("config-interval", -1i32)
            // H264 dynamic payload type 96.
            .property("pt", 96u32)
            .build()
            .context("build rtph264pay")?;
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
                &audio_fakesink,
                &encoder,
                &rtph264pay,
                &tee,
            ])
            .context("add elements")?;

        ndisrc.link(&ndisrcdemux).context("link ndisrc -> demux")?;
        videoconvert
            .link(&encoder)
            .context("link videoconvert -> encoder")?;
        encoder
            .link(&rtph264pay)
            .context("link encoder -> rtph264pay")?;
        rtph264pay.link(&tee).context("link rtph264pay -> tee")?;

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
