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

use super::adaptive::{BitrateDecision, CompatBitrateController, START_BPS};
use super::negotiation::{
    align_payload_type, await_ice_gathering, await_media_caps, negotiate_sdp,
    parse_h264_payload_type, parse_vp8_payload_type,
};
use super::{
    build::{compat_raw_caps, consumer_h264_caps},
    AddConsumerError, NdiPipeline, PipelineSnapshot, SessionSnapshot, StreamProfile, WhepAnswer,
};
use crate::whep_session::{IceCandidate, LivenessState, WhepConnectionState, WhepSession};

/// How often the per-consumer compat AIMD loop samples RTCP and steps the
/// controller (#387). 1.5 s matches the controller's anti-thrash cadence
/// (10 s increase interval, 5 s post-decrease cooldown) while reacting to a
/// loss spike within a couple of ticks.
const CONTROLLER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

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
    /// Per-consumer adaptive-bitrate task (#387) — only set for COMPAT
    /// consumers (which own a vp8enc). Aborted on Drop alongside `bus_task`.
    controller_task: Option<tokio::task::JoinHandle<()>>,
    /// Latest controller-applied target-bitrate (bits/s), shared with the
    /// controller task. `Some` for COMPAT, `None` for DEFAULT. Read by snapshot.
    compat_target_bitrate: Option<Arc<std::sync::atomic::AtomicI32>>,
    link: Option<ConsumptionLink>,
    sdp_answer: String,
    session_id: String,
    armed: bool,
}

/// Everything `defuse` hands to a stored `WhepSession`.
pub(super) struct DefusedBranch {
    pub pipeline: gst::Pipeline,
    pub webrtcbin: gst::Element,
    pub sdp_answer: String,
    pub link: ConsumptionLink,
    pub bus_task: tokio::task::JoinHandle<()>,
    pub controller_task: Option<tokio::task::JoinHandle<()>>,
    pub compat_target_bitrate: Option<Arc<std::sync::atomic::AtomicI32>>,
}

impl ConsumerBranch {
    fn new(pipeline: gst::Pipeline, webrtcbin: gst::Element, session_id: &str) -> Self {
        Self {
            pipeline,
            webrtcbin,
            bus_task: None,
            controller_task: None,
            compat_target_bitrate: None,
            link: None,
            sdp_answer: String::new(),
            session_id: session_id.to_string(),
            armed: true,
        }
    }

