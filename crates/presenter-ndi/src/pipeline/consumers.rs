//! WHEP consumer management: build/tear down a FRESH per-consumer pipeline
//! (`appsrc → rtph264pay|rtpvp8pay → webrtcbin`) fed by the requested
//! profile branch's `StreamProducer` (the `?profile=compat` WHEP query
//! selects the 854×480@20 realtime-VP8 compat stream; everything else gets
//! the 720p H264 default — see [`super::StreamProfile`], whose profile
//! implies the codec), forward trickle ICE, and diagnostic snapshots.
//!
//! This follows the gst-plugin-rs `webrtcsink` reference recipe exactly:
//!
//! 1. one FRESH `gst::Pipeline` per consumer (a webrtcbin spliced into an
//!    already-running pipeline never gets its rtpsession latency configured —
//!    the #373 "connected but black" straggler bug);
//! 2. the consumer pipeline SHARES the encoder pipeline's clock + base-time
//!    (`use_clock` / `set_start_time(NONE)` / `set_base_time`), so forwarded
//!    buffer timestamps stay valid on the consumer's timeline;
//! 3. a per-pipeline BUS WATCH services every `Latency` message with
//!    `recalculate_latency()` — webrtcbin adds its transport elements DURING
//!    negotiation, and without this handler the new elements' latency is never
//!    distributed (the root cause of the nondeterministic zero-RTP failures);
//! 4. the appsink→appsrc bridge is `gstreamer_utils::StreamProducer`, which
//!    forwards samples (caps, segment and PTS preserved), propagates producer
//!    latency to the consumer appsrcs, gates new consumers on a keyframe, and
//!    forwards force-keyunit (browser PLI) requests upstream to the encoder;
//! 5. create-answer WAITS for media caps (with the send `ssrc`) to reach the
//!    webrtcbin sink pad, so the answer always announces the SSRC the browser
//!    needs to demux the stream (see `negotiation::await_media_caps`).
//!
//! Construction runs inside `tokio::task::spawn_blocking` (non-Send glib
//! work); every partially-built consumer is owned by the [`ConsumerBranch`]
//! RAII guard, so EVERY exit path — error, async cancellation of the awaiting
//! HTTP handler, or a value stranded in the oneshot channel — tears the
//! pipeline down instead of leaking it.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_utils::{ConsumptionLink, StreamProducer};
use gstreamer_video as gst_video;
use gstreamer_webrtc as gst_webrtc;
use tokio::sync::mpsc::UnboundedSender;

use super::negotiation::{
    align_payload_type, await_ice_gathering, await_media_caps, negotiate_sdp,
    parse_h264_payload_type, parse_vp8_payload_type,
};
use super::{
    build::{consumer_h264_caps, consumer_vp8_caps},
    AddConsumerError, NdiPipeline, PipelineSnapshot, SessionSnapshot, StreamProfile, WhepAnswer,
};
use crate::whep_session::{IceCandidate, LivenessState, WhepConnectionState, WhepSession};

/// RAII owner of a consumer pipeline from the moment it is assembled until it
/// is either promoted into a stored [`WhepSession`] (via [`Self::defuse`]) or
/// dropped — in which case Drop aborts the bus-watch task, disconnects the
/// StreamProducer link, and sets the pipeline to Null. This single guard
/// covers every leak window: `?` error paths in the blocking builder, the
/// oneshot value stranded when the receiver is cancelled, and cancellation of
/// `add_consumer` between receiving the value and storing the session.
pub(super) struct ConsumerBranch {
    pipeline: gst::Pipeline,
    webrtcbin: gst::Element,
    bus_task: Option<tokio::task::JoinHandle<()>>,
    link: Option<ConsumptionLink>,
    sdp_answer: String,
    session_id: String,
    armed: bool,
}

impl ConsumerBranch {
    fn new(pipeline: gst::Pipeline, webrtcbin: gst::Element, session_id: &str) -> Self {
        Self {
            pipeline,
            webrtcbin,
            bus_task: None,
            link: None,
            sdp_answer: String::new(),
            session_id: session_id.to_string(),
            armed: true,
        }
    }

