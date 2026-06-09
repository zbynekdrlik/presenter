//! WHEP consumer management: add/remove a per-consumer `webrtcbin` branch off
//! the shared encoder's tee, forward trickle ICE, and diagnostic snapshots.
//!
//! `add_consumer` orchestrates an async wrapper around the non-Send glib work
//! (element creation, linking, SDP negotiation) which runs inside
//! `tokio::task::spawn_blocking`. That blocking work is decomposed into the
//! focused free functions below (`build_branch_elements`, `link_and_splice_branch`,
//! `connect_branch_signals`, `sync_states_and_recalc`, `negotiate_sdp`,
//! `align_payload_type`, `await_ice_gathering`) so each stage stays small and the
//! load-bearing ORDER (build → add → link+splice → signals → sync+recalc →
//! SDP → align pt → await ICE) is explicit in `negotiate_consumer_blocking`.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_webrtc as gst_webrtc;
use tokio::sync::mpsc::UnboundedSender;

use super::{
    AddConsumerError, NdiPipeline, PipelineSnapshot, SessionSnapshot, WhepAnswer,
    MAX_CONSUMERS_PER_SOURCE,
};
use crate::whep_session::{IceCandidate, WhepConnectionState, WhepSession};

/// The product of a successful consumer negotiation: the per-consumer
/// (webrtcbin, queue, payloader) elements, the tee request pad feeding the
/// branch, and the SDP answer text.
type NegotiatedBranch = (gst::Element, gst::Element, gst::Element, gst::Pad, String);

