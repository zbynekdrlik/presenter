//! Per-source GStreamer pipeline owning ndisrc + shared encoder + fanout tee.
//!
//! Each `NdiPipeline` instance corresponds to ONE active NDI source. The
//! pipeline builds a shared-encoder topology:
//!
//! ```text
//! ndisrc → ndisrcdemux → videoconvert → vah264enc → rtph264pay → tee
//!                audio ↘ fakesink     (one encoder)              |
//!                                                    ┌───────────┘
//!                                                    ├─ src_0 → queue → webrtcbin (consumer #1)
//!                                                    ├─ src_1 → queue → webrtcbin (consumer #2)
//!                                                    └─ src_N → queue → webrtcbin (consumer #N)
//! ```
//!
//! Per-consumer state lives in `WhepSession` (`whep_session.rs`). The pipeline
//! owns the shared encoder + tee and a `tokio::sync::Mutex<HashMap<String,
//! WhepSession>>` of active sessions.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_webrtc as gst_webrtc;
use tokio::sync::watch;

use crate::whep_session::{IceCandidate, WhepConnectionState, WhepSession};

/// Pipeline lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineState {
    /// Built but not yet PLAYING (waiting for ASYNC_DONE).
    Starting,
    /// PLAYING — WHEP endpoint is live and accepting subscribers.
    Streaming,
    /// Tearing down or torn down.
    Stopped,
    /// Error state — pipeline failed and must be recreated.
    Errored(String),
}

/// Answer returned by `add_consumer` to the HTTP WHEP shim.
pub struct WhepAnswer {
    pub session_id: String,
    pub sdp_answer: String,
    pub initial_candidates: Vec<IceCandidate>,
}

/// Snapshot of the pipeline state for the diagnostic route (Task 8 fills
/// `source_id`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSnapshot {
    pub source_id: String,
    pub state: String,
    pub encoder_factory: Option<String>,
    pub encoder_count: usize,
    pub consumer_count: usize,
    pub sessions: Vec<SessionSnapshot>,
}

/// Per-consumer snapshot entry.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub id: String,
    pub connection_state: WhepConnectionState,
}

