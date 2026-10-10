//! WHEP HTTP bridge + pipeline state snapshots. Translates the WHEP
//! signaller protocol (`WhepOp` → `WhepReply`) into direct `NdiPipeline`
//! `add_consumer` / `add_ice_candidate` / `remove_consumer` calls, and
//! exposes the `/healthz` + `/ndi/snapshot/:id` snapshot helpers. Split out
//! of the manager god-file (#357).

use std::sync::atomic::Ordering;

use anyhow::{anyhow, Result};

use crate::pipeline::{AddConsumerError, NdiPipeline, PipelineState, StreamProfile};

use super::{ActiveSource, NdiManager, NdiSessionError, WhepOp, WhepReply};

/// Whether to emit the `pipeline_snapshots` contention WARN for this 1-based
/// consecutive-timeout `streak`. Logs the first timeout and then only at
/// power-of-two milestones, so a long `start_pipeline`/`rebuild_pipeline` window
/// (which holds `active` for up to 8 s) produces ~log2(N)+1 WARN lines instead
/// of one per status poll — the resolume `should_log_error` idiom (#484), fixing
/// the thousands-of-lines flood this ticket addresses (#736). Pure → unit-tested.
fn should_log_contention(streak: u32) -> bool {
    streak > 0 && streak.is_power_of_two()
}

impl NdiManager {
    /// Snapshot of every active pipeline's state, `(source_id, PipelineState)`
    /// per source in the active map — or `None` when the 200 ms lock wait
    /// expired, i.e. "the manager is busy, we could not look", as opposed to
    /// `Some(vec![])`, "we looked and there are no pipelines". The bound keeps
    /// a status poll from stalling behind a pipeline start/teardown; there is
    /// deliberately no empty-on-timeout variant (it reads as "no pipelines").
    ///
    /// The distinction is load-bearing for #546: a caller that cannot tell the two
    /// apart concludes "active, on the network, no pipeline" and tells the operator
    /// to go fix a sending machine that is fine. Since #741 the `active` lock is no
    /// longer held across the 8 s streaming-ready wait (a `Starting` reservation is
    /// inserted, the lock released, the wait done unlocked), so an activating source
    /// now READS as `Starting` here rather than tripping this timeout — but the
    /// `None` path stays load-bearing for the brief genuine contention that remains
    /// (the reserve/finalize critical sections and `stop_*`'s `pipeline.stop().await`
    /// under the lock).
    pub async fn pipeline_snapshots_checked(&self) -> Option<Vec<(String, PipelineState)>> {
        match tokio::time::timeout(std::time::Duration::from_millis(200), self.active.lock()).await
        {
            Ok(guard) => {
                // Acquired → contention (if any) has cleared; reset the streak so
                // the next contention burst logs fresh from its first timeout.
                self.snapshot_contention_streak.store(0, Ordering::Relaxed);
                Some(
                    guard
                        .iter()
                        .map(|(id, src)| (id.clone(), src.pipeline.state()))
                        .collect(),
                )
            }
            Err(_) => {
                // #736: the WARN used to fire on EVERY 200 ms timeout, so an 8 s
                // `start_pipeline`/`rebuild_pipeline` window (holding `active`)
                // flooded the journal with thousands of identical lines across
                // status polls + /healthz. Gate it on a power-of-two streak so
                // contention stays visible but rare, not routine.
                let streak = self
                    .snapshot_contention_streak
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                if should_log_contention(streak) {
                    tracing::warn!(
                        streak,
                        "pipeline_snapshots lock acquisition timed out after 200 ms — \
                         likely contended with a long-running pipeline start/rebuild; \
                         reporting the snapshot as unavailable (#333 item 7, #546, #736)"
                    );
                }
                None
            }
        }
    }

    /// Single-source snapshot for `GET /ndi/snapshot/:source_id`. Returns
    /// `None` if the source isn't active in the manager's active map.
    ///
    /// Uses the same 200 ms lock-acquisition timeout pattern as
    /// `pipeline_snapshots_checked` so a `/ndi/snapshot/:id` probe doesn't stall
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