    /// Take ownership of the parts for a stored `WhepSession`, disarming the
    /// guard. Returns None if the branch never completed negotiation (no link
    /// or bus task) — callers treat that as an internal error.
    #[allow(clippy::type_complexity)]
    fn defuse(
        mut self,
    ) -> Option<(
        gst::Pipeline,
        gst::Element,
        String,
        ConsumptionLink,
        tokio::task::JoinHandle<()>,
    )> {
        let link = self.link.take()?;
        let bus_task = self.bus_task.take()?;
        self.armed = false;
        Some((
            self.pipeline.clone(),
            self.webrtcbin.clone(),
            std::mem::take(&mut self.sdp_answer),
            link,
            bus_task,
        ))
    }
}

impl Drop for ConsumerBranch {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(task) = self.bus_task.take() {
            task.abort();
        }
        // Dropping the link disconnects this consumer from the StreamProducer.
        drop(self.link.take());
        let _ = self.pipeline.set_state(gst::State::Null);
        tracing::debug!(
            session_id = %self.session_id,
            "ConsumerBranch dropped before becoming a session — torn down"
        );
    }
}

impl NdiPipeline {
    /// The encoder-branch producer serving `profile` — the single point
    /// where a consumer's requested profile maps to a concrete stream.
    /// `request_keyframe` MUST target the producer returned here so a
    /// compat joiner's IDR request reaches the compat encoder.
    pub(super) fn producer_for_profile(&self, profile: StreamProfile) -> &StreamProducer {
        match profile {
            StreamProfile::Default => &self.producer,
            StreamProfile::Compat => &self.producer_compat,
        }
    }

    /// Add a WHEP consumer: build a fresh `appsrc → rtph264pay|rtpvp8pay →
    /// webrtcbin` pipeline on the encoder's clock, fed from the branch the
    /// consumer's WHEP POST requested (`profile` — `?profile=compat` selects
    /// the 854×480@20 realtime-VP8 compat stream; the profile implies the
    /// codec), perform the SDP offer/answer exchange, connect its appsrc to
    /// that branch's `StreamProducer`, and return a `WhepAnswer` (SDP answer
    /// + initial ICE candidates).
    ///
    /// Returns `Err(AddConsumerError::CapReached)` before allocating any
    /// GStreamer resources when the session count is at
    /// `MAX_CONSUMERS_PER_SOURCE` — but only AFTER reaping dead (zombie)
    /// sessions, so a map full of vanished clients self-heals exactly when a
    /// new display tries to join (see `reap_and_check_cap`).
    pub async fn add_consumer(
        &self,
        sdp_offer_bytes: Vec<u8>,
        profile: StreamProfile,
    ) -> Result<WhepAnswer, AddConsumerError> {
        self.reap_and_check_cap().await?;

        let session_id = WhepSession::new_session_id();
        // Select the profile's producer HERE; the blocking builder receives
        // exactly one producer and everything downstream (appsrc link,
        // keyframe request) follows it.
        let producer = self.producer_for_profile(profile).clone();
        // Share the ENCODER pipeline's clock + base-time with the consumer
        // pipeline (webrtcsink does exactly this for its session pipelines).
        // The encoder pipeline is guaranteed Streaming before WHEP POSTs are
        // routed (manager's ensure_streaming gate), so these are valid.
        let enc_clock = self.pipeline.clock();
        let enc_base_time = self.pipeline.base_time();
        // Tokio runtime handle so the blocking builder can spawn the
        // per-pipeline bus-watch task.
        let rt = tokio::runtime::Handle::current();

        // Channel for ICE candidates emitted by webrtcbin. `ice_tx` is kept for
        // the stored WhepSession; a clone is moved into the signal closure.
        let (ice_tx, mut ice_rx) = tokio::sync::mpsc::unbounded_channel::<IceCandidate>();
        let ice_tx_for_signal = ice_tx.clone();

        // Shared connection state updated by the notify::connection-state handler
        // (fires from a GStreamer streaming thread → std::sync::Mutex).
        let connection_state = Arc::new(std::sync::Mutex::new(WhepConnectionState::New));
        let connection_state_for_signal = connection_state.clone();

        // Channel to receive the negotiation result from within spawn_blocking.
        // The value is a ConsumerBranch GUARD: if this future is cancelled
        // while the branch sits buffered in the channel, the channel's drop
        // tears the pipeline down instead of leaking it.
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<Result<ConsumerBranch>>();

        let session_id_for_blocking = session_id.clone();

        tokio::task::spawn_blocking(move || {
            let result = build_consumer_pipeline_blocking(
                &producer,
                profile,
                &sdp_offer_bytes,
                &session_id_for_blocking,
                ice_tx_for_signal,
                connection_state_for_signal,
                enc_clock,
                enc_base_time,
                rt,
            );
            // If the receiver is already gone, the returned branch guard drops
            // here and tears itself down.
            let _ = answer_tx.send(result);
        });

        // Await the negotiation result, then IMMEDIATELY promote the branch
        // into a WhepSession: from this point any cancellation of this future
        // (client disconnect) drops the session, whose Drop performs the full
        // teardown — no awaits happen while the parts are unguarded.
        let branch = answer_rx
            .await
            .context("spawn_blocking answer channel dropped")??;
        let (consumer_pipeline, webrtcbin, sdp_answer, link, bus_task) = branch
            .defuse()
            .ok_or_else(|| anyhow!("negotiated consumer branch missing link or bus task"))?;
        let session = WhepSession {
            session_id: session_id.clone(),
            consumer_pipeline,
            webrtcbin,
            link,
            bus_task,
            connection_state,
            // Fresh RTCP-liveness tracker: full stale-window grace from now.
            liveness: Arc::new(std::sync::Mutex::new(LivenessState::new())),
            ice_tx,
        };

        // Drain any ICE candidates already buffered (half-trickle: include
        // in the WHEP answer body). Allow up to 50 ms for more to trickle.
        let mut initial_candidates = Vec::new();
        let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(50);
        while let Ok(Some(c)) = tokio::time::timeout_at(drain_deadline, ice_rx.recv()).await {
            initial_candidates.push(c);
        }

        {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(session_id.clone(), session);
        }

        tracing::info!(
            session_id = %session_id,
            profile = ?profile,
            initial_candidates = initial_candidates.len(),
            "WHEP consumer added (own pipeline on shared clock)"
        );

        Ok(WhepAnswer {
            session_id,
            sdp_answer,
            initial_candidates,
        })
    }