/// Owns one GStreamer pipeline for one NDI source.
pub struct NdiPipeline {
    /// Underlying GStreamer pipeline.
    pipeline: gst::Pipeline,
    /// WHEP URL that subscribers (browsers) POST to.
    whep_url: String,
    /// State observer for the manager / WS event emitter.
    state_tx: watch::Sender<PipelineState>,
    state_rx: watch::Receiver<PipelineState>,
    /// Bus watch task handle so we can cancel on Drop.
    bus_watch: Option<tokio::task::JoinHandle<()>>,
    /// Active per-consumer sessions.
    sessions: Arc<tokio::sync::Mutex<HashMap<String, WhepSession>>>,
    /// Tee element — `add_consumer` / `remove_consumer` request/release pads.
    tee: Arc<gst::Element>,
}

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
    /// state-channel pinned at `Stopped`, no whep_url, no bus_watch. Its
    /// public surface (`state()`, `stop()`, drop) behaves identically to a
    /// real-but-never-started pipeline.
    #[cfg(test)]
    pub fn stopped_for_test() -> Self {
        // gstreamer::init() is idempotent and runs without plugins.
        let _ = gstreamer::init();
        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);
        // A minimal placeholder tee for the stopped_for_test variant.
        // We can't call gst::ElementFactory::make here safely without
        // gst init — but init() above ensures it. Use fakesink as the
        // tee placeholder; stopped_for_test is only used for state tests,
        // not fanout topology tests.
        let placeholder = gst::ElementFactory::make("fakesink")
            .build()
            .unwrap_or_else(|_| {
                // If even fakesink isn't available (stripped env), synthesise
                // a pipeline object as a stand-in — this path only needs
                // enough to avoid a panic on Drop.
                panic!("stopped_for_test: gstreamer init succeeded but fakesink unavailable — environment is too stripped");
            });
        Self {
            pipeline: gst::Pipeline::new(),
            whep_url: String::new(),
            state_tx,
            state_rx,
            bus_watch: None,
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            tee: Arc::new(placeholder),
        }
    }

    /// Test-only: force the observed state without going through the bus
    /// watch. Lets state-machine tests exercise Streaming/Errored branches
    /// without needing real GStreamer messaging.
    #[cfg(test)]
    pub fn set_state_for_test(&mut self, state: PipelineState) {
        self.state_tx.send_replace(state);
    }

    /// Build but do not yet start the pipeline.
    ///
    /// `whep_url` is the axum route path (e.g. `/ndi/whep/<source_id>`) used
    /// as a logical key; the element does NOT bind its own HTTP port.
    pub fn build(ndi_name: &str, whep_url: String) -> Result<Self> {
        super::init().context("gstreamer init failed")?;
        let encoder_name = super::hw_h264_encoder().ok_or_else(|| {
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
            "nvh264enc" | "nvcudah264enc" | "nvautogpuh264enc" => {
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
        encoder.link(&rtph264pay).context("link encoder -> rtph264pay")?;
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
            bus_watch: None,
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            tee: Arc::new(tee),
        })
    }

    /// Transition the pipeline to PLAYING. Returns immediately; the state
    /// watcher moves to `Streaming` once the PIPELINE element posts
    /// `StateChanged → Playing` on the bus.
    pub async fn start(&mut self) -> Result<()> {
        self.state_tx.send_replace(PipelineState::Starting);
        let pipeline = self.pipeline.clone();
        let state_tx = self.state_tx.clone();
        let pipeline_obj = pipeline.upcast_ref::<gst::Object>().clone();

        // Bus watch: drives the state transitions Starting → Streaming → Errored/Stopped.
        //
        // Live sources (ndisrc) skip `AsyncDone` — they go PAUSED → PLAYING
        // directly via `NoPreroll`. We watch `StateChanged` filtered to the
        // PIPELINE element itself and trip Streaming when it reaches PLAYING.
        // Element-level state changes are ignored — they fire earlier and would
        // race against encoder/tee setup.
        let bus = pipeline
            .bus()
            .ok_or_else(|| anyhow!("pipeline has no bus"))?;
        self.bus_watch = Some(tokio::spawn(async move {
            let mut stream = bus.stream();
            use futures_util::StreamExt;
            while let Some(msg) = stream.next().await {
                match msg.view() {
                    gst::MessageView::StateChanged(sc)
                        if sc.src() == Some(&pipeline_obj)
                            && sc.current() == gst::State::Playing =>
                    {
                        let _ = state_tx.send(PipelineState::Streaming);
                    }
                    gst::MessageView::AsyncDone(_) => {
                        // Harmless duplicate for live pipelines (the
                        // StateChanged branch above already fired); load-bearing
                        // for non-live test cases like videotestsrc.
                        let _ = state_tx.send(PipelineState::Streaming);
                    }
                    gst::MessageView::Error(err) => {
                        let detail = format!(
                            "{}: {}",
                            err.error(),
                            err.debug().unwrap_or_default().as_str()
                        );
                        tracing::error!(error = %detail, "pipeline error");
                        let _ = state_tx.send(PipelineState::Errored(detail));
                    }
                    gst::MessageView::Eos(_) => {
                        tracing::warn!("pipeline EOS received → state=Stopped");
                        let _ = state_tx.send(PipelineState::Stopped);
                    }
                    _ => {}
                }
            }
        }));

        pipeline
            .set_state(gst::State::Playing)
            .context("failed to set pipeline PLAYING")?;
        Ok(())
    }

    /// Tear down the pipeline. Safe to call multiple times.
    pub async fn stop(&mut self) {
        self.teardown();
        let _ = self.state_tx.send(PipelineState::Stopped);
    }

    /// Synchronous teardown: release per-consumer state, set pipeline state to
    /// Null, and abort the bus-watch task.
    /// Shared between `stop()` and `Drop` so the invariant lives in one place.
    /// Idempotent — GStreamer ignores a duplicate Null transition.
    fn teardown(&mut self) {
        // Release per-consumer resources before tearing down the pipeline.
        // Match the order used by remove_consumer: set elements to Null,
        // remove from pipeline, then release the tee request-pad.
        // sessions is a tokio::sync::Mutex; try_lock in Drop avoids blocking.
        if let Ok(mut sessions) = self.sessions.try_lock() {
            for (_id, session) in sessions.drain() {
                let _ = session.webrtcbin.set_state(gst::State::Null);
                let _ = session.queue.set_state(gst::State::Null);
                let _ = self.pipeline.remove(&session.webrtcbin);
                let _ = self.pipeline.remove(&session.queue);
                (*self.tee).release_request_pad(&session.tee_src_pad);
            }
        } else {
            // Lock contention during Drop is unusual. GStreamer will free the
            // elements when the bin drops anyway; leave a debug log rather than
            // spinning.
            tracing::debug!(
                "NdiPipeline teardown: sessions mutex contended; \
                 skipping explicit per-consumer cleanup (GStreamer will free on bin drop)"
            );
        }
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(h) = self.bus_watch.take() {
            h.abort();
        }
    }

    pub fn whep_url(&self) -> &str {
        &self.whep_url
    }

    pub fn state(&self) -> PipelineState {
        self.state_rx.borrow().clone()
    }

    pub fn state_watcher(&self) -> watch::Receiver<PipelineState> {
        self.state_rx.clone()
    }

    /// Test-only: force an `Errored` state transition without actually
    /// disturbing the underlying GStreamer pipeline. Used by the WHEP
    /// kill-pipeline test route to simulate an `ndisrc` "Internal data
    /// stream error" — the realistic failure mode that the production
    /// `PipelineSupervisor` is designed to recover from.
    ///
    /// The supervisor (still alive, still subscribed to this state
    /// channel) reacts to the Errored transition exactly as it would
    /// for a real ndisrc fault: rebuild the pipeline via
    /// `NdiManager::rebuild_pipeline`.
    #[cfg(feature = "test-helpers")]
    pub fn simulate_error_for_test(&self, msg: &str) {
        let _ = self.state_tx.send(PipelineState::Errored(msg.to_string()));
    }

    /// Compat shim for manager.rs (removed in Task 5).
    ///
    /// Returns `None` because we no longer have a `whepserversink` element.
    /// `manager.rs`'s `whep_signaller_call` (which still calls
    /// `emit_by_name` on `whepserversink`'s signaller) will `?` out on this
    /// `None` and surface a 503 to the HTTP shim — acceptable until Task 5
    /// routes `WhepOp` to the new `add_consumer` / `add_ice_candidate` /
    /// `remove_consumer` methods.
    ///
    /// The caps-wait in `manager.rs::rebuild_pipeline` also reads this; it
    /// will time out immediately with "pipeline has no sink element", which
    /// prevents the pipeline from reaching Streaming. Task 5 replaces the
    /// caps-wait with a tee-pad-based caps check.
    pub fn sink_element(&self) -> Option<gst::Element> {
        None
    }

    /// Add a WHEP consumer: request a tee src pad, create a webrtcbin
    /// element, link them via a queue, perform SDP offer/answer exchange,
    /// and return a `WhepAnswer` containing the SDP answer + initial ICE
    /// candidates.
    ///
    /// Non-Send glib work (element creation, linking, signal connections)
    /// runs inside `tokio::task::spawn_blocking`.
    pub async fn add_consumer(&self, sdp_offer_bytes: Vec<u8>) -> Result<WhepAnswer> {
        let session_id = WhepSession::new_session_id();
        let pipeline = self.pipeline.clone();
        let tee = (*self.tee).clone();

        // Channel for ICE candidates emitted by webrtcbin.
        let (ice_tx, mut ice_rx) = tokio::sync::mpsc::unbounded_channel::<IceCandidate>();
        let ice_tx_clone = ice_tx.clone();

        // Shared connection state updated by the notify::connection-state handler.
        // Uses std::sync::Mutex because the GStreamer signal fires from raw
        // std::thread (GLib streaming thread) — tokio::sync::Mutex would risk
        // deadlock in that context.
        let connection_state = Arc::new(std::sync::Mutex::new(WhepConnectionState::New));
        let connection_state_for_signal = connection_state.clone();

        let session_id_for_signal = session_id.clone();

        // Channel to receive the SDP answer from within spawn_blocking.
        // Returns (webrtcbin, queue, tee_pad, sdp_text) on success.
        let (answer_tx, answer_rx) =
            tokio::sync::oneshot::channel::<Result<(gst::Element, gst::Element, gst::Pad, String)>>();

        let sdp_offer_bytes_clone = sdp_offer_bytes.clone();
        let session_id_for_blocking = session_id.clone();

        tokio::task::spawn_blocking(move || {
            // Parse the SDP offer.
            let sdp_msg = gstreamer_webrtc::gst_sdp::SDPMessage::parse_buffer(
                &sdp_offer_bytes_clone,
            )
            .map_err(|e| anyhow!("SDP parse failed: {e}"))?;

            let offer_desc = gst_webrtc::WebRTCSessionDescription::new(
                gst_webrtc::WebRTCSDPType::Offer,
                sdp_msg,
            );

            // Step 1: request a tee src pad. On any subsequent failure we MUST
            // release this pad before returning — otherwise it leaks forever.
            let tee_pad = tee
                .request_pad_simple("src_%u")
                .ok_or_else(|| anyhow!("tee has no src_%u request pad template"))?;

            // Step 2: do all remaining work in an inner closure so that a single
            // match on the result can unconditionally release tee_pad on Err.
            // The closure returns (webrtcbin, queue, sdp_text) on success.
            let inner_result = (|| -> Result<(gst::Element, gst::Element, String)> {
                // Build queue + webrtcbin.
                let queue = gst::ElementFactory::make("queue")
                    .build()
                    .context("build queue")?;
                let webrtcbin = gst::ElementFactory::make("webrtcbin")
                    .name(session_id_for_blocking.as_str())
                    .build()
                    .context("build webrtcbin")?;

                pipeline.add_many([&queue, &webrtcbin]).context("add queue+webrtcbin")?;

                // From here, on any error we must remove queue + webrtcbin from the
                // pipeline before returning. Use a nested closure for this.
                let pipeline_result: Result<String> = (|| -> Result<String> {
                    // Link: tee_src → queue → webrtcbin
                    let queue_sink = queue
                        .static_pad("sink")
                        .ok_or_else(|| anyhow!("queue has no sink pad"))?;
                    tee_pad.link(&queue_sink).context("link tee_pad -> queue.sink")?;
                    queue.link(&webrtcbin).context("link queue -> webrtcbin")?;

                    // Connect on-ice-candidate signal.
                    // Signal signature: void(webrtcbin, sdp_mline_index: u32, candidate: &str)
                    // Confirmed from gst-inspect-1.0 webrtcbin and gst-plugin-webrtc-0.15.2/imp.rs:3211
                    // Args array: [webrtcbin_value, mline_index_value, candidate_value]
                    {
                        let ice_tx = ice_tx_clone.clone();
                        webrtcbin.connect("on-ice-candidate", false, move |args| {
                            let sdp_mline_index = args
                                .get(1)
                                .and_then(|v| v.get::<u32>().ok())
                                .unwrap_or(0);
                            let candidate = args
                                .get(2)
                                .and_then(|v| v.get::<String>().ok())
                                .unwrap_or_default();
                            let _ = ice_tx.send(IceCandidate {
                                sdp_mline_index,
                                candidate,
                            });
                            None
                        });
                    }

                    // Connect notify::connection-state to update shared state.
                    // This signal fires from a GStreamer streaming thread (raw std::thread
                    // spawned by GLib) — NOT from within a tokio context. We therefore use
                    // std::sync::Mutex::lock() directly; no async dance or block_on needed.
                    // The critical section is nanoseconds (a simple enum write), so
                    // contention is negligible.
                    webrtcbin.connect_notify(
                        Some("connection-state"),
                        {
                            let connection_state_for_signal = connection_state_for_signal.clone();
                            let session_id_for_signal = session_id_for_signal.clone();
                            move |webrtcbin, _pspec| {
                                let gst_state = webrtcbin
                                    .property::<gst_webrtc::WebRTCPeerConnectionState>("connection-state");
                                let our_state = WhepConnectionState::from(gst_state);
                                // std::sync::Mutex — safe to lock from any thread, no async runtime
                                // required. Panic on poison is acceptable; the signal thread holds
                                // the lock for a trivial enum write.
                                *connection_state_for_signal.lock().unwrap() = our_state;
                                tracing::debug!(
                                    session_id = %session_id_for_signal,
                                    state = ?our_state,
                                    "WHEP consumer connection-state changed"
                                );
                            }
                        },
                    );

                    // Set webrtcbin to PLAYING so it can do ICE + DTLS.
                    webrtcbin
                        .set_state(gst::State::Playing)
                        .context("set webrtcbin to Playing")?;

                    // Set the remote description (the browser's offer).
                    let (remote_desc_tx, remote_desc_rx) =
                        std::sync::mpsc::sync_channel::<()>(1);
                    let promise = gst::Promise::with_change_func(move |_reply| {
                        let _ = remote_desc_tx.send(());
                    });
                    webrtcbin.emit_by_name::<()>("set-remote-description", &[&offer_desc, &promise]);
                    // Wait for the set-remote-description promise to resolve.
                    // Propagate timeout as an error — proceeding to create-answer on a
                    // webrtcbin that hasn't processed the offer produces an invalid SDP.
                    remote_desc_rx
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .context("set-remote-description promise timed out or sender dropped")?;

                    // Create the SDP answer.
                    let (answer_sdp_tx, answer_sdp_rx) =
                        std::sync::mpsc::sync_channel::<Option<gst_webrtc::WebRTCSessionDescription>>(1);
                    let promise = gst::Promise::with_change_func(move |reply| {
                        let answer = reply
                            .ok()
                            .and_then(|r| r)
                            .and_then(|r| r.value("answer").ok())
                            .and_then(|v| v.get::<gst_webrtc::WebRTCSessionDescription>().ok());
                        let _ = answer_sdp_tx.send(answer);
                    });
                    webrtcbin.emit_by_name::<()>("create-answer", &[&None::<gst::Structure>, &promise]);

                    let answer_desc = answer_sdp_rx
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .context("create-answer promise timed out")?
                        .ok_or_else(|| anyhow!("create-answer returned no answer"))?;

                    // Set the local description (our answer).
                    webrtcbin.emit_by_name::<()>(
                        "set-local-description",
                        &[&answer_desc, &None::<gst::Promise>],
                    );

                    answer_desc
                        .sdp()
                        .as_text()
                        .map_err(|e| anyhow!("answer SDP as_text failed: {e}"))
                })();

                match pipeline_result {
                    Ok(sdp_text) => Ok((queue, webrtcbin, sdp_text)),
                    Err(e) => {
                        // Remove partially-allocated pipeline elements before releasing
                        // tee_pad (the outer match handles tee_pad release on Err).
                        let _ = webrtcbin.set_state(gst::State::Null);
                        let _ = queue.set_state(gst::State::Null);
                        let _ = pipeline.remove(&webrtcbin);
                        let _ = pipeline.remove(&queue);
                        Err(e)
                    }
                }
            })();

            match inner_result {
                Ok((queue, webrtcbin, sdp_text)) => {
                    answer_tx
                        .send(Ok((webrtcbin, queue, tee_pad, sdp_text)))
                        .ok();
                }
                Err(e) => {
                    // Release the tee pad we reserved at Step 1.
                    tee.release_request_pad(&tee_pad);
                    answer_tx.send(Err(e)).ok();
                }
            }
            Ok::<(), anyhow::Error>(())
        });

        // Await the answer from the blocking task.
        let (webrtcbin, queue, tee_pad, sdp_answer) = answer_rx
            .await
            .context("spawn_blocking answer channel dropped")??;

        // Drain any ICE candidates already buffered (half-trickle: include
        // in the WHEP answer body). Allow up to 50 ms for additional
        // candidates to trickle in.
        let mut initial_candidates = Vec::new();
        let drain_deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(50);
        while let Ok(Some(c)) = tokio::time::timeout_at(drain_deadline, ice_rx.recv()).await {
            initial_candidates.push(c);
        }

        // Store the session.
        let session = WhepSession {
            session_id: session_id.clone(),
            webrtcbin,
            queue,
            tee_src_pad: tee_pad,
            connection_state,
            ice_tx,
        };
        {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(session_id.clone(), session);
        }

        tracing::info!(
            session_id = %session_id,
            initial_candidates = initial_candidates.len(),
            "WHEP consumer added"
        );

        Ok(WhepAnswer {
            session_id,
            sdp_answer,
            initial_candidates,
        })
    }

    /// Forward a trickle ICE candidate from the browser to the webrtcbin.
    pub async fn add_ice_candidate(
        &self,
        session_id: &str,
        sdp_mline_index: u32,
        candidate: &str,
    ) -> Result<()> {
        let sessions = self.sessions.lock().await;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        let webrtcbin = session.webrtcbin.clone();
        let candidate = candidate.to_string();
        drop(sessions);
        tokio::task::spawn_blocking(move || {
            webrtcbin
                .emit_by_name::<()>("add-ice-candidate", &[&sdp_mline_index, &candidate]);
        })
        .await
        .context("spawn_blocking join")?;
        tracing::debug!(
            session_id,
            sdp_mline_index,
            "WHEP ICE candidate forwarded to webrtcbin"
        );
        Ok(())
    }

    /// Remove a WHEP consumer: tear down webrtcbin and release the tee pad.
    /// Idempotent — safe to call on an unknown session_id.
    pub async fn remove_consumer(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        let Some(session) = sessions.remove(session_id) else {
            tracing::debug!(
                session_id,
                "remove_consumer called on unknown session — idempotent no-op"
            );
            return Ok(());
        };
        let pipeline = self.pipeline.clone();
        let tee = (*self.tee).clone();
        let webrtcbin = session.webrtcbin.clone();
        let queue = session.queue.clone();
        let tee_pad = session.tee_src_pad.clone();
        let remaining_count = sessions.len();
        // Drop the WhepSession so its Drop impl sets webrtcbin + queue to Null
        // BEFORE we remove them from the pipeline.
        drop(session);
        drop(sessions);
        tokio::task::spawn_blocking(move || -> Result<()> {
            // Already Null via WhepSession::Drop, but idempotent.
            let _ = webrtcbin.set_state(gst::State::Null);
            let _ = queue.set_state(gst::State::Null);
            // Remove queue first (it is upstream of webrtcbin in the branch).
            pipeline
                .remove(&queue)
                .context("pipeline.remove queue")?;
            pipeline
                .remove(&webrtcbin)
                .context("pipeline.remove webrtcbin")?;
            tee.release_request_pad(&tee_pad);
            Ok(())
        })
        .await
        .context("spawn_blocking join")??;
        tracing::info!(
            session_id,
            remaining = remaining_count,
            "WHEP consumer removed"
        );
        Ok(())
    }

    /// Snapshot the pipeline state for the diagnostic route.
    /// `source_id` is left empty — the manager fills it in (Task 8).
    pub async fn snapshot(&self) -> PipelineSnapshot {
        let sessions = self.sessions.lock().await;
        let mut session_snaps = Vec::with_capacity(sessions.len());
        for (id, session) in sessions.iter() {
            let connection_state = *session.connection_state.lock().unwrap();
            session_snaps.push(SessionSnapshot {
                id: id.clone(),
                connection_state,
            });
        }
        // Collect once — iterate_encoders() walks the pipeline; calling it
        // twice would walk it twice for no benefit.
        let encoders: Vec<gst::Element> = self.iterate_encoders().collect();
        let encoder_count = encoders.len();
        let encoder_factory = encoders
            .into_iter()
            .next()
            .and_then(|el| el.factory().map(|f| f.name().to_string()));
        PipelineSnapshot {
            source_id: String::new(), // manager fills this in (Task 8)
            state: format!("{:?}", *self.state_rx.borrow()),
            encoder_factory,
            encoder_count,
            consumer_count: sessions.len(),
            sessions: session_snaps,
        }
    }

    /// Iterate encoder elements in the pipeline. Returns a collected Vec
    /// iterator so callers don't need to hold the pipeline lock.
    pub fn iterate_encoders(&self) -> impl Iterator<Item = gst::Element> + '_ {
        let mut found: Vec<gst::Element> = Vec::new();
        for el in self.pipeline.iterate_elements().into_iter().flatten() {
            if let Some(factory) = el.factory() {
                let name = factory.name();
                if name == "vah264enc"
                    || name == "nvh264enc"
                    || name == "x264enc"
                    || name == "nvcudah264enc"
                    || name == "nvautogpuh264enc"
                {
                    found.push(el);
                }
            }
        }
        found.into_iter()
    }

    /// Iterate webrtcbin elements in the pipeline.
    pub fn iterate_webrtcbins(&self) -> impl Iterator<Item = gst::Element> + '_ {
        let mut found: Vec<gst::Element> = Vec::new();
        for el in self.pipeline.iterate_elements().into_iter().flatten() {
            if let Some(factory) = el.factory() {
                if factory.name() == "webrtcbin" {
                    found.push(el);
                }
            }
        }
        found.into_iter()
    }
}