    /// Per-pipeline delivery HEALTH for `/healthz.ndi_pipelines[]` (#768):
    /// `(source_id, state, drop_ratio, consumers)` per active source. Clones the
    /// pipeline Arcs out from under the `active` lock (200 ms bounded, like the
    /// other snapshot readers) and computes the cheap delivery totals UNLOCKED,
    /// so it NEVER holds `active` across the per-pipeline sessions-lock await
    /// (the #741 stall). `None` on lock-timeout — `/healthz` must never hang,
    /// and "could not look" must stay distinguishable from "no pipelines": the
    /// stage's last-resort reload guard reads an empty list as "source down"
    /// (same contract as [`Self::pipeline_snapshots_checked`]). Uses the cheap
    /// atomic-only totals, NOT the RTCP get-stats `snapshot()`.
    pub async fn pipeline_health_snapshots(
        &self,
    ) -> Option<Vec<crate::pipeline::health::PipelineDropHealth>> {
        let pipelines = clone_active_sources(&self.active, HEALTH_LOCK_WAIT).await?;
        let mut out = Vec::with_capacity(pipelines.len());
        for (source_id, pipeline) in pipelines {
            let state = pipeline.state();
            let totals = pipeline.consumer_delivery_totals().await;
            out.push(crate::pipeline::health::PipelineDropHealth {
                source_id,
                state,
                drop_ratio: crate::pipeline::health::drop_ratio(totals.pushed, totals.dropped),
                consumers: totals.consumers,
                // #768 D3: current-health trailing-30s aggregate for the watchdog.
                drop_ratio_30s: totals.drop_ratio_30s,
                pushed_fps_30s: totals.pushed_fps_30s,
            });
        }
        Some(out)
    }