    /// Forward a trickle ICE candidate from the browser to the webrtcbin.
    ///
    /// NOTE: the deployed WHEP flow is NON-TRICKLE — the browser waits for its
    /// own ICE gathering to complete before POSTing the offer, and the server
    /// returns a full answer with all candidates (see `await_ice_gathering`). A
    /// late trickle candidate arriving before the session is inserted into the
    /// map returns "session not found"; acceptable because the answer already
    /// carried the full candidate set.
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
            webrtcbin.emit_by_name::<()>("add-ice-candidate", &[&sdp_mline_index, &candidate]);
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

    /// Remove a WHEP consumer: disconnect it from the StreamProducer and tear
    /// down its pipeline. Idempotent — safe to call on an unknown session_id.
    pub async fn remove_consumer(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        let Some(session) = sessions.remove(session_id) else {
            tracing::debug!(
                session_id,
                "remove_consumer called on unknown session — idempotent no-op"
            );
            return Ok(());
        };
        let remaining_count = sessions.len();
        drop(sessions);
        // Fanout-delivery stats: how many encoded buffers this consumer was
        // actually fed vs dropped (e.g. while waiting for its first keyframe).
        let pushed = session.link.pushed();
        let dropped = session.link.dropped();
        // Dropping the WhepSession: ConsumptionLink::drop disconnects from the
        // producer, Drop aborts the bus task and nulls the pipeline. Run the
        // (blocking) pipeline teardown off the async thread.
        tokio::task::spawn_blocking(move || drop(session))
            .await
            .context("spawn_blocking join")?;
        tracing::info!(
            session_id,
            remaining = remaining_count,
            buffers_pushed = pushed,
            buffers_dropped = dropped,
            "WHEP consumer removed (pipeline torn down)"
        );
        Ok(())
    }