impl Drop for NdiPipeline {
    fn drop(&mut self) {
        self.teardown();
    }
}

#[cfg(test)]
impl NdiPipeline {
    /// Build a pipeline with the shared-encoder fanout topology using
    /// `videotestsrc` in place of `ndisrc`/`ndisrcdemux`, so the test runs on
    /// hosts without gst-plugin-ndi registered.
    ///
    /// Fails with an error if `encoder_name` is not registered.
    pub fn stopped_for_test_with_topology(encoder_name: &str) -> Result<Self> {
        super::init()?;
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
            "nvh264enc" | "nvcudah264enc" | "nvautogpuh264enc" => {
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
        let payer = gst::ElementFactory::make("rtph264pay")
            .name("rtpay")
            .build()
            .context("rtph264pay")?;
        let tee = gst::ElementFactory::make("tee")
            .name("tee")
            .property("allow-not-linked", true)
            .build()
            .context("tee")?;
        pipeline
            .add_many([&src, &convert, &encoder, &payer, &tee])
            .context("add")?;
        gst::Element::link_many([&src, &convert, &encoder, &payer, &tee]).context("link")?;
        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);
        Ok(Self {
            pipeline,
            whep_url: String::new(),
            state_tx,
            state_rx,
            bus_watch: None,
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            tee: Arc::new(tee),
        })
    }