    /// Store a client-reported frame-stats sample for one WHEP session (#768
    /// D6). The session id is globally unique (UUID), so it is matched across
    /// every active pipeline without needing the source id in the URL.
    ///
    /// Clones the pipeline Arcs out from under the `active` lock (bounded by
    /// `CLIENT_STATS_LOCK_WAIT`, 2 s — not a probe) and records UNLOCKED, so it
    /// NEVER holds `active` across the per-pipeline sessions-lock await (the
    /// #741 stall). `NdiSessionError::SessionNotFound` when no active pipeline
    /// has the session (unknown/expired → router maps to 404);
    /// `NdiSessionError::Busy` when the lock wait expired (→ 503) — the stage
    /// reporter stops for good on a 404, so a busy manager must never say it.
    pub async fn record_client_stats(
        &self,
        session_id: &str,
        sample: crate::pipeline::client_stats::ClientStatsSample,
    ) -> Result<(), NdiSessionError> {
        let pipelines = clone_active_pipelines(&self.active)
            .await
            .inspect_err(|_| {
                tracing::debug!(
                    session_id,
                    "ndi client-stats: active lock busy (2 s), answering Busy"
                );
            })?;
        for pipeline in pipelines {
            if pipeline.record_client_stats(session_id, sample).await {
                return Ok(());
            }
        }
        Err(NdiSessionError::SessionNotFound {
            session_id: session_id.to_string(),
        })
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

/// Lock budget of the `/healthz` pipeline reader: a readiness probe must never
/// hang behind a pipeline start/teardown.
const HEALTH_LOCK_WAIT: std::time::Duration = std::time::Duration::from_millis(200);

/// Lock budget of a client-stats POST. Not a probe — waiting out a short
/// teardown under the lock beats answering `Busy` (a 503 the stage console
/// logs), and the handler holds nothing while it waits.
const CLIENT_STATS_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

type ActiveMap = tokio::sync::Mutex<std::collections::HashMap<String, ActiveSource>>;

/// `(source_id, pipeline)` for every active source, cloned out under the
/// `active` lock within `wait`, or `None` when that wait expired — "could not
/// look", never "no pipelines". Takes the map directly so the bound is
/// testable without libndi.
async fn clone_active_sources(
    active: &ActiveMap,
    wait: std::time::Duration,
) -> Option<Vec<(String, std::sync::Arc<NdiPipeline>)>> {
    let guard = tokio::time::timeout(wait, active.lock()).await.ok()?;
    Some(
        guard
            .iter()
            .map(|(id, src)| (id.clone(), std::sync::Arc::clone(&src.pipeline)))
            .collect(),
    )
}

/// The active pipelines for a client-stats POST, or `NdiSessionError::Busy`
/// when the lock wait expired. Busy is NOT "session not found": the caller
/// could not look, and the stage reporter stops for good on a 404.
async fn clone_active_pipelines(
    active: &ActiveMap,
) -> Result<Vec<std::sync::Arc<NdiPipeline>>, NdiSessionError> {
    let sources = clone_active_sources(active, CLIENT_STATS_LOCK_WAIT)
        .await
        .ok_or(NdiSessionError::Busy)?;
    Ok(sources.into_iter().map(|(_, pipeline)| pipeline).collect())
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

    #[test]
    fn contention_warn_is_power_of_two_gated() {
        // #736: the WARN used to fire on EVERY 200 ms lock-acquisition timeout,
        // flooding the journal with thousands of identical lines during the 8 s
        // pipeline start/rebuild windows. Now only the 1st timeout + power-of-two
        // milestones log, so a streak of N contentions produces ~log2(N)+1 lines
        // instead of N.
        assert!(!should_log_contention(0), "no contention must not log");
        assert!(should_log_contention(1));
        assert!(should_log_contention(2));
        assert!(!should_log_contention(3));
        assert!(should_log_contention(4));
        assert!(!should_log_contention(5));
        assert!(!should_log_contention(6));
        assert!(!should_log_contention(7));
        assert!(should_log_contention(8));
        assert!(!should_log_contention(9));
    }

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

    /// A client-stats POST that cannot get the `active` lock within its 2 s
    /// budget (a source switch tearing the old pipeline down under the lock)
    /// must say BUSY, not "session not found": the stage reporter stops for
    /// good on a 404, so a busy manager answering 404 silenced a healthy TV's
    /// stats for the rest of its session. Once the lock frees, the same call
    /// succeeds. Paused clock: the 2 s wait elapses instantly.
    #[tokio::test(start_paused = true)]
    async fn a_held_active_map_reads_as_busy_not_session_not_found() {
        let active: tokio::sync::Mutex<std::collections::HashMap<String, ActiveSource>> =
            tokio::sync::Mutex::new(std::collections::HashMap::new());
        let guard = active.lock().await;
        let busy = clone_active_pipelines(&active).await;
        assert!(
            matches!(busy, Err(NdiSessionError::Busy)),
            "a lock wait that expired must be Busy (-> 503, the reporter retries)"
        );
        drop(guard);
        let free = clone_active_pipelines(&active).await;
        assert!(
            matches!(free, Ok(ref pipelines) if pipelines.is_empty()),
            "a free lock must yield the (empty) pipeline list"
        );
    }

    /// `/healthz`'s pipeline reader must say "could not look" (`None`) when
    /// the `active` lock is held past its 200 ms budget — an empty list there
    /// reads as "source down" and vetoes the stage's last-resort reload.
    #[tokio::test(start_paused = true)]
    async fn a_held_active_map_is_unreadable_for_the_health_snapshot() {
        let active: tokio::sync::Mutex<std::collections::HashMap<String, ActiveSource>> =
            tokio::sync::Mutex::new(std::collections::HashMap::new());
        let guard = active.lock().await;
        assert!(
            clone_active_sources(&active, PROBE_LOCK_WAIT)
                .await
                .is_none(),
            "a lock wait that expired must read as unknown, not as no pipelines"
        );
        drop(guard);
        assert!(
            matches!(
                clone_active_sources(&active, PROBE_LOCK_WAIT).await,
                Some(ref sources) if sources.is_empty()
            ),
            "a free lock must yield the (empty) source list"
        );
    }

    /// `/ndi/snapshot/{id}` must say BUSY when the `active` lock is held past
    /// its 200 ms budget: answering "not found" there told the operator an
    /// active source was "not active" during every source switch.
    #[tokio::test(start_paused = true)]
    async fn a_held_active_map_is_busy_for_the_single_source_snapshot() {
        let active: ActiveMap = tokio::sync::Mutex::new(std::collections::HashMap::new());
        let guard = active.lock().await;
        assert!(
            matches!(
                clone_active_pipeline(&active, "src-1").await,
                Err(NdiSessionError::Busy)
            ),
            "a lock wait that expired must be Busy (-> 503), never not-active (-> 404)"
        );
        drop(guard);
        assert!(
            matches!(clone_active_pipeline(&active, "src-1").await, Ok(None)),
            "a free lock without that source must read as not active"
        );
    }

    /// The client-stats POST waits out a short hold that the probes give up
    /// on: a ~300 ms teardown under the lock must not turn a healthy TV's
    /// sample into a 503, while a probe still answers within its 200 ms.
    /// Pins CLIENT_STATS_LOCK_WAIT > PROBE_LOCK_WAIT, deterministic on a
    /// paused clock.
    #[tokio::test(start_paused = true)]
    async fn client_stats_outlasts_a_hold_the_probes_give_up_on() {
        let active = std::sync::Arc::new(ActiveMap::new(std::collections::HashMap::new()));
        let guard = std::sync::Arc::clone(&active).lock_owned().await;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            drop(guard);
        });
        assert!(
            clone_active_sources(&active, PROBE_LOCK_WAIT)
                .await
                .is_none(),
            "a probe must give up on a hold longer than its 200 ms budget"
        );
        assert!(
            matches!(clone_active_pipelines(&active).await, Ok(ref pipelines) if pipelines.is_empty()),
            "a client-stats POST must wait out a 300 ms hold, not answer Busy"
        );
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