    /// Snapshot the pipeline state for the diagnostic route.
    /// `source_id` is left empty — the manager fills it in (Task 8).
    pub async fn snapshot(&self) -> PipelineSnapshot {
        // Phase 1 (cheap, under the lock): identity + counters + webrtcbin handle.
        let partial: Vec<(String, WhepConnectionState, u64, u64, gst::Element)> = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(id, session)| {
                    let connection_state = *session
                        .connection_state
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    (
                        id.clone(),
                        connection_state,
                        session.link.pushed(),
                        session.link.dropped(),
                        session.webrtcbin.clone(),
                    )
                })
                .collect()
        };
        // Phase 2 (blocking): RTCP receiver-report stats per webrtcbin — the
        // get-stats promise wait must NOT block the async thread.
        let session_snaps: Vec<SessionSnapshot> = tokio::task::spawn_blocking(move || {
            partial
                .into_iter()
                .map(|(id, connection_state, pushed, dropped, webrtcbin)| {
                    let (rtt, jitter, lost) = rtcp_remote_inbound(&webrtcbin);
                    SessionSnapshot {
                        id,
                        connection_state,
                        buffers_pushed: pushed,
                        buffers_dropped: dropped,
                        rtcp_round_trip_ms: rtt,
                        rtcp_jitter_ms: jitter,
                        rtcp_packets_lost: lost,
                    }
                })
                .collect()
        })
        .await
        .unwrap_or_default();
        let encoders: Vec<gst::Element> = self.iterate_encoders().collect();
        let encoder_count = encoders.len();
        let encoder_factory = encoders
            .into_iter()
            .next()
            .and_then(|el| el.factory().map(|f| f.name().to_string()));
        let consumer_count = session_snaps.len();
        PipelineSnapshot {
            source_id: String::new(),
            state: format!("{:?}", *self.state_rx.borrow()),
            encoder_factory,
            encoder_count,
            consumer_count,
            sessions: session_snaps,
        }
    }

    /// Iterate encoder elements in the ENCODER pipeline. Returns a collected Vec
    /// iterator so callers don't need to hold the pipeline lock.
    ///
    /// The ENCODER pipeline holds EXACTLY TWO encoder elements BY DESIGN —
    /// "encoder" (H264, 720p default profile) and "encoder_compat" (vp8enc,
    /// 854×480 realtime-VP8 compat profile): one per PROFILE, never per
    /// consumer. Consumer pipelines contain NO encoder (only
    /// appsrc/rtph264pay|rtpvp8pay/webrtcbin) — the #336 invariant.
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
                    || name == "vp8enc"
                {
                    found.push(el);
                }
            }
        }
        found.into_iter()
    }

    /// Collect the webrtcbin elements across ALL consumer pipelines (one per
    /// active consumer). Used by tests to assert per-consumer fanout counts.
    /// Uses `try_lock` so it never blocks; in production it is diagnostic only.
    pub fn iterate_webrtcbins(&self) -> Vec<gst::Element> {
        let mut found: Vec<gst::Element> = Vec::new();
        if let Ok(sessions) = self.sessions.try_lock() {
            for session in sessions.values() {
                for el in session
                    .consumer_pipeline
                    .iterate_elements()
                    .into_iter()
                    .flatten()
                {
                    if let Some(factory) = el.factory() {
                        if factory.name() == "webrtcbin" {
                            found.push(el);
                        }
                    }
                }
            }
        }
        found
    }
}

