//! WHEP HTTP bridge + pipeline state snapshots. Translates the WHEP
//! signaller protocol (`WhepOp` → `WhepReply`) into direct `NdiPipeline`
//! `add_consumer` / `add_ice_candidate` / `remove_consumer` calls, and
//! exposes the `/healthz` + `/ndi/snapshot/:id` snapshot helpers. Split out
//! of the manager god-file (#357).

use anyhow::{anyhow, Result};

use crate::pipeline::{AddConsumerError, NdiPipeline, PipelineState, StreamProfile};

use super::{ActiveSource, NdiManager, NdiSessionError, WhepOp, WhepReply};

impl NdiManager {
    /// Snapshot of every active pipeline's current state.
    ///
    /// Returns one entry per source currently in the active map, as
    /// `(source_id, PipelineState)`. Used by `/healthz` (#333 item 7) so
    /// dashboards can detect activation failures within seconds instead of
    /// inferring from operator-reported 'red error' status.
    ///
    /// Bounded by a 200 ms lock-acquisition timeout (deep-review 🟡 #1):
    /// `start_pipeline` and `rebuild_pipeline` hold the same `active` mutex
    /// for up to 8 s during the caps-wait. Without the timeout, a `/healthz`
    /// request that races a pipeline start would block long enough to
    /// trip a 5 s LB health-check timeout — exactly the failure mode
    /// item 7 was supposed to expose. On timeout we return an empty vec
    /// and log a warning; the caller (LB / dashboard) sees "no pipelines"
    /// for one poll cycle, which is preferable to a hung probe.
    ///
    /// Callers that must not read a timeout as "no pipelines" (the #546 source
    /// status join — an empty map there means "sending nothing", which is a very
    /// different sentence to put in front of an operator) use
    /// [`Self::pipeline_snapshots_checked`] instead.
    pub async fn pipeline_snapshots(&self) -> Vec<(String, PipelineState)> {
        self.pipeline_snapshots_checked().await.unwrap_or_default()
    }

    /// Like [`Self::pipeline_snapshots`], but `None` when the 200 ms lock wait
    /// expired — i.e. "the manager is busy (almost always: building/starting a
    /// pipeline), we could not look", as opposed to `Some(vec![])`, "we looked
    /// and there are no pipelines".
    ///
    /// The distinction is load-bearing for #546: `start_pipeline` holds this same
    /// mutex across its 8 s caps-wait, so during EVERY normal activation a caller
    /// that cannot tell the two apart concludes "active, on the network, no
    /// pipeline" and tells the operator to go fix a sending machine that is fine.
    pub async fn pipeline_snapshots_checked(&self) -> Option<Vec<(String, PipelineState)>> {
        match tokio::time::timeout(std::time::Duration::from_millis(200), self.active.lock()).await
        {
            Ok(guard) => Some(
                guard
                    .iter()
                    .map(|(id, src)| (id.clone(), src.pipeline.state()))
                    .collect(),
            ),
            Err(_) => {
                tracing::warn!(
                    "pipeline_snapshots lock acquisition timed out after 200 ms — \
                     likely contended with a long-running pipeline start/rebuild; \
                     reporting the snapshot as unavailable (#333 item 7, #546)"
                );
                None
            }
        }
    }

    /// Single-source snapshot for `GET /ndi/snapshot/:source_id`. Returns
    /// `None` if the source isn't active in the manager's active map.
    ///
    /// Uses the same 200 ms lock-acquisition timeout pattern as
    /// `pipeline_snapshots` so a `/ndi/snapshot/:id` probe doesn't stall
    /// behind a concurrent pipeline start/rebuild. On timeout returns `None`
    /// (caller maps to 503).
    pub async fn pipeline_snapshot(
        &self,
        source_id: &str,
    ) -> Option<crate::pipeline::PipelineSnapshot> {
        let guard = tokio::time::timeout(std::time::Duration::from_millis(200), self.active.lock())
            .await
            .ok()?;
        let pipeline = std::sync::Arc::clone(&guard.get(source_id)?.pipeline);
        drop(guard);
        let mut snap = pipeline.snapshot().await;
        snap.source_id = source_id.to_string();
        Some(snap)
    }

    /// Test-only: trigger an Errored state on the source's pipeline so
    /// the PipelineSupervisor reacts as it would for a real ndisrc fault.
    /// Returns `true` if the source was active (state injection succeeded),
    /// `false` if not (caller should map to 404).
    #[cfg(feature = "test-helpers")]
    pub async fn simulate_pipeline_error(&self, source_id: &str, msg: &str) -> bool {
        let active = self.active.lock().await;
        match active.get(source_id) {
            Some(src) => {
                src.pipeline.simulate_error_for_test(msg);
                true
            }
            None => false,
        }
    }