    /// Sync stub: add a consumer WITHOUT SDP exchange (tests only).
    /// Enforces the same 8-consumer cap as production (cap enforcement
    /// is in Task 6; here we just mirror it via anyhow).
    pub fn add_consumer_stub(&mut self, session_id: &str) -> Result<()> {
        {
            let sessions = self
                .sessions
                .try_lock()
                .context("sessions mutex contention in test")?;
            if sessions.len() >= 8 {
                return Err(anyhow!(
                    "consumer cap reached (8 per source) — test stub mirror"
                ));
            }
        }
        let tee_pad = (*self.tee)
            .request_pad_simple("src_%u")
            .ok_or_else(|| anyhow!("tee has no src request pad"))?;
        // Use a real queue element so the WhepSession.queue field is valid.
        // This also keeps the test topology consistent with production (tee → queue → webrtcbin).
        let queue = gst::ElementFactory::make("queue")
            .build()
            .context("queue (test)")?;
        let webrtcbin = gst::ElementFactory::make("webrtcbin")
            .name(session_id)
            .build()
            .context("webrtcbin (test)")?;
        self.pipeline
            .add_many([&queue, &webrtcbin])
            .context("pipeline.add queue+webrtcbin")?;
        // Link: tee → queue → webrtcbin
        let queue_sink = queue
            .static_pad("sink")
            .ok_or_else(|| anyhow!("queue has no sink pad (test)"))?;
        tee_pad
            .link(&queue_sink)
            .context("link tee_pad -> queue.sink (test)")?;
        queue.link(&webrtcbin).context("link queue -> webrtcbin (test)")?;
        let (ice_tx, _ice_rx) = tokio::sync::mpsc::unbounded_channel();
        let session = WhepSession {
            session_id: session_id.to_string(),
            webrtcbin,
            queue,
            tee_src_pad: tee_pad,
            connection_state: Arc::new(std::sync::Mutex::new(WhepConnectionState::New)),
            ice_tx,
        };
        self.sessions
            .try_lock()
            .unwrap()
            .insert(session_id.to_string(), session);
        Ok(())
    }