/// Build a fresh `appsrc → rtph264pay|rtpvp8pay → webrtcbin` pipeline for
/// one consumer, negotiate SDP, connect its appsrc to the requested profile
/// branch's StreamProducer (already selected by `add_consumer`), and return
/// the [`ConsumerBranch`] guard owning all of it.
///
/// The profile implies the codec (`StreamProfile::encoding_name` — Default →
/// H264, Compat → VP8), so the offer MUST carry that codec's rtpmap — its
/// dynamic payload type pre-seats the per-consumer payloader. An offer
/// without it is rejected (presetting pt=96 could NEVER work — the browser
/// can't decode a codec it didn't offer). In practice every browser offer
/// carries both H264 and VP8.
///
/// ORDER (load-bearing): build elements (appsrc configured LIVE before any
/// state change) → new pipeline on the ENCODER's clock/base-time → link →
/// guard → signals → bus watch → PLAYING (and wait) → connect appsrc to the
/// producer → wait for media caps (ssrc) on the webrtcbin sink pad → SDP
/// offer/answer/SLD → align payload type → await ICE gathering → read the
/// final local SDP. Any `?` exit drops the guard, which tears everything down.
#[allow(clippy::too_many_arguments)]
fn build_consumer_pipeline_blocking(
    producer: &StreamProducer,
    profile: StreamProfile,
    sdp_offer_bytes: &[u8],
    session_id: &str,
    ice_tx: UnboundedSender<IceCandidate>,
    connection_state: Arc<std::sync::Mutex<WhepConnectionState>>,
    enc_clock: Option<gst::Clock>,
    enc_base_time: Option<gst::ClockTime>,
    rt: tokio::runtime::Handle,
) -> Result<ConsumerBranch> {
    let sdp_msg = gstreamer_webrtc::gst_sdp::SDPMessage::parse_buffer(sdp_offer_bytes)
        .map_err(|e| anyhow!("SDP parse failed: {e}"))?;
    let offer_desc =
        gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Offer, sdp_msg);
    let offer_str = std::str::from_utf8(sdp_offer_bytes).unwrap_or("");
    let encoding_name = profile.encoding_name();
    // Both tiers stream constrained-baseline H264 now (compat = low-res H264,
    // not VP8); the profile selects resolution, so the payload type is H264's.
    let pt = match profile {
        StreamProfile::Default => parse_h264_payload_type(offer_str),
        StreamProfile::Compat => parse_h264_payload_type(offer_str),
    };
    let Some(pt) = pt else {
        tracing::warn!(
            session_id = %session_id,
            profile = ?profile,
            codec = encoding_name,
            "WHEP offer carries no rtpmap for the requested profile's codec — \
             rejecting consumer (the browser could not decode this stream)"
        );
        return Err(anyhow!(
            "WHEP offer contains no {encoding_name} video rtpmap"
        ));
    };

    let (appsrc, payloader, webrtcbin) = build_consumer_elements(session_id, profile, pt)?;

    let consumer_pipeline = gst::Pipeline::with_name(&format!("consumer_{session_id}"));
    adopt_encoder_timeline(&consumer_pipeline, enc_clock, enc_base_time, session_id);
    consumer_pipeline
        .add_many([appsrc.upcast_ref::<gst::Element>(), &payloader, &webrtcbin])
        .context("add appsrc+payloader+webrtcbin to consumer pipeline")?;

    // From here every resource is owned by the guard: any `?` below drops it,
    // aborting the bus task (if spawned), disconnecting the producer link (if
    // connected) and setting the pipeline to Null.
    let mut branch = ConsumerBranch::new(consumer_pipeline.clone(), webrtcbin.clone(), session_id);

    // appsrc → payloader → webrtcbin. The pay→webrtc link is filtered to the
    // profile codec's application/x-rtp caps (payload OMITTED — it is
    // re-aligned to the negotiated pt in align_payload_type; pinning it here
    // would fight that).
    appsrc
        .upcast_ref::<gst::Element>()
        .link(&payloader)
        .context("link appsrc -> payloader")?;
    let rtp_caps = gst::Caps::builder("application/x-rtp")
        .field("media", "video")
        .field("encoding-name", encoding_name)
        .field("clock-rate", 90_000i32)
        .build();
    payloader
        .link_filtered(&webrtcbin, &rtp_caps)
        .context("link payloader -> webrtcbin (codec caps)")?;

    connect_branch_signals(&webrtcbin, ice_tx, connection_state, session_id.to_string());

    // Install the bus watch BEFORE going PLAYING. THE load-bearing part:
    // webrtcbin constructs its transport elements (nicesink, dtls, rtpbin
    // internals) DURING SDP negotiation — i.e. AFTER the pipeline's initial
    // latency distribution. GStreamer signals this by posting a `Latency`
    // message which the APPLICATION must service with recalculate_latency()
    // (gst-launch and webrtcsink both do). Without it, the rtpsession's send
    // latency stays unconfigured → "Can't determine running time for this
    // packet without knowing configured latency" → ZERO RTP forwarded.
    branch.bus_task = Some(spawn_consumer_bus_watch(
        &consumer_pipeline,
        session_id,
        &rt,
    )?);

    // Bring the pipeline to PLAYING and WAIT for it, so clock/base-time and
    // the initial latency configuration are in place before media flows.
    consumer_pipeline
        .set_state(gst::State::Playing)
        .context("set consumer pipeline PLAYING")?;
    let (sret, _cur, _pending) = consumer_pipeline.state(Some(gst::ClockTime::from_seconds(5)));
    sret.context("consumer pipeline did not reach PLAYING within 5s")?;

    // Connect this consumer's appsrc to the encoder's StreamProducer. The
    // producer starts forwarding samples at the next keyframe (requesting one
    // upstream via force-key-unit).
    branch.link = Some(
        producer
            .add_consumer(&appsrc)
            .map_err(|e| anyhow!("StreamProducer::add_consumer failed: {e}"))?,
    );

    // GOP is 240 frames — explicitly request an IDR so this consumer starts
    // decoding immediately instead of waiting for the GOP boundary.
    request_keyframe(producer);

    // The answer must announce the send SSRC — wait for media caps first.
    // (await_media_caps keys on the `ssrc` caps field, codec-agnostic.)
    await_media_caps(&webrtcbin, session_id);
    negotiate_sdp(&webrtcbin, &offer_desc, session_id)?;
    align_payload_type(&webrtcbin, &payloader, encoding_name, session_id);
    await_ice_gathering(&webrtcbin, session_id);

    let local_desc =
        webrtcbin.property::<gst_webrtc::WebRTCSessionDescription>("local-description");
    branch.sdp_answer = local_desc
        .sdp()
        .as_text()
        .map_err(|e| anyhow!("local-description SDP as_text failed: {e}"))?;
    Ok(branch)
}