    /// Forward a WHEP HTTP exchange to the source's pipeline. Replaces the
    /// pre-#336 `emit_by_name`-on-whepserversink path. Routes each `WhepOp`
    /// variant to the corresponding `NdiPipeline` method.
    ///
    /// The active-map mutex guard is always DROPPED before calling any
    /// potentially-blocking pipeline method (`add_consumer` spawn_blocks for
    /// ~10s, `add_ice_candidate` and `remove_consumer` also spawn_block).
    /// To achieve this without copying the pipeline, `ActiveSource.pipeline`
    /// is an `Arc<NdiPipeline>` — we clone the `Arc` (cheap refcount bump)
    /// inside the lock, drop the guard, then call the pipeline method outside.
    pub async fn whep_signaller_call(&self, source_id: &str, op: WhepOp) -> Result<WhepReply> {
        match op {
            WhepOp::Post {
                id: None,
                body,
                profile,
                turn_server,
            } => self.whep_post(source_id, body, profile, turn_server).await,
            WhepOp::Post { id: Some(_), .. } => self.whep_reoffer(source_id).await,
            WhepOp::Patch {
                id,
                body,
                headers: _,
            } => self.whep_patch(source_id, &id, &body).await,
            WhepOp::Delete { id } => self.whep_delete(source_id, &id).await,
        }
    }

    /// Lock the active map, validate the source is streaming, and clone its
    /// pipeline Arc out of the guard (cheap refcount bump) so blocking
    /// pipeline methods are called WITHOUT the map lock held.
    async fn streaming_pipeline(&self, source_id: &str) -> Result<std::sync::Arc<NdiPipeline>> {
        let active = self.active.lock().await;
        let src = active
            .get(source_id)
            .ok_or(NdiSessionError::SourceNotActive)?;
        Self::ensure_streaming(src)?;
        Ok(std::sync::Arc::clone(&src.pipeline))
    }

    /// WHEP POST (new consumer): SDP offer in, 201 + SDP answer + Location
    /// out. `profile` is parsed from the `?profile=` query but always resolves
    /// to the single shared 720p H264 stream that feeds the new consumer.
    async fn whep_post(
        &self,
        source_id: &str,
        body: Vec<u8>,
        profile: StreamProfile,
        turn_server: Option<String>,
    ) -> Result<WhepReply> {
        let pipeline = self.streaming_pipeline(source_id).await?;
        // `add_consumer` returns the pipeline's OWN typed `AddConsumerError`
        // (its `CapReached` variant + a catch-all `Other(anyhow::Error)`) —
        // translate `CapReached` into the shared `NdiSessionError` HERE, at
        // the one place it crosses into the router-facing `anyhow::Result`,
        // so `ndi_whep.rs` has a single downcast target for every WHEP
        // status decision (#589). `Other` passes through unchanged.
        let answer = pipeline
            .add_consumer(body, profile, turn_server)
            .await
            .map_err(translate_add_consumer_error)?;
        let location = format!("/ndi/whep/{source_id}/{}", answer.session_id);
        tracing::info!(
            source_id = %source_id,
            session_id = %answer.session_id,
            profile = ?profile,
            "WHEP POST → 201"
        );
        Ok(WhepReply {
            status: 201,
            headers: vec![
                ("location".to_string(), location),
                ("content-type".to_string(), "application/sdp".to_string()),
            ],
            body: Some(answer.sdp_answer.into_bytes()),
        })
    }

    /// Session-scoped re-offer — out of scope for #336; 501. Validates the
    /// source first to preserve 404 semantics for unknown sources (the HTTP
    /// shim tests assert this contract).
    async fn whep_reoffer(&self, source_id: &str) -> Result<WhepReply> {
        let _ = self.streaming_pipeline(source_id).await?;
        tracing::warn!(source_id = %source_id, "WHEP session-scoped POST (re-offer) is unsupported");
        Ok(WhepReply {
            status: 501,
            headers: vec![("content-type".to_string(), "text/plain".to_string())],
            body: Some(b"WHEP re-offer unsupported".to_vec()),
        })
    }

