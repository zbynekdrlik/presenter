//! WHEP HTTP bridge + pipeline state snapshots. Translates the WHEP
//! signaller protocol (`WhepOp` → `WhepReply`) into direct `NdiPipeline`
//! `add_consumer` / `add_ice_candidate` / `remove_consumer` calls, and
//! exposes the `/healthz` + `/ndi/snapshot/:id` snapshot helpers. Split out
//! of the manager god-file (#357).

use anyhow::{anyhow, Result};

use crate::pipeline::{NdiPipeline, PipelineState, StreamProfile};

use super::{ActiveSource, NdiManager, WhepOp, WhepReply, SOURCE_NOT_ACTIVE_ERR};

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
    pub async fn pipeline_snapshots(&self) -> Vec<(String, PipelineState)> {
        match tokio::time::timeout(std::time::Duration::from_millis(200), self.active.lock()).await
        {
            Ok(guard) => guard
                .iter()
                .map(|(id, src)| (id.clone(), src.pipeline.state()))
                .collect(),
            Err(_) => {
                tracing::warn!(
                    "pipeline_snapshots lock acquisition timed out after 200 ms — \
                     likely contended with a long-running pipeline start/rebuild; \
                     returning empty snapshot so /healthz does not stall (#333 item 7)"
                );
                Vec::new()
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
            } => self.whep_post(source_id, body, profile).await,
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
            .ok_or_else(|| anyhow!(SOURCE_NOT_ACTIVE_ERR))?;
        Self::ensure_streaming(src)?;
        Ok(std::sync::Arc::clone(&src.pipeline))
    }

    /// WHEP POST (new consumer): SDP offer in, 201 + SDP answer + Location
    /// out. `profile` selects the encode branch (default 720p H264 / compat
    /// 854×480 realtime VP8) that feeds the new consumer.
    async fn whep_post(
        &self,
        source_id: &str,
        body: Vec<u8>,
        profile: StreamProfile,
    ) -> Result<WhepReply> {
        let pipeline = self.streaming_pipeline(source_id).await?;
        let answer = pipeline.add_consumer(body, profile).await?;
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
                .ok_or_else(|| anyhow!(SOURCE_NOT_ACTIVE_ERR))?;
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