    /// Take ownership of the parts for a stored `WhepSession`, disarming the
    /// guard. Returns None if the branch never completed negotiation (no link
    /// or bus task) — callers treat that as an internal error. The
    /// controller task + bitrate handle are COMPAT-only (`None` for DEFAULT).
    fn defuse(mut self) -> Option<DefusedBranch> {
        let link = self.link.take()?;
        let bus_task = self.bus_task.take()?;
        self.armed = false;
        Some(DefusedBranch {
            pipeline: self.pipeline.clone(),
            webrtcbin: self.webrtcbin.clone(),
            sdp_answer: std::mem::take(&mut self.sdp_answer),
            link,
            bus_task,
            controller_task: self.controller_task.take(),
            compat_target_bitrate: self.compat_target_bitrate.take(),
        })
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
        if let Some(task) = self.controller_task.take() {
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
        let defused = branch
            .defuse()
            .ok_or_else(|| anyhow!("negotiated consumer branch missing link or bus task"))?;
        let session = WhepSession {
            session_id: session_id.clone(),
            consumer_pipeline: defused.pipeline,
            webrtcbin: defused.webrtcbin,
            link: defused.link,
            bus_task: defused.bus_task,
            connection_state,
            // Fresh RTCP-liveness tracker: full stale-window grace from now.
            liveness: Arc::new(std::sync::Mutex::new(LivenessState::new())),
            ice_tx,
            compat_controller_task: defused.controller_task,
            compat_target_bitrate: defused.compat_target_bitrate,
        };
        let sdp_answer = defused.sdp_answer;

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
        // Phase 1 (cheap, under the lock): identity + counters + webrtcbin
        // handle + the per-consumer compat target-bitrate (an atomic load —
        // `None` for DEFAULT, `Some(bps)` for COMPAT — #387).
        type SnapPartial = (
            String,
            WhepConnectionState,
            u64,
            u64,
            gst::Element,
            Option<i32>,
        );
        let partial: Vec<SnapPartial> = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(id, session)| {
                    let connection_state = *session
                        .connection_state
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    let compat_bitrate = session
                        .compat_target_bitrate
                        .as_ref()
                        .map(|b| b.load(std::sync::atomic::Ordering::Relaxed));
                    (
                        id.clone(),
                        connection_state,
                        session.link.pushed(),
                        session.link.dropped(),
                        session.webrtcbin.clone(),
                        compat_bitrate,
                    )
                })
                .collect()
        };
        // Phase 2 (blocking): RTCP receiver-report stats per webrtcbin — the
        // get-stats promise wait must NOT block the async thread.
        let session_snaps: Vec<SessionSnapshot> = tokio::task::spawn_blocking(move || {
            partial
                .into_iter()
                .map(
                    |(id, connection_state, pushed, dropped, webrtcbin, compat_bitrate)| {
                        let (rtt, jitter, lost) = rtcp_remote_inbound(&webrtcbin);
                        SessionSnapshot {
                            id,
                            connection_state,
                            buffers_pushed: pushed,
                            buffers_dropped: dropped,
                            rtcp_round_trip_ms: rtt,
                            rtcp_jitter_ms: jitter,
                            rtcp_packets_lost: lost,
                            compat_target_bitrate_bps: compat_bitrate,
                        }
                    },
                )
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
    /// Since #387 the ENCODER pipeline holds EXACTLY ONE encoder — the SHARED
    /// H264 "encoder" (720p default profile). The compat path fans RAW (the
    /// shared vp8enc is gone), and each COMPAT consumer owns its `vp8enc`
    /// (`venc_<session>`) in its OWN pipeline so its bitrate adapts per-TV.
    /// The #336 invariant — the EXPENSIVE H264 720p encoder never multiplies
    /// per consumer — is intact; the bounded per-consumer VP8 480p exception is
    /// affordable on the N100 (≤3 weak TVs). DEFAULT consumer pipelines contain
    /// NO encoder; COMPAT consumer pipelines contain exactly one vp8enc.
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
    let pt = match profile {
        StreamProfile::Default => parse_h264_payload_type(offer_str),
        StreamProfile::Compat => parse_vp8_payload_type(offer_str),
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

    // For COMPAT, `encoder` is the per-consumer vp8enc (#387) that sits
    // between appsrc(RAW) and the payloader; for DEFAULT it is None (the
    // shared H264 producer already fans encoded frames).
    let (appsrc, encoder, payloader, webrtcbin) = build_consumer_elements(session_id, profile, pt)?;

    let consumer_pipeline = gst::Pipeline::with_name(&format!("consumer_{session_id}"));
    adopt_encoder_timeline(&consumer_pipeline, enc_clock, enc_base_time, session_id);
    add_consumer_elements(
        &consumer_pipeline,
        &appsrc,
        encoder.as_ref(),
        &payloader,
        &webrtcbin,
    )?;

    // From here every resource is owned by the guard: any `?` below drops it,
    // aborting the bus + controller tasks (if spawned), disconnecting the
    // producer link (if connected) and setting the pipeline to Null.
    let mut branch = ConsumerBranch::new(consumer_pipeline.clone(), webrtcbin.clone(), session_id);

    // appsrc → [vp8enc →] payloader → webrtcbin. The pay→webrtc link is
    // filtered to the profile codec's application/x-rtp caps (payload OMITTED —
    // re-aligned to the negotiated pt in align_payload_type; pinning it here
    // would fight that).
    link_consumer_elements(
        &appsrc,
        encoder.as_ref(),
        &payloader,
        &webrtcbin,
        encoding_name,
    )?;

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

    // COMPAT only (#387): spawn the per-consumer AIMD bitrate loop driving THIS
    // consumer's vp8enc target-bitrate from its OWN RTCP loss/RTT. DEFAULT
    // consumers (shared H264 encoder) get no controller — their `encoder` is
    // None and the branch's bitrate handle stays None.
    if let Some(encoder) = encoder {
        let bitrate = Arc::new(std::sync::atomic::AtomicI32::new(START_BPS));
        branch.controller_task = Some(spawn_compat_bitrate_controller(
            webrtcbin.clone(),
            encoder,
            bitrate.clone(),
            session_id.to_string(),
            &rt,
        ));
        branch.compat_target_bitrate = Some(bitrate);
    }

    // The answer must announce the send SSRC — wait for media caps first.
    // (await_media_caps keys on the `ssrc` caps field, codec-agnostic.)
    await_media_caps(&webrtcbin, session_id);
    negotiate_sdp(&webrtcbin, &offer_desc)?;
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

/// Add the per-consumer elements to the consumer pipeline: `appsrc`,
/// `[vp8enc]` (COMPAT only — #387), `payloader`, `webrtcbin`.
fn add_consumer_elements(
    pipeline: &gst::Pipeline,
    appsrc: &gst_app::AppSrc,
    encoder: Option<&gst::Element>,
    payloader: &gst::Element,
    webrtcbin: &gst::Element,
) -> Result<()> {
    pipeline
        .add_many([appsrc.upcast_ref::<gst::Element>(), payloader, webrtcbin])
        .context("add appsrc+payloader+webrtcbin to consumer pipeline")?;
    if let Some(encoder) = encoder {
        pipeline
            .add(encoder)
            .context("add per-consumer vp8enc to consumer pipeline")?;
    }
    Ok(())
}

/// Link the per-consumer chain: `appsrc → [vp8enc →] payloader → webrtcbin`.
/// For COMPAT the appsrc carries RAW I420 and the per-consumer `vp8enc`
/// encodes it (#387); for DEFAULT the appsrc carries encoded H264 straight
/// into the payloader. The pay→webrtc link is filtered to the codec's
/// application/x-rtp caps (payload OMITTED — re-aligned later in
/// `align_payload_type`).
fn link_consumer_elements(
    appsrc: &gst_app::AppSrc,
    encoder: Option<&gst::Element>,
    payloader: &gst::Element,
    webrtcbin: &gst::Element,
    encoding_name: &str,
) -> Result<()> {
    let appsrc_el = appsrc.upcast_ref::<gst::Element>();
    match encoder {
        Some(encoder) => {
            gst::Element::link_many([appsrc_el, encoder, payloader])
                .context("link appsrc -> vp8enc -> payloader")?;
        }
        None => {
            appsrc_el
                .link(payloader)
                .context("link appsrc -> payloader")?;
        }
    }
    let rtp_caps = gst::Caps::builder("application/x-rtp")
        .field("media", "video")
        .field("encoding-name", encoding_name)
        .field("clock-rate", 90_000i32)
        .build();
    payloader
        .link_filtered(webrtcbin, &rtp_caps)
        .context("link payloader -> webrtcbin (codec caps)")
}

/// Spawn the per-consumer adaptive-bitrate task (#387): every
/// `CONTROLLER_INTERVAL` it reads this consumer's webrtcbin RTCP
/// remote-inbound stats (the peer's view of OUR stream — packets-lost/-received
/// deltas → loss fraction, round-trip-time), feeds them to the per-consumer
/// `CompatBitrateController` AIMD step, and — when the decision moves — sets
/// the consumer's own `vp8enc` `target-bitrate` LIVE (vp8enc takes bits/s i32;
/// no caps change → NO decoder port-reconfig, so the Vestel OMX is never
/// killed — addendum 2). The latest value is mirrored into `bitrate` so
/// `snapshot` can read it. The task never ends on its own; the explicit abort
/// (by `ConsumerBranch`/`WhepSession` Drop) is its only exit.
///
/// get-stats blocks on a promise, so each sample runs in `spawn_blocking` off
/// the async runtime (never blocking this task). The DECISION logic is unit-
/// tested in `adaptive.rs` (13 tests); this WIRING (stat-read → controller →
/// set target-bitrate) is proven by the lab tc-netem functional check, not a
/// unit test (no live RTCP in unit tests).
fn spawn_compat_bitrate_controller(
    webrtcbin: gst::Element,
    encoder: gst::Element,
    bitrate: Arc<std::sync::atomic::AtomicI32>,
    session_id: String,
    rt: &tokio::runtime::Handle,
) -> tokio::task::JoinHandle<()> {
    rt.spawn(async move {
        let mut controller = CompatBitrateController::new();
        // Mirror the controller's start value (== the vp8enc's start
        // target-bitrate) so the snapshot reflects it before the first tick.
        bitrate.store(
            controller.current_bitrate_bps(),
            std::sync::atomic::Ordering::Relaxed,
        );
        let mut prev = LossSample::default();
        let mut interval = tokio::time::interval(CONTROLLER_INTERVAL);
        // Skip the immediate first tick — wait one interval so RTCP has a chance
        // to arrive before the first observation.
        interval.tick().await;
        loop {
            interval.tick().await;
            let wb = webrtcbin.clone();
            let Ok(sample) = tokio::task::spawn_blocking(move || read_loss_sample(&wb)).await
            else {
                continue; // spawn_blocking join error — try again next tick.
            };
            let (observed_loss, dt) = prev.delta(&sample);
            prev = sample.clone();
            let decision: BitrateDecision = controller.update(observed_loss, sample.rtt_ms, dt);
            tracing::debug!(
                session_id = %session_id,
                observed_loss,
                rtt_ms = sample.rtt_ms,
                target_bitrate_bps = decision.target_bitrate_bps,
                changed = decision.changed,
                "compat AIMD tick"
            );
            if decision.changed {
                // Live, no caps change → no Vestel OMX port-reconfig.
                encoder.set_property("target-bitrate", decision.target_bitrate_bps);
            }
            bitrate.store(
                decision.target_bitrate_bps,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    })
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
/// profile branch's producer appsink — byte-stream/AU H264 for Default, RAW
/// I420 854×480 for Compat), an OPTIONAL per-consumer `vp8enc`
/// (`venc_<session>`) for Compat ONLY (#387 — DEFAULT shares the H264
/// encoder), a per-consumer payloader (`rtph264pay` / `rtpvp8pay`) pre-seated
/// on the offer's dynamic payload type `pt`, and a `webrtcbin`.
///
/// Returns `(appsrc, Option<encoder>, payloader, webrtcbin)`. The encoder is
/// `Some` for COMPAT (its `target-bitrate` is driven per-TV by
/// `CompatBitrateController`) and `None` for DEFAULT.
#[allow(clippy::type_complexity)]
pub(super) fn build_consumer_elements(
    session_id: &str,
    profile: StreamProfile,
    pt: u32,
) -> Result<(
    gst_app::AppSrc,
    Option<gst::Element>,
    gst::Element,
    gst::Element,
)> {
    // Initial caps match the profile branch's producer appsink caps filter so
    // the very first forwarded sample agrees (`consumer_h264_caps` /
    // `compat_raw_caps` pin BOTH sides of each bridge). Compat now carries RAW
    // I420 (the consumer encodes VP8 itself — #387).
    let bridge_caps = match profile {
        StreamProfile::Default => consumer_h264_caps(),
        StreamProfile::Compat => compat_raw_caps(),
    };
    let appsrc = gst_app::AppSrc::builder()
        .name(format!("src_{session_id}"))
        .caps(&bridge_caps)
        .build();
    // CRITICAL ORDER: apply the consumer configuration (is-live=true,
    // format=time, leaky downstream, 500ms queue bound) NOW — BEFORE the
    // pipeline transitions to PLAYING. basesrc latches `live_running` only at
    // the PAUSED→PLAYING transition and ONLY if the source is already live;
    // if `is_live` flips to true afterwards (which is what happened when
    // StreamProducer::add_consumer — which calls this internally — ran after
    // PLAYING), the appsrc's task blocks in "live source waiting for running
    // state" FOREVER and not a single buffer is ever pushed downstream —
    // connected, but black. add_consumer re-applies the same configuration
    // later, which is a harmless no-op. Holds identically for the RAW compat
    // appsrc.
    StreamProducer::configure_consumer(&appsrc);

    // COMPAT: this consumer's OWN vp8enc (#387) so its bitrate adapts per-TV.
    let encoder = match profile {
        StreamProfile::Default => None,
        StreamProfile::Compat => Some(build_compat_vp8_encoder(session_id)?),
    };

    // Per-consumer payloader so each webrtcbin negotiates its own dynamic
    // payload type with its browser (#336); pre-seated on the browser's
    // offered pt for the profile's codec.
    let payloader = match profile {
        StreamProfile::Default => build_h264_payloader(session_id, pt)?,
        StreamProfile::Compat => build_vp8_payloader(session_id, pt)?,
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

    Ok((appsrc, encoder, payloader, webrtcbin))
}

/// Build ONE compat consumer's `vp8enc` ("venc_<session>") with libwebrtc-
/// parity realtime tuning (#387). Each weak TV gets its OWN encoder so its
/// `target-bitrate` is driven independently by that consumer's
/// `CompatBitrateController` from its OWN RTCP loss/RTT — a SHARED encoder
/// could only serve the worst TV's bitrate to all of them. STARTS HIGH at
/// `START_BPS` (900k) per the quality policy (reduce only on measured loss).
/// Property types verified via gst-inspect-1.0 and locked by
/// `compat_consumer_has_per_consumer_adaptive_vp8enc`:
///
/// - `deadline=1` (µs/frame, Integer64): libvpx realtime mode.
/// - `cpu-used=8`: fastest realtime encode preset (quality ↓, speed ↑).
/// - `end-usage=cbr` + `target-bitrate=START_BPS` (bits/sec): constant-bitrate
///   like libwebrtc's rate controller; the controller sets it live thereafter.
/// - `keyframe-max-dist=240`: GOP parity with the H264 branch; joins are
///   served by `request_keyframe`, not scheduled keyframe pulses.
/// - `token-partitions="4"`: partitioned token coding lets the TV's libvpx
///   decoder spread entropy decode across its 4 cores (the VDO.Ninja delta).
/// - `threads=4`: one encode thread per partition on the server.
/// - `error-resilient=default` (flags): frames stay decodable after loss.
/// - `lag-in-frames=0`: zero lookahead — no encoder-side frame delay.
fn build_compat_vp8_encoder(session_id: &str) -> Result<gst::Element> {
    gst::ElementFactory::make("vp8enc")
        .name(format!("venc_{session_id}"))
        .property("deadline", 1i64)
        .property("cpu-used", 8i32)
        .property_from_str("end-usage", "cbr")
        .property("target-bitrate", START_BPS)
        .property("keyframe-max-dist", 240i32)
        .property_from_str("token-partitions", "4")
        .property("threads", 4i32)
        .property_from_str("error-resilient", "default")
        .property("lag-in-frames", 0i32)
        .build()
        .with_context(|| format!("build per-consumer vp8enc venc_{session_id}"))
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

/// Ask the producer's branch for an immediate keyframe — an IDR with SPS/PPS
/// on the H264 (default) branch (all-headers=true). Pushed upstream from THAT
/// producer's appsink sink pad — the same path StreamProducer uses to forward
/// browser PLIs — so a compat consumer's request reaches the compat RAW
/// producer (whose upstream is the raw conditioning chain). On the compat path
/// the per-consumer vp8enc also re-emits a keyframe when its consumer joins
/// (kf-max-dist=240). REQUIRED for consumer join with GOP=240: without it a
/// new consumer waits up to 8s/12s for the next scheduled keyframe.
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

/// One RTCP-loss observation for the per-consumer AIMD loop (#387): the peer's
/// cumulative packets-lost, the count of packets the peer has acknowledged
/// (extended-highest-seq from the RR block — a proxy for packets RECEIVED+LOST
/// the peer has seen), the current RTT (ms), and a monotonic capture time so
/// the loop derives a real `dt`. Cumulative counters → the controller wants a
/// per-interval FRACTION, so [`LossSample::delta`] differences two samples.
#[derive(Clone)]
struct LossSample {
    /// Cumulative packets the peer reported lost (`remote-inbound-rtp`
    /// `packets-lost`), or the RR block's `rb-packetslost` fallback.
    packets_lost: i64,
    /// Extended highest sequence number the peer has received
    /// (`rb-exthighestseq`) — advances by the count of packets the peer has
    /// seen (received + lost) since the stream start.
    ext_highest_seq: u64,
    /// Current round-trip-time (ms); 0.0 when no RR is present yet.
    rtt_ms: f64,
    /// When this sample was captured (for the controller's `dt_secs`).
    captured: std::time::Instant,
}

impl Default for LossSample {
    fn default() -> Self {
        Self {
            packets_lost: 0,
            ext_highest_seq: 0,
            rtt_ms: 0.0,
            captured: std::time::Instant::now(),
        }
    }
}

impl LossSample {
    /// Loss FRACTION and elapsed seconds between `self` (older) and `next`
    /// (newer). `observed_loss = lost_delta / max(1, seq_delta)`, clamped to
    /// `0.0..=1.0` (the denominator is the packets the peer saw in the window:
    /// `ext_highest_seq` advances by received+lost). A non-advancing or
    /// regressing sequence (reconnect, stats reset, or no fresh RR) yields a
    /// clean 0.0 observation — never a spurious decrease.
    fn delta(&self, next: &LossSample) -> (f64, f64) {
        let dt = next
            .captured
            .saturating_duration_since(self.captured)
            .as_secs_f64()
            .max(0.001);
        let lost_delta = (next.packets_lost - self.packets_lost).max(0);
        let seq_delta = next.ext_highest_seq.saturating_sub(self.ext_highest_seq);
        let observed_loss = if seq_delta == 0 {
            0.0
        } else {
            (lost_delta as f64 / seq_delta as f64).clamp(0.0, 1.0)
        };
        (observed_loss, dt)
    }
}

/// Read one [`LossSample`] from a webrtcbin's RTCP remote-inbound stats (#387).
/// Reuses the `get-stats` promise idiom of [`rtcp_remote_inbound`] /
/// `reaper::peer_rr_fingerprint`: the peer's RR carries `round-trip-time` +
/// `packets-lost` in `remote-inbound-rtp`, and the nested
/// `gst-rtpsource-stats` RR block carries `rb-exthighestseq` (packets the peer
/// has seen) + `rb-packetslost`. Returns a default (zero) sample when no RR has
/// arrived yet or the promise times out — a clean observation, so a
/// not-yet-connected consumer never triggers a decrease.
fn read_loss_sample(webrtcbin: &gst::Element) -> LossSample {
    let captured = std::time::Instant::now();
    let (tx, rx) = std::sync::mpsc::channel();
    let promise = gst::Promise::with_change_func(move |reply| {
        if let Ok(Some(stats)) = reply {
            let _ = tx.send(stats.to_owned());
        }
    });
    webrtcbin.emit_by_name::<()>("get-stats", &[&None::<gst::Pad>, &promise]);
    let Ok(stats) = rx.recv_timeout(std::time::Duration::from_millis(500)) else {
        return LossSample {
            captured,
            ..Default::default()
        };
    };
    let mut sample = LossSample {
        captured,
        ..Default::default()
    };
    for (_field, value) in stats.iter() {
        let Ok(s) = value.get::<gst::Structure>() else {
            continue;
        };
        if s.has_field("round-trip-time") {
            if let Ok(rtt) = s.get::<f64>("round-trip-time") {
                sample.rtt_ms = rtt * 1000.0;
            }
            if let Some(lost) = stats_i64(&s, "packets-lost") {
                sample.packets_lost = lost;
            }
        }
        if let Ok(nested) = s.get::<gst::Structure>("gst-rtpsource-stats") {
            if nested.get::<bool>("have-rb").unwrap_or(false) {
                if let Some(seq) = stats_u64(&nested, "rb-exthighestseq") {
                    sample.ext_highest_seq = seq;
                }
                // Prefer the RR block's packets-lost when the top-level field
                // is absent on this GStreamer version.
                if sample.packets_lost == 0 {
                    if let Some(lost) = stats_i64(&nested, "rb-packetslost") {
                        sample.packets_lost = lost;
                    }
                }
            }
        }
    }
    sample
}

/// Read a numeric stats field as i64, tolerating i64/u64/i32/u32 across
/// GStreamer versions (webrtcbin uses different reprs for different counters).
fn stats_i64(s: &gst::Structure, field: &str) -> Option<i64> {
    s.get::<i64>(field)
        .ok()
        .or_else(|| s.get::<u64>(field).ok().map(|v| v as i64))
        .or_else(|| s.get::<i32>(field).ok().map(i64::from))
        .or_else(|| s.get::<u32>(field).ok().map(i64::from))
}

/// Read a numeric stats field as u64, tolerating the u64/i64/u32 representation
/// webrtcbin uses across GStreamer versions (mirrors `reaper::stats_u64`).
fn stats_u64(s: &gst::Structure, field: &str) -> Option<u64> {
    s.get::<u64>(field)
        .ok()
        .or_else(|| s.get::<i64>(field).ok().map(|v| v.max(0) as u64))
        .or_else(|| s.get::<u32>(field).ok().map(u64::from))
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