    /// Sync stub: remove a consumer (tests only). Idempotent.
    pub fn remove_consumer_stub(&mut self, session_id: &str) -> Result<()> {
        let Some(session) = self.sessions.try_lock().unwrap().remove(session_id) else {
            return Ok(());
        };
        let _ = session.webrtcbin.set_state(gst::State::Null);
        let _ = session.queue.set_state(gst::State::Null);
        self.pipeline.remove(&session.queue).ok();
        self.pipeline.remove(&session.webrtcbin).ok();
        (*self.tee).release_request_pad(&session.tee_src_pad);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Regression test for #336: shared-encoder fanout.
    ///
    /// The pre-fix pipeline ended in `whepserversink`, which (by gst-plugin-rs
    /// 0.15 design) spawns one independent encoder per WHEP consumer. With
    /// multiple stage-display browsers connecting to the same NDI source,
    /// 3-4 encoders saturated the N100's iGPU VAAPI scheduler — the
    /// 2026-05-24 production incident.
    ///
    /// The fix builds the pipeline with `ndisrc → demux → videoconvert →
    /// vah264enc → rtph264pay → tee` (one encoder), then `add_consumer`
    /// dynamically requests a tee src pad + spawns one `webrtcbin` per
    /// consumer that reads from the shared encoded stream. This test asserts
    /// the load-bearing invariant: regardless of how many consumers are
    /// added, the pipeline iterator yields EXACTLY ONE encoder element.
    ///
    /// Runs on every CI host (no GPU/libndi required) — uses
    /// `stopped_for_test_with_topology()` which builds the encoder + tee
    /// + per-consumer webrtcbin elements with `x264enc` (always available)
    /// in lieu of real HW encoders.
    #[tokio::test]
    async fn pipeline_has_single_encoder_for_n_consumers() {
        super::super::init().expect("gst init");
        // Build a pipeline-shaped value with the fanout topology, force
        // `x264enc` as the encoder so this runs on every CI host.
        let mut pipeline = NdiPipeline::stopped_for_test_with_topology("x264enc")
            .expect("test-only topology builder must succeed when x264enc registered");

        // Simulate N WHEP POSTs.
        for i in 0..4 {
            pipeline
                .add_consumer_stub(&format!("test-session-{i}"))
                .expect("add_consumer must succeed up to the soft cap");
        }

        // Count elements whose factory name is one of the encoder factories.
        let encoder_factories = ["vah264enc", "nvh264enc", "x264enc"];
        let encoder_count = pipeline.iterate_encoders().count();
        let webrtcbin_count = pipeline.iterate_webrtcbins().count();

        assert_eq!(
            encoder_count, 1,
            "REGRESSION (#336): pipeline must have EXACTLY ONE encoder for N consumers; \
             got {encoder_count} encoders for 4 consumers. Encoder factories considered: {encoder_factories:?}"
        );
        assert_eq!(
            webrtcbin_count, 4,
            "pipeline must have one webrtcbin per consumer; got {webrtcbin_count} for 4 consumers"
        );
    }
}