    /// WHEP PATCH: parse an `application/trickle-ice-sdpfrag` body — extract
    /// `a=mid:` (mline index) and `a=candidate:` lines — and forward each
    /// candidate to the pipeline.
    async fn whep_patch(&self, source_id: &str, id: &str, body: &[u8]) -> Result<WhepReply> {
        let pipeline = self.streaming_pipeline(source_id).await?;
        let body_str =
            std::str::from_utf8(body).map_err(|e| anyhow!("PATCH body not utf8: {e}"))?;
        let mut count = 0;
        let mut mline_idx: u32 = 0;
        for raw_line in body_str.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("a=mid:") {
                if let Ok(n) = rest.trim().parse::<u32>() {
                    mline_idx = n;
                }
                // Non-integer mid (RFC 8839 allows e.g. "audio") falls
                // through; mline_idx stays at the last valid integer (or 0).
                // Browsers use integer mids in WHEP practice.
            } else if line.starts_with("a=candidate:") {
                // webrtcbin's add-ice-candidate signal accepts the
                // candidate string without the leading "a=" prefix.
                let cand_value = &line[2..];
                pipeline
                    .add_ice_candidate(id, mline_idx, cand_value)
                    .await?;
                count += 1;
            }
        }
        tracing::debug!(
            source_id = %source_id,
            session_id = %id,
            candidate_count = count,
            "WHEP PATCH dispatched"
        );
        Ok(WhepReply {
            status: 204,
            headers: vec![],
            body: None,
        })
    }

    /// WHEP DELETE: tear down the consumer. Proceeds regardless of pipeline
    /// state — teardown must succeed even while the pipeline is erroring, so
    /// `ensure_streaming` is intentionally skipped here.
    async fn whep_delete(&self, source_id: &str, id: &str) -> Result<WhepReply> {
        let pipeline = {
            let active = self.active.lock().await;
            let src = active
                .get(source_id)
                .ok_or(NdiSessionError::SourceNotActive)?;
            std::sync::Arc::clone(&src.pipeline)
            // active lock dropped here
        };
        pipeline.remove_consumer(id).await?;
        tracing::info!(
            source_id = %source_id,
            session_id = %id,
            "WHEP DELETE → consumer removed"
        );
        Ok(WhepReply {
            status: 204,
            headers: vec![],
            body: None,
        })
    }

    /// Pipeline state must be Streaming or Starting for WHEP ops to proceed.
    /// Stopped / Errored produce an error that the HTTP shim maps to 503.
    fn ensure_streaming(src: &ActiveSource) -> Result<()> {
        match src.pipeline.state() {
            PipelineState::Streaming | PipelineState::Starting => Ok(()),
            PipelineState::Stopped => Err(anyhow!("pipeline stopped")),
            PipelineState::Errored(e) => Err(anyhow!("pipeline errored: {e}")),
        }
    }
}

/// Translate the pipeline's OWN typed `AddConsumerError` into the shared
/// router-facing `NdiSessionError` at the ONE place it crosses into
/// `anyhow::Result` (`whep_post`), so `ndi_whep.rs` has a single downcast
/// target for every WHEP status decision (#589). `Other` passes through
/// unchanged.
///
/// Pure + directly unit-tested (no `NdiManager` / libndi needed) so the
/// `CapReached` → `NdiSessionError::ConsumerCapReached` translation seam —
/// which is the ONLY place that mapping happens — is exercised on every
/// host, including CI runners without libndi (#616 Gap A).
fn translate_add_consumer_error(err: AddConsumerError) -> anyhow::Error {
    match err {
        AddConsumerError::CapReached { max } => NdiSessionError::ConsumerCapReached { max }.into(),
        AddConsumerError::Other(err) => err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::MAX_CONSUMERS_PER_SOURCE;

    /// #616 Gap A: `translate_add_consumer_error` is the ONLY place
    /// `AddConsumerError::CapReached` becomes
    /// `NdiSessionError::ConsumerCapReached`. Before this function was
    /// extracted, no test exercised that translation seam — the pipeline
    /// test asserted on `AddConsumerError::CapReached` (BEFORE), the router
    /// test hand-built `NdiSessionError::ConsumerCapReached` (AFTER), and
    /// nothing connected them. This test drives through the translation and
    /// downcasts the result to assert the typed `NdiSessionError` variant
    /// — not just `AddConsumerError`.
    #[test]
    fn cap_reached_translates_to_ndi_session_error() {
        let err = translate_add_consumer_error(AddConsumerError::CapReached {
            max: MAX_CONSUMERS_PER_SOURCE,
        });
        match err.downcast_ref::<NdiSessionError>() {
            Some(NdiSessionError::ConsumerCapReached { max }) => {
                assert_eq!(
                    *max, MAX_CONSUMERS_PER_SOURCE,
                    "cap value must survive the translation",
                );
            }
            other => panic!(
                "CapReached must translate to NdiSessionError::ConsumerCapReached, got: {other:?}"
            ),
        }
    }

    /// `Other(anyhow::Error)` must pass through UNCHANGED — the inner
    /// error is extracted and returned as-is, not wrapped in an
    /// `NdiSessionError` variant. The message text must survive verbatim.
    #[test]
    fn other_error_passes_through_unchanged() {
        let err =
            translate_add_consumer_error(AddConsumerError::Other(anyhow!("signaller emit failed")));
        // Not an NdiSessionError at all — it's the raw inner anyhow.
        assert!(
            err.downcast_ref::<NdiSessionError>().is_none(),
            "Other must NOT be wrapped in an NdiSessionError variant"
        );
        assert!(
            err.to_string().contains("signaller emit failed"),
            "inner error message must survive unchanged, got: {}",
            err
        );
    }
}