/// Put a consumer pipeline on the ENCODER pipeline's clock + base-time so the
/// producer's forwarded buffer timestamps (PTS + segment, preserved by
/// StreamProducer's push_sample) are valid on this pipeline's timeline.
/// set_start_time(NONE) stops the PLAYING transition from re-selecting a
/// base-time. This mirrors webrtcsink's session-pipeline setup verbatim.
fn adopt_encoder_timeline(
    consumer_pipeline: &gst::Pipeline,
    enc_clock: Option<gst::Clock>,
    enc_base_time: Option<gst::ClockTime>,
    session_id: &str,
) {
    if let Some(clock) = &enc_clock {
        consumer_pipeline.use_clock(Some(clock));
    }
    consumer_pipeline.set_start_time(gst::ClockTime::NONE);
    match enc_base_time {
        Some(base) => consumer_pipeline.set_base_time(base),
        None => {
            // Defensive: should not happen (WHEP POSTs are gated on the encoder
            // pipeline being Streaming). Surface it loudly — timestamps would
            // be on the wrong timeline.
            tracing::warn!(
                session_id = %session_id,
                "encoder pipeline has no base-time; consumer timeline may be wrong"
            );
        }
    }
}

/// Spawn the per-consumer-pipeline bus watch: services `Latency` messages with
/// `recalculate_latency()` (via `call_async`, exactly like webrtcsink) and
/// logs pipeline errors. The task never ends on its own (it holds the bus
/// alive); the explicit abort — by `ConsumerBranch`/`WhepSession` Drop — is
/// its ONLY exit.
fn spawn_consumer_bus_watch(
    consumer_pipeline: &gst::Pipeline,
    session_id: &str,
    rt: &tokio::runtime::Handle,
) -> Result<tokio::task::JoinHandle<()>> {
    let bus = consumer_pipeline
        .bus()
        .ok_or_else(|| anyhow!("consumer pipeline has no bus"))?;
    let pipeline_weak = consumer_pipeline.downgrade();
    let sid = session_id.to_string();
    Ok(rt.spawn(async move {
        use futures_util::StreamExt;
        let mut stream = bus.stream();
        while let Some(msg) = stream.next().await {
            match msg.view() {
                gst::MessageView::Latency(_) => {
                    if let Some(pipeline) = pipeline_weak.upgrade() {
                        // call_async: run on a GStreamer worker thread, never
                        // blocking this task or risking a state-lock deadlock.
                        pipeline.call_async(|p| {
                            let _ = p.recalculate_latency();
                        });
                    }
                }
                gst::MessageView::Error(err) => {
                    tracing::warn!(
                        session_id = %sid,
                        error = %err.error(),
                        debug = ?err.debug(),
                        "consumer pipeline error (client watchdog will reconnect)"
                    );
                }
                _ => {}
            }
        }
    }))
}

