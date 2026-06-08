//! WHEP consumer management: add/remove a per-consumer `webrtcbin` branch off
//! the shared encoder's tee, forward trickle ICE, and diagnostic snapshots.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_webrtc as gst_webrtc;

use super::{
    AddConsumerError, NdiPipeline, PipelineSnapshot, SessionSnapshot, WhepAnswer,
    MAX_CONSUMERS_PER_SOURCE,
};
use crate::whep_session::{IceCandidate, WhepConnectionState, WhepSession};

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
    /// runs inside `tokio::task::spawn_blocking`.
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
        // Returns (webrtcbin, queue, payloader, tee_pad, sdp_text) on success.
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<
            Result<(gst::Element, gst::Element, gst::Element, gst::Pad, String)>,
        >();

        let session_id_for_blocking = session_id.clone();

        tokio::task::spawn_blocking(move || {
            // Parse the SDP offer.
            let sdp_msg = gstreamer_webrtc::gst_sdp::SDPMessage::parse_buffer(&sdp_offer_bytes)
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
            // The closure returns (queue, payloader, webrtcbin, sdp_text) on success.
            let inner_result =
                (|| -> Result<(gst::Element, gst::Element, gst::Element, String)> {
                    // Build queue + per-consumer rtph264pay + webrtcbin.
                    //
                    // The payloader is PER-CONSUMER (downstream of the shared tee)
                    // so each rtph264pay adopts the dynamic H264 payload type its
                    // browser negotiated with webrtcbin (propagated upstream via
                    // caps). A single shared payloader emits one fixed pt and
                    // produces a caps mismatch on the queue→webrtcbin link for any
                    // browser that picks a different pt (Chrome uses e.g. 103) →
                    // zero RTP forwarded → connected but black screen.
                    let queue = gst::ElementFactory::make("queue")
                        .build()
                        .context("build queue")?;
                    let payloader = gst::ElementFactory::make("rtph264pay")
                        .name(format!("pay_{session_id_for_blocking}"))
                        // Resend SPS/PPS with every IDR for fast browser recovery.
                        .property("config-interval", -1i32)
                        .build()
                        .context("build rtph264pay")?;
                    let webrtcbin = gst::ElementFactory::make("webrtcbin")
                        .name(session_id_for_blocking.as_str())
                        // max-bundle: put audio + video on ONE ICE/DTLS transport,
                        // matching the browser's `a=group:BUNDLE` offer. With the
                        // default `none` policy webrtcbin negotiates a SEPARATE
                        // transport per m-line; the browser (bundled) only services
                        // one, so the second DTLS handshake retransmits ClientHello
                        // forever and the RTCPeerConnection never leaves
                        // `connectionState: "connecting"` — no SRTP, no frames,
                        // black screen. Confirmed via dtlsconnection logs: video
                        // DTLS completed, audio DTLS hung, until bundling.
                        .property_from_str("bundle-policy", "max-bundle")
                        .build()
                        .context("build webrtcbin")?;

                    pipeline
                        .add_many([&queue, &payloader, &webrtcbin])
                        .context("add queue+payloader+webrtcbin")?;

                    // From here, on any error we must remove the elements we added
                    // from the pipeline before returning. Use a nested closure.
                    let pipeline_result: Result<String> = (|| -> Result<String> {
                        // Link the consumer branch downstream-first: queue → rtph264pay
                        // → webrtcbin. The tee_src → queue link is deferred until
                        // AFTER the branch is PLAYING (post-negotiation) — linking a
                        // live tee into a not-yet-running branch leaves the new tee
                        // src pad in a state where it never pushes, so the payloader
                        // gets zero buffers (connected, but black screen).
                        queue.link(&payloader).context("link queue -> rtph264pay")?;
                        payloader
                            .link(&webrtcbin)
                            .context("link rtph264pay -> webrtcbin")?;

                        // Connect on-ice-candidate signal.
                        // Signal signature: void(webrtcbin, sdp_mline_index: u32, candidate: &str)
                        // Confirmed from gst-inspect-1.0 webrtcbin and gst-plugin-webrtc-0.15.2/imp.rs:3211
                        // Args array: [webrtcbin_value, mline_index_value, candidate_value]
                        {
                            let ice_tx = ice_tx_clone.clone();
                            webrtcbin.connect("on-ice-candidate", false, move |args| {
                                let sdp_mline_index =
                                    args.get(1).and_then(|v| v.get::<u32>().ok()).unwrap_or(0);
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
                        webrtcbin.connect_notify(Some("connection-state"), {
                            let connection_state_for_signal = connection_state_for_signal.clone();
                            let session_id_for_signal = session_id_for_signal.clone();
                            move |webrtcbin, _pspec| {
                                let gst_state = webrtcbin
                                    .property::<gst_webrtc::WebRTCPeerConnectionState>(
                                        "connection-state",
                                    );
                                let our_state = WhepConnectionState::from(gst_state);
                                // std::sync::Mutex — safe to lock from any thread, no async runtime
                                // required. On poison we recover the guard via into_inner() rather
                                // than panic; the signal thread only does a trivial enum write.
                                *connection_state_for_signal
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner()) = our_state;
                                tracing::debug!(
                                    session_id = %session_id_for_signal,
                                    state = ?our_state,
                                    "WHEP consumer connection-state changed"
                                );
                            }
                        });

                        // Set webrtcbin to PLAYING so it can do ICE + DTLS.
                        // NOTE: the upstream branch (queue + rtph264pay) is started
                        // LATER, AFTER set-local-description — see below. Pushing RTP
                        // into webrtcbin before the remote offer is applied corrupts
                        // negotiation (webrtcbin builds a transceiver from the early
                        // media caps) and DTLS then never completes.
                        webrtcbin
                            .set_state(gst::State::Playing)
                            .context("set webrtcbin to Playing")?;

                        // Set the remote description (the browser's offer).
                        let (remote_desc_tx, remote_desc_rx) =
                            std::sync::mpsc::sync_channel::<()>(1);
                        let promise = gst::Promise::with_change_func(move |_reply| {
                            let _ = remote_desc_tx.send(());
                        });
                        webrtcbin
                            .emit_by_name::<()>("set-remote-description", &[&offer_desc, &promise]);
                        // Wait for the set-remote-description promise to resolve.
                        // Propagate timeout as an error — proceeding to create-answer on a
                        // webrtcbin that hasn't processed the offer produces an invalid SDP.
                        remote_desc_rx
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .context(
                                "set-remote-description promise timed out or sender dropped",
                            )?;

                        // Create the SDP answer.
                        let (answer_sdp_tx, answer_sdp_rx) = std::sync::mpsc::sync_channel::<
                            Option<gst_webrtc::WebRTCSessionDescription>,
                        >(1);
                        let promise = gst::Promise::with_change_func(move |reply| {
                            let answer = reply
                                .ok()
                                .and_then(|r| r)
                                .and_then(|r| r.value("answer").ok())
                                .and_then(|v| v.get::<gst_webrtc::WebRTCSessionDescription>().ok());
                            let _ = answer_sdp_tx.send(answer);
                        });
                        webrtcbin.emit_by_name::<()>(
                            "create-answer",
                            &[&None::<gst::Structure>, &promise],
                        );

                        let answer_desc = answer_sdp_rx
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .context("create-answer promise timed out")?
                            .ok_or_else(|| anyhow!("create-answer returned no answer"))?;

                        // Set the local description (our answer). Use a promise so we
                        // know it has been applied before we wait on ICE gathering.
                        let (sld_tx, sld_rx) = std::sync::mpsc::sync_channel::<()>(1);
                        let sld_promise = gst::Promise::with_change_func(move |_reply| {
                            let _ = sld_tx.try_send(());
                        });
                        webrtcbin.emit_by_name::<()>(
                            "set-local-description",
                            &[&answer_desc, &sld_promise],
                        );
                        if sld_rx
                            .recv_timeout(std::time::Duration::from_secs(2))
                            .is_err()
                        {
                            // Surface a slow apply: the payload-type read below
                            // depends on the local-description being in place. The
                            // final WHEP answer is re-read after the ICE-gather
                            // wait, so this is observability, not a hard failure.
                            tracing::warn!(
                                session_id = %session_id_for_blocking,
                                "set-local-description did not confirm within 2s; \
                                 payload-type alignment may read a stale SDP"
                            );
                        }

                        // Align the payloader's RTP payload type with the one
                        // webrtcbin negotiated in the answer. The browser assigns a
                        // dynamic PT to H264 (Chrome uses e.g. 103) and webrtcbin
                        // answers with THAT pt — but rtph264pay defaults to pt=96, so
                        // its caps don't match webrtcbin's negotiated sink and ZERO
                        // RTP flows (connected, but black screen). Read the answer's
                        // H264 pt and set it on the payloader before media starts.
                        {
                            let local_sdp = webrtcbin
                                .property::<gst_webrtc::WebRTCSessionDescription>(
                                    "local-description",
                                )
                                .sdp()
                                .as_text()
                                .unwrap_or_default();
                            if let Some(pt) = parse_h264_payload_type(&local_sdp) {
                                payloader.set_property("pt", pt);
                                tracing::debug!(
                                    session_id = %session_id_for_blocking,
                                    pt,
                                    "aligned rtph264pay pt to negotiated H264 payload type"
                                );
                            } else {
                                tracing::warn!(
                                    session_id = %session_id_for_blocking,
                                    "could not find negotiated H264 payload type in answer SDP; \
                                     leaving rtph264pay at default pt (media may not flow)"
                                );
                            }
                        }

                        // Non-trickle ICE: wait for candidate gathering to COMPLETE so
                        // the returned local-description SDP carries `a=candidate`
                        // lines. The deployment is LAN-only (host candidates, no
                        // STUN/TURN), so gathering completes in well under a second.
                        //
                        // Without this the WHEP answer contained ZERO candidates and
                        // the browser's ICE agent stayed in "new" forever -> no DTLS
                        // -> no SRTP -> no decoded frames -> black/white stage screen.
                        // This is the load-bearing half of the #336 regression fix
                        // (the browser half waits for its own gathering before POST).
                        {
                            let (gather_tx, gather_rx) = std::sync::mpsc::sync_channel::<()>(1);
                            let gather_tx_signal = gather_tx.clone();
                            // SignalHandlerId is intentionally not stored: dropping it
                            // does not disconnect, and webrtcbin is torn down whole on
                            // remove_consumer. Once we stop reading gather_rx the
                            // closure's try_send becomes a harmless no-op.
                            let _ = webrtcbin.connect_notify(
                                Some("ice-gathering-state"),
                                move |wb, _| {
                                    let st = wb.property::<gst_webrtc::WebRTCICEGatheringState>(
                                        "ice-gathering-state",
                                    );
                                    if st == gst_webrtc::WebRTCICEGatheringState::Complete {
                                        let _ = gather_tx_signal.try_send(());
                                    }
                                },
                            );
                            // Cover the race where gathering already completed before
                            // the notify handler was connected.
                            if webrtcbin.property::<gst_webrtc::WebRTCICEGatheringState>(
                                "ice-gathering-state",
                            ) == gst_webrtc::WebRTCICEGatheringState::Complete
                            {
                                let _ = gather_tx.try_send(());
                            }
                            if gather_rx
                                .recv_timeout(std::time::Duration::from_secs(5))
                                .is_err()
                            {
                                tracing::warn!(
                                    session_id = %session_id_for_blocking,
                                    "ICE gathering did not reach Complete within 5s; \
                                     returning answer with whatever candidates were gathered"
                                );
                            }
                        }

                        // Negotiation is complete. NOW start the upstream branch so
                        // H264 buffers flow tee → queue → rtph264pay → webrtcbin.
                        // Elements added to an already-PLAYING pipeline stay in NULL
                        // until synced; webrtcbin started itself above, but queue +
                        // rtph264pay must be synced or the send path never runs
                        // (connected, but zero RTP → black screen). Starting them
                        // here (post-negotiation) avoids the early-media corruption
                        // that breaks DTLS when started before set-remote-description.
                        queue
                            .sync_state_with_parent()
                            .context("sync queue state with pipeline")?;
                        payloader
                            .sync_state_with_parent()
                            .context("sync rtph264pay state with pipeline")?;

                        // Branch is PLAYING — NOW splice it into the live tee. The
                        // tee starts pushing H264 to this consumer's queue at the
                        // next buffer; the encoder's ~1s keyframe interval (GOP 30
                        // — nvh264enc gop-size / vah264enc|x264enc key-int-max)
                        // plus h264parse config-interval=-1 means the browser gets
                        // a decodable keyframe within ~1s.
                        let queue_sink = queue
                            .static_pad("sink")
                            .ok_or_else(|| anyhow!("queue has no sink pad"))?;
                        tee_pad
                            .link(&queue_sink)
                            .context("link tee_pad -> queue.sink")?;

                        // Return the FINAL local description — now populated with the
                        // gathered ICE candidates — as the WHEP answer body.
                        let local_desc = webrtcbin
                            .property::<gst_webrtc::WebRTCSessionDescription>("local-description");
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
                    answer_tx
                        .send(Ok((webrtcbin, queue, payloader, tee_pad, sdp_text)))
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
            // Already Null via WhepSession::Drop, but idempotent.
            let _ = webrtcbin.set_state(gst::State::Null);
            let _ = payloader.set_state(gst::State::Null);
            let _ = queue.set_state(gst::State::Null);
            // Remove in upstream→downstream order: queue → rtph264pay → webrtcbin.
            // Capture results WITHOUT `?` and release the tee request-pad
            // UNCONDITIONALLY afterwards: the session is already gone from the
            // map, so an early `?` return would orphan the tee src pad forever
            // (teardown() only walks live sessions). Release first, report the
            // first error second.
            let r_queue = pipeline.remove(&queue);
            let r_pay = pipeline.remove(&payloader);
            let r_webrtc = pipeline.remove(&webrtcbin);
            tee.release_request_pad(&tee_pad);
            r_queue.context("pipeline.remove queue")?;
            r_pay.context("pipeline.remove rtph264pay")?;
            r_webrtc.context("pipeline.remove webrtcbin")?;
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
