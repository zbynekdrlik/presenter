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
        // Returns (webrtcbin, queue, tee_pad, sdp_text) on success.
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<
            Result<(gst::Element, gst::Element, gst::Pad, String)>,
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

                pipeline
                    .add_many([&queue, &webrtcbin])
                    .context("add queue+webrtcbin")?;

                // From here, on any error we must remove queue + webrtcbin from the
                // pipeline before returning. Use a nested closure for this.
                let pipeline_result: Result<String> = (|| -> Result<String> {
                    // Link: tee_src → queue → webrtcbin
                    let queue_sink = queue
                        .static_pad("sink")
                        .ok_or_else(|| anyhow!("queue has no sink pad"))?;
                    tee_pad
                        .link(&queue_sink)
                        .context("link tee_pad -> queue.sink")?;
                    queue.link(&webrtcbin).context("link queue -> webrtcbin")?;

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
                    webrtcbin
                        .set_state(gst::State::Playing)
                        .context("set webrtcbin to Playing")?;

                    // Set the remote description (the browser's offer).
                    let (remote_desc_tx, remote_desc_rx) = std::sync::mpsc::sync_channel::<()>(1);
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
                        .context("set-remote-description promise timed out or sender dropped")?;

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
                    webrtcbin
                        .emit_by_name::<()>("create-answer", &[&None::<gst::Structure>, &promise]);

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
        let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(50);
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
            pipeline.remove(&queue).context("pipeline.remove queue")?;
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