/// Build the per-consumer elements: an `appsrc` (caps matching the requested
/// profile branch's producer appsink — byte-stream/AU H264 for Default,
/// video/x-vp8 for Compat), a per-consumer payloader (`rtph264pay` /
/// `rtpvp8pay`) pre-seated on the offer's dynamic payload type `pt` for the
/// profile's codec, and a `webrtcbin`.
pub(super) fn build_consumer_elements(
    session_id: &str,
    profile: StreamProfile,
    pt: u32,
) -> Result<(gst_app::AppSrc, gst::Element, gst::Element)> {
    // Initial caps match the profile branch's producer appsink caps filter
    // so the very first forwarded sample agrees (`consumer_h264_caps` /
    // `consumer_vp8_caps` pin BOTH sides of each bridge).
    let bridge_caps = match profile {
        StreamProfile::Default => consumer_h264_caps(),
        StreamProfile::Compat => consumer_h264_caps(),
    };
    let appsrc = gst_app::AppSrc::builder()
        .name(format!("src_{session_id}"))
        .caps(&bridge_caps)
        .build();
    // DIAGNOSTIC (temporary): gap probe on the per-consumer appsrc OUTPUT
    // (post-fanout). A >250ms gap here ⇒ the StreamProducer fanout stalls
    // (server-side, fixable); smooth here while TVs still blip ⇒ network/TV
    // jitter-buffer/decode (downstream of us).
    crate::pipeline::build::install_gap_probe(
        appsrc.upcast_ref::<gst::Element>(),
        "src",
        "appsrc-out",
    );
    // CRITICAL ORDER: apply the consumer configuration (is-live=true,
    // format=time, leaky downstream, 500ms queue bound) NOW — BEFORE the
    // pipeline transitions to PLAYING. basesrc latches `live_running` only at
    // the PAUSED→PLAYING transition and ONLY if the source is already live;
    // if `is_live` flips to true afterwards (which is what happened when
    // StreamProducer::add_consumer — which calls this internally — ran after
    // PLAYING), the appsrc's task blocks in "live source waiting for running
    // state" FOREVER and not a single buffer is ever pushed downstream —
    // connected, but black. add_consumer re-applies the same configuration
    // later, which is a harmless no-op.
    StreamProducer::configure_consumer(&appsrc);

    // Per-consumer payloader so each webrtcbin negotiates its own dynamic
    // payload type with its browser (#336); pre-seated on the browser's
    // offered pt for the profile's codec.
    let payloader = match profile {
        StreamProfile::Default => build_h264_payloader(session_id, pt)?,
        StreamProfile::Compat => build_h264_payloader(session_id, pt)?,
    };

    let webrtcbin = gst::ElementFactory::make("webrtcbin")
        .name(session_id)
        // max-bundle: audio + video on ONE ICE/DTLS transport, matching the
        // browser's `a=group:BUNDLE` offer (default `none` hangs the 2nd DTLS
        // handshake → connecting forever → black).
        .property_from_str("bundle-policy", "max-bundle")
        // Explicit jitterbuffer/session latency (200 ms is webrtcbin's own
        // default; set explicitly so the value is visible and stable).
        .property("latency", 200u32)
        .build()
        .context("build webrtcbin")?;

    Ok((appsrc, payloader, webrtcbin))
}

/// Per-consumer `rtph264pay`. config-interval=-1 resends SPS/PPS before every
/// IDR; aggregate-mode=zero-latency is webrtcsink parity (aggregate NALs only
/// until a VCL unit is complete — never hold a frame's data back for packing
/// efficiency).
fn build_h264_payloader(session_id: &str, pt: u32) -> Result<gst::Element> {
    gst::ElementFactory::make("rtph264pay")
        .name(format!("pay_{session_id}"))
        .property("config-interval", -1i32)
        .property("pt", pt)
        .property_from_str("aggregate-mode", "zero-latency")
        .build()
        .context("build rtph264pay")
}