impl NdiPipeline {
    /// Add a WHEP consumer: request a tee src pad, create a webrtcbin
    /// element, link them via a queue, perform SDP offer/answer exchange,
    /// and return a `WhepAnswer` containing the SDP answer + initial ICE
    /// candidates.
    ///
    /// Returns `Err(AddConsumerError::CapReached)` before allocating any
    /// GStreamer resources when the session count is at `MAX_CONSUMERS_PER_SOURCE`.
    ///
    /// Non-Send glib work (element creation, linking, signal connections)
    /// runs inside `tokio::task::spawn_blocking` via
    /// [`negotiate_consumer_blocking`].
    pub async fn add_consumer(
        &self,
        sdp_offer_bytes: Vec<u8>,
    ) -> Result<WhepAnswer, AddConsumerError> {
        // Enforce soft consumer cap BEFORE allocating any GStreamer resources.
        //
        // Soft cap: this check + the session insert below are NOT atomic.
        // Two concurrent add_consumer calls at count=MAX-1 can both pass and
        // momentarily reach count=MAX+1. Acceptable per spec ("soft cap, not a
        // hard atomic invariant") — N100 saturation is gradual, not a cliff.
        {
            let sessions = self.sessions.lock().await;
            if sessions.len() >= MAX_CONSUMERS_PER_SOURCE {
                tracing::warn!(
                    current_count = sessions.len(),
                    max = MAX_CONSUMERS_PER_SOURCE,
                    "WHEP consumer cap reached — rejecting new POST"
                );
                return Err(AddConsumerError::CapReached {
                    max: MAX_CONSUMERS_PER_SOURCE,
                });
            }
        }

        let session_id = WhepSession::new_session_id();
        let pipeline = self.pipeline.clone();
        let tee = (*self.tee).clone();

        // Channel for ICE candidates emitted by webrtcbin. `ice_tx` is kept for
        // the stored WhepSession; a clone is moved into the signal closure.
        let (ice_tx, mut ice_rx) = tokio::sync::mpsc::unbounded_channel::<IceCandidate>();
        let ice_tx_for_signal = ice_tx.clone();

        // Shared connection state updated by the notify::connection-state handler.
        // Uses std::sync::Mutex because the GStreamer signal fires from raw
        // std::thread (GLib streaming thread) — tokio::sync::Mutex would risk
        // deadlock in that context. `connection_state` is kept for the session;
        // a clone is moved into the signal closure.
        let connection_state = Arc::new(std::sync::Mutex::new(WhepConnectionState::New));
        let connection_state_for_signal = connection_state.clone();

        // Channel to receive the negotiation result from within spawn_blocking.
        // Returns (webrtcbin, queue, payloader, tee_pad, sdp_text) on success.
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<Result<NegotiatedBranch>>();

        let session_id_for_blocking = session_id.clone();

        tokio::task::spawn_blocking(move || {
            negotiate_and_reply(
                pipeline,
                tee,
                sdp_offer_bytes,
                session_id_for_blocking,
                ice_tx_for_signal,
                connection_state_for_signal,
                answer_tx,
            );
        });

        // Await the negotiation result from the blocking task.
        let (webrtcbin, queue, payloader, tee_pad, sdp_answer) = answer_rx
            .await
            .context("spawn_blocking answer channel dropped")??;

        // Drain any ICE candidates already buffered (half-trickle: include
        // in the WHEP answer body). Allow up to 50 ms for additional
        // candidates to trickle in.
        let mut initial_candidates = Vec::new();
        let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(50);
        while let Ok(Some(c)) = tokio::time::timeout_at(drain_deadline, ice_rx.recv()).await {
            initial_candidates.push(c);
        }

        // Store the session.
        let session = WhepSession {
            session_id: session_id.clone(),
            webrtcbin,
            queue,
            payloader,
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
    ///
    /// NOTE: the deployed WHEP flow is NON-TRICKLE — the browser waits for its
    /// own ICE gathering to complete before POSTing the offer, and the server
    /// returns a full answer with all candidates (see `await_ice_gathering`).
    /// This handler therefore is essentially never exercised in production. A
    /// late trickle candidate arriving before the session is inserted into the
    /// map (the brief window between the blocking task finishing and the
    /// `sessions.insert` in `add_consumer`) returns "session not found"; that
    /// is acceptable because the answer already carried the full candidate set.
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
        let payloader = session.payloader.clone();
        let tee_pad = session.tee_src_pad.clone();
        let remaining_count = sessions.len();
        // Drop the WhepSession so its Drop impl sets the branch elements to
        // Null BEFORE we remove them from the pipeline.
        drop(session);
        drop(sessions);
        tokio::task::spawn_blocking(move || -> Result<()> {
            teardown_branch(&pipeline, &tee, &webrtcbin, &queue, &payloader, &tee_pad)
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
        // Collect session state under the lock, then drop the lock before
        // walking pipeline elements (iterate_encoders does not need the
        // sessions mutex and may take time for large pipelines).
        let session_snaps: Vec<SessionSnapshot> = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(id, session)| {
                    let connection_state = *session
                        .connection_state
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    SessionSnapshot {
                        id: id.clone(),
                        connection_state,
                    }
                })
                .collect()
        };
        // Walk pipeline elements (no session-lock needed).
        let encoders: Vec<gst::Element> = self.iterate_encoders().collect();
        let encoder_count = encoders.len();
        let encoder_factory = encoders
            .into_iter()
            .next()
            .and_then(|el| el.factory().map(|f| f.name().to_string()));
        let consumer_count = session_snaps.len();
        PipelineSnapshot {
            source_id: String::new(), // manager fills this in (Task 8)
            state: format!("{:?}", *self.state_rx.borrow()),
            encoder_factory,
            encoder_count,
            consumer_count,
            sessions: session_snaps,
        }
    }

    /// Iterate encoder elements in the pipeline. Returns a collected Vec
    /// iterator so callers don't need to hold the pipeline lock.
    ///
    /// Note: `nvcudah264enc` and `nvautogpuh264enc` are included here as a
    /// defensive measure — `hw_h264_encoder()` never returns them, but an
    /// external path (e.g. a future plugin or test fixture) could instantiate
    /// one inside the pipeline and we want to count it correctly.
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

/// spawn_blocking body for `add_consumer`: run the negotiation, then deliver the
/// result over `answer_tx`. If the receiver was dropped (the `add_consumer`
/// future was cancelled — e.g. the HTTP client disconnected mid-negotiation),
/// nobody will store the session and nobody will ever tear it down
/// (`teardown()` only walks live sessions), so the fully-built branch is torn
/// down here to avoid leaking it into the live pipeline.
#[allow(clippy::too_many_arguments)]
fn negotiate_and_reply(
    pipeline: gst::Pipeline,
    tee: gst::Element,
    sdp_offer_bytes: Vec<u8>,
    session_id: String,
    ice_tx: UnboundedSender<IceCandidate>,
    connection_state: Arc<std::sync::Mutex<WhepConnectionState>>,
    answer_tx: tokio::sync::oneshot::Sender<Result<NegotiatedBranch>>,
) {
    match negotiate_consumer_blocking(
        &pipeline,
        &tee,
        &sdp_offer_bytes,
        &session_id,
        ice_tx,
        connection_state,
    ) {
        Ok(tuple) => {
            // Err(returned) means the receiver was dropped; `returned` is the
            // Ok(tuple) we tried to send. Collapse both patterns.
            if let Err(Ok((webrtcbin, queue, payloader, tee_pad, _))) = answer_tx.send(Ok(tuple)) {
                let _ = teardown_branch(&pipeline, &tee, &webrtcbin, &queue, &payloader, &tee_pad);
                tracing::warn!(
                    session_id = %session_id,
                    "add_consumer receiver dropped before storing session; \
                     tore down orphaned branch"
                );
            }
        }
        Err(e) => {
            answer_tx.send(Err(e)).ok();
        }
    }
}

/// Run the full per-consumer negotiation on the calling (blocking) thread.
///
/// Returns `(webrtcbin, queue, payloader, tee_pad, sdp_answer)` on success.
/// The load-bearing ORDER is explicit here: request tee pad → build branch
/// elements → add to pipeline → link + splice into the live tee → connect
/// signals → sync states + recalculate latency → SDP offer/answer/SLD → align
/// payload type → await ICE gathering → read final local description.
///
/// All partially-allocated resources are released on any error: pipeline
/// elements are set to Null + removed; the requested tee pad is released.
fn negotiate_consumer_blocking(
    pipeline: &gst::Pipeline,
    tee: &gst::Element,
    sdp_offer_bytes: &[u8],
    session_id: &str,
    ice_tx: UnboundedSender<IceCandidate>,
    connection_state: Arc<std::sync::Mutex<WhepConnectionState>>,
) -> Result<NegotiatedBranch> {
    // Parse the SDP offer.
    let sdp_msg = gstreamer_webrtc::gst_sdp::SDPMessage::parse_buffer(sdp_offer_bytes)
        .map_err(|e| anyhow!("SDP parse failed: {e}"))?;
    let offer_desc =
        gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Offer, sdp_msg);

    // The browser's offer lists the dynamic H264 payload type it will accept
    // (Chrome commonly 102/103/108…). Pre-seat the per-consumer rtph264pay to
    // THAT pt so early-media RTP already matches what webrtcbin will answer;
    // re-confirmed against the answer in `align_payload_type`.
    let offer_h264_pt = std::str::from_utf8(sdp_offer_bytes)
        .ok()
        .and_then(parse_h264_payload_type);
    if offer_h264_pt.is_none() {
        tracing::warn!(
            session_id = %session_id,
            "WHEP offer carries no H264 payload type — rtph264pay left at default \
             pt 96; the browser cannot decode unless it offered H264 (open-source \
             Chromium has no H264; real Chrome/Safari/Edge do)"
        );
    }

    // Step 1: request a tee src pad. On any subsequent failure we MUST release
    // this pad before returning — otherwise it leaks forever.
    let tee_pad = tee
        .request_pad_simple("src_%u")
        .ok_or_else(|| anyhow!("tee has no src_%u request pad template"))?;

    // Step 2: do all remaining work in an inner closure so a single match can
    // unconditionally release tee_pad on Err.
    let inner_result = (|| -> Result<(gst::Element, gst::Element, gst::Element, String)> {
        let (queue, payloader, webrtcbin) = build_branch_elements(session_id, offer_h264_pt)?;

        pipeline
            .add_many([&queue, &payloader, &webrtcbin])
            .context("add queue+payloader+webrtcbin")?;

        // From here, on any error we must remove the elements we added from the
        // pipeline before returning. Use a nested closure.
        let pipeline_result: Result<String> = (|| -> Result<String> {
            link_and_splice_branch(&tee_pad, &queue, &payloader, &webrtcbin)?;
            connect_branch_signals(&webrtcbin, ice_tx, connection_state, session_id.to_string());
            sync_states_and_recalc(pipeline, &queue, &payloader, &webrtcbin, session_id)?;
            negotiate_sdp(&webrtcbin, &offer_desc)?;
            align_payload_type(&webrtcbin, &payloader, session_id);
            await_ice_gathering(&webrtcbin, session_id);

            // The branch was spliced into the tee and brought to PLAYING before
            // negotiation, so H264 buffers are already flowing tee → queue →
            // rtph264pay → webrtcbin. The browser's RTCP PLI pulls a keyframe
            // (webrtcbin forwards it as force-key-unit) so decode starts within
            // ~1 RTT of DTLS. Return the FINAL local description — now populated
            // with the gathered ICE candidates — as the WHEP answer body.
            let local_desc =
                webrtcbin.property::<gst_webrtc::WebRTCSessionDescription>("local-description");
            local_desc
                .sdp()
                .as_text()
                .map_err(|e| anyhow!("local-description SDP as_text failed: {e}"))
        })();

        match pipeline_result {
            Ok(sdp_text) => Ok((queue, payloader, webrtcbin, sdp_text)),
            Err(e) => {
                // Remove partially-allocated pipeline elements before releasing
                // tee_pad (the outer match handles tee_pad release on Err).
                let _ = webrtcbin.set_state(gst::State::Null);
                let _ = payloader.set_state(gst::State::Null);
                let _ = queue.set_state(gst::State::Null);
                let _ = pipeline.remove(&webrtcbin);
                let _ = pipeline.remove(&payloader);
                let _ = pipeline.remove(&queue);
                Err(e)
            }
        }
    })();

    match inner_result {
        Ok((queue, payloader, webrtcbin, sdp_text)) => {
            Ok((webrtcbin, queue, payloader, tee_pad, sdp_text))
        }
        Err(e) => {
            // Release the tee pad we reserved at Step 1.
            tee.release_request_pad(&tee_pad);
            Err(e)
        }
    }
}

/// Build the per-consumer branch elements: a leaky `queue`, a per-consumer
/// `rtph264pay`, and a `webrtcbin`. Not yet added to the pipeline or linked.
fn build_branch_elements(
    session_id: &str,
    offer_h264_pt: Option<u32>,
) -> Result<(gst::Element, gst::Element, gst::Element)> {
    // LEAKY queue: a stalled or dead consumer branch (browser closed without
    // DELETE, frozen tab, network drop) must NOT back-pressure the shared tee
    // and starve the OTHER consumers. `leaky=downstream` drops the oldest
    // buffered frame instead of blocking; bounded to ~1s so a dead branch caps
    // quickly. A live consumer keeps its queue near-empty, so this never drops
    // for it. (If a dropped frame is a keyframe, the browser's RTCP PLI pulls a
    // fresh IDR via webrtcbin's force-key-unit — recovery in ~1 GOP.)
    let queue = gst::ElementFactory::make("queue")
        .property_from_str("leaky", "downstream")
        .property("max-size-time", 1_000_000_000u64)
        .property("max-size-bytes", 0u32)
        .property("max-size-buffers", 0u32)
        .build()
        .context("build queue")?;
    // The payloader is PER-CONSUMER (downstream of the shared tee) so each
    // rtph264pay adopts the dynamic H264 payload type its browser negotiated
    // with webrtcbin. A single shared payloader emits one fixed pt and produces
    // a caps mismatch on the queue→webrtcbin link for any browser that picks a
    // different pt (Chrome uses e.g. 103) → zero RTP forwarded → black screen.
    let payloader = gst::ElementFactory::make("rtph264pay")
        .name(format!("pay_{session_id}"))
        // Resend SPS/PPS with every IDR for fast browser recovery.
        .property("config-interval", -1i32)
        // Pre-seat pt to the browser's offered H264 pt (see above).
        .property("pt", offer_h264_pt.unwrap_or(96))
        .build()
        .context("build rtph264pay")?;
    let webrtcbin = gst::ElementFactory::make("webrtcbin")
        .name(session_id)
        // max-bundle: put audio + video on ONE ICE/DTLS transport, matching the
        // browser's `a=group:BUNDLE` offer. With the default `none` policy
        // webrtcbin negotiates a SEPARATE transport per m-line; the browser
        // (bundled) only services one, so the second DTLS handshake retransmits
        // ClientHello forever and the RTCPeerConnection never leaves
        // `connectionState: "connecting"` — no SRTP, no frames, black screen.
        .property_from_str("bundle-policy", "max-bundle")
        // Explicit, fixed latency so this webrtcbin's internal rtpsession ALWAYS
        // has a configured latency and can compute running time for outgoing RTP
        // — independent of pipeline latency recalculation, which is unreliable
        // for a webrtcbin added to an already-running live pipeline (or re-added
        // after another consumer was removed). Without a configured latency the
        // rtpsession logs "Can't determine running time" and forwards ZERO RTP
        // (#372 black stage for 2nd+/reconnecting consumers). 200 ms is
        // webrtcbin's own default.
        .property("latency", 200u32)
        .build()
        .context("build webrtcbin")?;
    Ok((queue, payloader, webrtcbin))
}

/// Link the consumer branch downstream-first (queue → rtph264pay → webrtcbin,
/// filtered H264 caps) and splice the tee request pad into the queue while
/// everything is still in NULL.
fn link_and_splice_branch(
    tee_pad: &gst::Pad,
    queue: &gst::Element,
    payloader: &gst::Element,
    webrtcbin: &gst::Element,
) -> Result<()> {
    // The rtph264pay→webrtcbin link is FILTERED with explicit
    // application/x-rtp H264 caps so webrtcbin builds its send transceiver from
    // a fixed H264 codec hint — not from whatever early media arrives first
    // (which, with a plain link, builds the transceiver wrong → zero RTP).
    //
    // The filter deliberately OMITS `payload`: the RTP payload type is the one
    // thing re-aligned to the NEGOTIATED pt after create-answer (see
    // `align_payload_type`). Pinning a `payload` here would FIGHT that
    // re-alignment whenever our pre-seat guess differs from webrtcbin's chosen
    // pt → caps mismatch → zero RTP. Leaving payload unconstrained lets the
    // payloader's own `pt` property be the single source of truth.
    queue.link(payloader).context("link queue -> rtph264pay")?;
    let rtp_caps = gst::Caps::builder("application/x-rtp")
        .field("media", "video")
        .field("encoding-name", "H264")
        .field("clock-rate", 90_000i32)
        .build();
    payloader
        .link_filtered(webrtcbin, &rtp_caps)
        .context("link rtph264pay -> webrtcbin (H264 caps)")?;

    // Splice the branch into the live tee NOW, while everything is still in
    // NULL, so the tee's sticky events (stream-start / caps / segment)
    // propagate down the whole branch as it transitions to PLAYING. Linking the
    // tee only AFTER the branch is already PLAYING leaves it without those
    // events and it never forwards a buffer — connected, but BLACK. That was
    // the actual NDI→stage regression. Linking before the PLAYING transition
    // fixes it; the tee then fans out correctly to every consumer.
    let queue_sink = queue
        .static_pad("sink")
        .ok_or_else(|| anyhow!("queue has no sink pad"))?;
    tee_pad
        .link(&queue_sink)
        .context("link tee_pad -> queue.sink")?;
    Ok(())
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
    // Confirmed from gst-inspect-1.0 webrtcbin and gst-plugin-webrtc-0.15.2/imp.rs:3211.
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
    // std::thread spawned by GLib) — NOT from within a tokio context. We use
    // std::sync::Mutex::lock() directly; the critical section is nanoseconds (a
    // simple enum write). On poison we recover the guard via into_inner().
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

/// Bring the branch to PLAYING (inheriting the pipeline's base time via
/// `sync_state_with_parent`) and recalculate pipeline latency so the new
/// webrtcbin's rtpsession learns the live pipeline's configured latency.
fn sync_states_and_recalc(
    pipeline: &gst::Pipeline,
    queue: &gst::Element,
    payloader: &gst::Element,
    webrtcbin: &gst::Element,
    session_id: &str,
) -> Result<()> {
    // webrtcbin MUST use sync_state_with_parent(), NOT set_state(PLAYING):
    // set_state gives a base time of 0, but a consumer added while the pipeline
    // has already been running for N seconds needs the pipeline's (non-zero)
    // base time so its internal rtpsession can compute running time for outgoing
    // RTP. With set_state, the FIRST consumer worked (base time ≈ 0 at startup)
    // but every LATER consumer's rtpsession logged "Can't determine running
    // time …" and forwarded ZERO RTP — connected, but a black stage (#372).
    webrtcbin
        .sync_state_with_parent()
        .context("sync webrtcbin state with pipeline")?;
    queue
        .sync_state_with_parent()
        .context("sync queue state with pipeline")?;
    payloader
        .sync_state_with_parent()
        .context("sync rtph264pay state with pipeline")?;

    // Recalculate pipeline latency so the newly-added webrtcbin's rtpsession
    // learns the LIVE pipeline's configured latency. Without it, a webrtcbin
    // added to an already-running live pipeline has an rtpsession with no
    // configured latency → "Can't determine running time" → ZERO RTP → black
    // stage. Forcing a recalc here configures the new rtpsession AND refreshes
    // the existing ones, so EVERY consumer receives media (#372).
    if let Err(e) = pipeline.recalculate_latency() {
        tracing::warn!(
            session_id = %session_id,
            error = %e,
            "pipeline.recalculate_latency() failed after adding consumer; \
             rtpsession may not forward RTP"
        );
    }
    Ok(())
}

/// Perform the SDP handshake on `webrtcbin`: set-remote-description (the
/// browser's offer), create-answer, set-local-description (our answer). Each
/// step waits on its GStreamer promise; a timeout is propagated as an error
/// (set-local is observability-only — the final answer is re-read later).
fn negotiate_sdp(
    webrtcbin: &gst::Element,
    offer_desc: &gst_webrtc::WebRTCSessionDescription,
) -> Result<()> {
    // Set the remote description (the browser's offer).
    let (remote_desc_tx, remote_desc_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let promise = gst::Promise::with_change_func(move |_reply| {
        let _ = remote_desc_tx.send(());
    });
    webrtcbin.emit_by_name::<()>("set-remote-description", &[offer_desc, &promise]);
    // Proceeding to create-answer on a webrtcbin that hasn't processed the offer
    // produces an invalid SDP, so propagate timeout as an error.
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
    let (sld_tx, sld_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let sld_promise = gst::Promise::with_change_func(move |_reply| {
        let _ = sld_tx.try_send(());
    });
    webrtcbin.emit_by_name::<()>("set-local-description", &[&answer_desc, &sld_promise]);
    if sld_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .is_err()
    {
        // The payload-type read in align_payload_type depends on the
        // local-description being in place; the final WHEP answer is re-read
        // after the ICE-gather wait, so this is observability, not a failure.
        tracing::warn!("set-local-description did not confirm within 2s; payload-type alignment may read a stale SDP");
    }
    Ok(())
}

/// Align the payloader's RTP payload type with the one webrtcbin negotiated in
/// the answer. The browser assigns a dynamic PT to H264 (Chrome uses e.g. 103)
/// and webrtcbin answers with THAT pt — but rtph264pay defaults to pt=96, so
/// its caps don't match webrtcbin's negotiated sink and ZERO RTP flows
/// (connected, but black screen).
fn align_payload_type(webrtcbin: &gst::Element, payloader: &gst::Element, session_id: &str) {
    let local_sdp = webrtcbin
        .property::<gst_webrtc::WebRTCSessionDescription>("local-description")
        .sdp()
        .as_text()
        .unwrap_or_default();
    // parse_h264_payload_type returns the FIRST `a=rtpmap:<pt> H264/...`. For a
    // multi-H264-profile answer the first pt may differ from webrtcbin's chosen
    // send pt, but the rtph264pay→webrtcbin caps filter omits `payload`
    // (see link_and_splice_branch), so a mismatch does not stall media.
    if let Some(pt) = parse_h264_payload_type(&local_sdp) {
        payloader.set_property("pt", pt);
        tracing::debug!(
            session_id = %session_id,
            pt,
            "aligned rtph264pay pt to negotiated H264 payload type"
        );
    } else {
        tracing::warn!(
            session_id = %session_id,
            "could not find negotiated H264 payload type in answer SDP; \
             leaving rtph264pay at default pt (media may not flow)"
        );
    }
}

/// Non-trickle ICE: wait for candidate gathering to COMPLETE so the returned
/// local-description SDP carries `a=candidate` lines. The deployment is LAN-only
/// (host candidates, no STUN/TURN), so gathering completes in well under a
/// second. Without this the WHEP answer had ZERO candidates and the browser's
/// ICE agent stayed "new" forever → no DTLS → no SRTP → no frames → black/white
/// stage (the load-bearing half of the #336 fix). Bounded by a 5 s timeout.
fn await_ice_gathering(webrtcbin: &gst::Element, session_id: &str) {
    let (gather_tx, gather_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let gather_tx_signal = gather_tx.clone();
    // SignalHandlerId intentionally not stored: dropping it does not disconnect,
    // and webrtcbin is torn down whole on remove_consumer. Once we stop reading
    // gather_rx the closure's try_send becomes a harmless no-op.
    let _ = webrtcbin.connect_notify(Some("ice-gathering-state"), move |wb, _| {
        let st = wb.property::<gst_webrtc::WebRTCICEGatheringState>("ice-gathering-state");
        if st == gst_webrtc::WebRTCICEGatheringState::Complete {
            let _ = gather_tx_signal.try_send(());
        }
    });
    // Cover the race where gathering already completed before the notify handler
    // was connected.
    if webrtcbin.property::<gst_webrtc::WebRTCICEGatheringState>("ice-gathering-state")
        == gst_webrtc::WebRTCICEGatheringState::Complete
    {
        let _ = gather_tx.try_send(());
    }
    if gather_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .is_err()
    {
        tracing::warn!(
            session_id = %session_id,
            "ICE gathering did not reach Complete within 5s; \
             returning answer with whatever candidates were gathered"
        );
    }
}

/// Tear down a consumer branch: set its elements to Null, remove them from the
/// pipeline, release the tee request pad, and recalculate pipeline latency.
///
/// The tee request-pad is released UNCONDITIONALLY even if a `remove` errors:
/// the session is already gone from the map, so an early `?` return would
/// orphan the tee src pad forever (teardown() only walks live sessions).
/// Latency is refreshed so the NEXT consumer added afterwards does not inherit
/// the removed consumer's stale latency config → "Can't determine running time"
/// → ZERO RTP (#372: a display reconnecting after another disconnected stayed
/// black). Errors from `pipeline.remove` are reported after the release.
fn teardown_branch(
    pipeline: &gst::Pipeline,
    tee: &gst::Element,
    webrtcbin: &gst::Element,
    queue: &gst::Element,
    payloader: &gst::Element,
    tee_pad: &gst::Pad,
) -> Result<()> {
    // Idempotent — WhepSession::Drop may already have set these to Null.
    let _ = webrtcbin.set_state(gst::State::Null);
    let _ = payloader.set_state(gst::State::Null);
    let _ = queue.set_state(gst::State::Null);
    // Remove in upstream→downstream order: queue → rtph264pay → webrtcbin.
    let r_queue = pipeline.remove(queue);
    let r_pay = pipeline.remove(payloader);
    let r_webrtc = pipeline.remove(webrtcbin);
    tee.release_request_pad(tee_pad);
    let _ = pipeline.recalculate_latency();
    r_queue.context("pipeline.remove queue")?;
    r_pay.context("pipeline.remove rtph264pay")?;
    r_webrtc.context("pipeline.remove webrtcbin")?;
    Ok(())
}

/// Extract the H264 RTP payload type negotiated in an SDP answer.
///
/// Scans for the first `a=rtpmap:<pt> H264/90000` line and returns `<pt>`.
/// WebRTC assigns H264 a *dynamic* payload type (96–127) that varies per
/// browser (Chrome commonly picks 102/103/108…), so the per-consumer
/// `rtph264pay` must be told this value or its RTP caps won't match
/// webrtcbin's negotiated sink and no media flows.
pub(crate) fn parse_h264_payload_type(sdp: &str) -> Option<u32> {
    for line in sdp.lines() {
        let line = line.trim();
        // a=rtpmap:103 H264/90000
        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            let mut parts = rest.splitn(2, ' ');
            let pt = parts.next()?;
            let codec = parts.next().unwrap_or("");
            if codec.to_ascii_uppercase().starts_with("H264/") {
                if let Ok(pt) = pt.trim().parse::<u32>() {
                    return Some(pt);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod pt_parse_tests {
    use super::parse_h264_payload_type;

    #[test]
    fn finds_dynamic_h264_payload_type() {
        let sdp = "v=0\r\n\
                   m=video 9 UDP/TLS/RTP/SAVPF 103 104\r\n\
                   a=rtpmap:103 H264/90000\r\n\
                   a=fmtp:103 packetization-mode=1\r\n\
                   a=rtpmap:104 rtx/90000\r\n";
        assert_eq!(parse_h264_payload_type(sdp), Some(103));
    }

    #[test]
    fn handles_alternate_pt_value() {
        // Different browsers pick different dynamic PTs.
        let sdp = "m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 H264/90000\r\n";
        assert_eq!(parse_h264_payload_type(sdp), Some(96));
    }

    #[test]
    fn returns_none_without_h264() {
        let sdp = "m=video 9 UDP/TLS/RTP/SAVPF 100\r\na=rtpmap:100 VP8/90000\r\n";
        assert_eq!(parse_h264_payload_type(sdp), None);
    }
}