/// Per-consumer `rtpvp8pay` for compat consumers. Only the pt needs seating
/// — VP8 has no SPS/PPS-style config to re-insert and the payloader
/// fragments each frame (including its token partitions) per RFC 7741 as-is.
fn build_vp8_payloader(session_id: &str, pt: u32) -> Result<gst::Element> {
    gst::ElementFactory::make("rtpvp8pay")
        .name(format!("pay_{session_id}"))
        .property("pt", pt)
        .build()
        .context("build rtpvp8pay")
}

/// Connect the per-consumer webrtcbin signals: on-ice-candidate (forwards
/// candidates to `ice_tx`) and notify::connection-state (updates the shared
/// `connection_state`). Both fire from a GStreamer streaming thread.
fn connect_branch_signals(
    webrtcbin: &gst::Element,
    ice_tx: UnboundedSender<IceCandidate>,
    connection_state: Arc<std::sync::Mutex<WhepConnectionState>>,
    session_id: String,
) {
    // on-ice-candidate signature: void(webrtcbin, sdp_mline_index: u32, candidate: &str)
    webrtcbin.connect("on-ice-candidate", false, move |args| {
        let sdp_mline_index = args.get(1).and_then(|v| v.get::<u32>().ok()).unwrap_or(0);
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

    // notify::connection-state fires from a GStreamer streaming thread (raw
    // std::thread) — use std::sync::Mutex directly. On poison recover the guard.
    webrtcbin.connect_notify(Some("connection-state"), move |webrtcbin, _pspec| {
        let gst_state =
            webrtcbin.property::<gst_webrtc::WebRTCPeerConnectionState>("connection-state");
        let our_state = WhepConnectionState::from(gst_state);
        *connection_state.lock().unwrap_or_else(|p| p.into_inner()) = our_state;
        tracing::debug!(
            session_id = %session_id,
            state = ?our_state,
            "WHEP consumer connection-state changed"
        );
    });
}

/// Ask the producer's branch encoder for an immediate keyframe — an IDR with
/// SPS/PPS on the H264 branch (all-headers=true), a VP8 keyframe on the
/// compat branch. Pushed upstream from THAT producer's appsink sink pad —
/// the same path StreamProducer uses to forward browser PLIs — so a compat
/// consumer's request reaches encoder_compat, not the default encoder.
/// REQUIRED for consumer join with GOP=240: without it a new consumer waits
/// up to 8s/12s for the next scheduled keyframe.
pub(super) fn request_keyframe(producer: &StreamProducer) {
    let event = gst_video::UpstreamForceKeyUnitEvent::builder()
        .all_headers(true)
        .build();
    if let Some(pad) = producer.appsink().static_pad("sink") {
        if !pad.push_event(event) {
            tracing::warn!("force-keyunit event was not handled upstream");
        }
    }
}

/// Pull RTCP receiver-report stats (the browser's view of the link) from a
/// webrtcbin via its `get-stats` signal. Returns (rtt_ms, jitter_ms,
/// packets_lost); all None when no RTCP has arrived yet (e.g. pre-connect)
/// or the promise times out (500ms bound).
fn rtcp_remote_inbound(webrtcbin: &gst::Element) -> (Option<f64>, Option<f64>, Option<i64>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let promise = gst::Promise::with_change_func(move |reply| {
        if let Ok(Some(stats)) = reply {
            let _ = tx.send(stats.to_owned());
        }
    });
    webrtcbin.emit_by_name::<()>("get-stats", &[&None::<gst::Pad>, &promise]);
    let Ok(stats) = rx.recv_timeout(std::time::Duration::from_millis(500)) else {
        return (None, None, None);
    };
    for (_field, value) in stats.iter() {
        let Ok(s) = value.get::<gst::Structure>() else {
            continue;
        };
        // The remote-inbound (RTCP RR) stats structure is the one carrying
        // round-trip-time; reading by field presence keeps this robust across
        // GStreamer versions.
        if s.has_field("round-trip-time") {
            let rtt = s.get::<f64>("round-trip-time").ok().map(|v| v * 1000.0);
            let jitter = s.get::<f64>("jitter").ok().map(|v| v * 1000.0);
            let lost = s
                .get::<i64>("packets-lost")
                .ok()
                .or_else(|| s.get::<u64>("packets-lost").ok().map(|v| v as i64))
                .or_else(|| s.get::<i32>("packets-lost").ok().map(i64::from));
            return (rtt, jitter, lost);
        }
    }
    (None, None, None)
}
