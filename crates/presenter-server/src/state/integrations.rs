use std::collections::HashMap;

use presenter_core::{
    AndroidStageDisplay, AndroidStageDisplayDraft, AndroidStageDisplayId, LiveEvent, ResolumeHost,
    ResolumeHostDraft, ResolumeHostId, VideoSource, VideoSourceDraft, VideoSourceId,
};

use presenter_ndi::PipelineStartError;

use super::AppState;
use crate::android_stage::AndroidStageDisplayStatusSnapshot;
use crate::resolume::ResolumeConnectionSnapshot;
use crate::state::video_source_status;
use crate::state::video_source_status::{Discovery, PipelineFact};

/// What the server can honestly say about every mapped NDI source right now (#546).
///
/// `discovered` rides along deliberately: seeing the mapped `RESOLUME-PP (cg-obs)`
/// next to the network's actual `STREAM-PP (stream)` is what makes a renamed or
/// switched-off sender obvious at a glance — the whole point of the ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSourceStatusSnapshot {
    /// False when this server has no NDI SDK and therefore cannot see the network.
    pub ndi_available: bool,
    /// The NDI names that ARE on the network right now.
    pub discovered: Vec<String>,
    pub sources: Vec<VideoSourceStatusEntry>,
}

/// One mapped source's live state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSourceStatusEntry {
    pub id: String,
    pub ndi_name: String,
    pub is_active: bool,
    /// `unknown | not-found | ready | connecting | not-broadcasting | live`.
    pub state: &'static str,
    /// The pipeline's error text, when it has one.
    pub detail: Option<String>,
}

impl VideoSourceStatusEntry {
    fn unknown(row: &VideoSource) -> Self {
        Self {
            id: row.id.to_string(),
            ndi_name: row.ndi_name.clone(),
            is_active: row.is_active,
            state: video_source_status::VideoSourceState::Unknown.as_str(),
            detail: None,
        }
    }
}

/// How a failed `start_pipeline` should be surfaced when activating a source.
///
/// Separates the published stage status from whether the activation itself is a
/// hard error. A SILENT source (broadcaster off / not producing) is NOT a hard
/// error — the source is genuinely activated and just waiting for signal, so the
/// HTTP activate succeeds and the stage shows a neutral `no-signal` placeholder.
/// A GENUINE pipeline failure is a hard error: publish `failed: <reason>` (red
/// overlay) and propagate the error to the caller (#448).
#[derive(Debug, Clone, PartialEq, Eq)]
struct NdiStartStatus {
    /// The `ndi_status` string published over the live hub.
    status: String,
    /// Whether activation should fail (propagate `Err`) — true only for a
    /// genuine pipeline failure, false for a silent/not-producing source.
    is_hard_error: bool,
}

/// Classify a `start_pipeline` error into the stage status to publish and
/// whether the activation is a hard error. See [`NdiStartStatus`] and #448.
fn ndi_status_for_start_error(err: &PipelineStartError) -> NdiStartStatus {
    match err {
        // The source is configured but its broadcaster is silent / not producing
        // — an EXPECTED state. Publish the neutral `no-signal` status (gray
        // "waiting for source" placeholder) and DON'T fail the activation (#448).
        PipelineStartError::SourceSilent { .. } => NdiStartStatus {
            status: "no-signal".to_string(),
            is_hard_error: false,
        },
        // A genuine pipeline failure → red `failed: <reason>` overlay + hard
        // error so the operator sees what's wrong and the activate call errors.
        PipelineStartError::Failed(e) => NdiStartStatus {
            status: format!("failed: {e}"),
            is_hard_error: true,
        },
        // #741: the in-flight reservation was superseded mid-wait (a concurrent
        // deactivate/switch). Defensive arm only — `activate_video_source`
        // handles Superseded FIRST and publishes nothing; keep it non-hard and
        // neutral so this fn stays total and can never surface a red overlay.
        PipelineStartError::Superseded => NdiStartStatus {
            status: "no-signal".to_string(),
            is_hard_error: false,
        },
    }
}

impl AppState {
    // Resolume methods
    pub async fn list_resolume_hosts(&self) -> anyhow::Result<Vec<ResolumeHost>> {
        self.repository.list_resolume_hosts().await
    }

    pub async fn resolume_status_snapshot(
        &self,
    ) -> HashMap<ResolumeHostId, ResolumeConnectionSnapshot> {
        self.resolume_registry.snapshot().await
    }

    pub async fn resolume_status_for(&self, id: ResolumeHostId) -> ResolumeConnectionSnapshot {
        self.resolume_registry.snapshot_for(id).await
    }

    pub async fn test_resolume_host_connection(
        &self,
        id: ResolumeHostId,
    ) -> anyhow::Result<crate::resolume::TestConnectionResult> {
        let host = self
            .repository
            .list_resolume_hosts()
            .await?
            .into_iter()
            .find(|h| h.id == id)
            // #608: typed refusal (#584/#586 pattern) — the router downcasts to
            // `RepositoryError` and maps `NotFound` to 404 instead of a bare 500.
            .ok_or(presenter_persistence::RepositoryError::NotFound(
                "resolume host not found",
            ))?;
        crate::resolume::test_connection(&host).await
    }

    pub async fn create_resolume_host(
        &self,
        draft: ResolumeHostDraft,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<ResolumeHost> {
        let host = self
            .repository
            .create_resolume_host(&draft, source, actor)
            .await?;
        self.sync_resolume_hosts().await?;
        Ok(host)
    }

    pub async fn update_resolume_host(
        &self,
        id: ResolumeHostId,
        draft: ResolumeHostDraft,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<ResolumeHost> {
        let host = self
            .repository
            .update_resolume_host(id, &draft, source, actor)
            .await?;
        self.sync_resolume_hosts().await?;
        Ok(host)
    }

    pub async fn delete_resolume_host(
        &self,
        id: ResolumeHostId,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        self.repository
            .delete_resolume_host(id, source, actor)
            .await?;
        self.sync_resolume_hosts().await
    }

    pub(super) async fn sync_resolume_hosts(&self) -> anyhow::Result<()> {
        // #483: wire the DB-backed per-push audit writer before any host worker
        // is spawned, so each push persists a `resolume_push_audit` row and the
        // cross-host perceived-latency line is emitted. Idempotent — only the
        // first call spawns the writer task.
        self.resolume_registry
            .attach_audit_writer(self.repository.clone());
        let hosts = self.repository.list_resolume_hosts().await?;
        self.resolume_registry.set_hosts(hosts).await;
        Ok(())
    }

    // Android stage methods
    pub async fn list_android_stage_displays(&self) -> anyhow::Result<Vec<AndroidStageDisplay>> {
        self.repository.list_android_stage_displays().await
    }

    pub async fn android_stage_status_snapshot(
        &self,
    ) -> HashMap<AndroidStageDisplayId, AndroidStageDisplayStatusSnapshot> {
        self.android_stage_registry.snapshot().await
    }

    pub async fn android_stage_status_for(
        &self,
        id: AndroidStageDisplayId,
    ) -> AndroidStageDisplayStatusSnapshot {
        self.android_stage_registry.snapshot_for(id).await
    }

    pub async fn create_android_stage_display(
        &self,
        draft: AndroidStageDisplayDraft,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<AndroidStageDisplay> {
        let display = self
            .repository
            .create_android_stage_display(&draft, source, actor)
            .await?;
        self.sync_android_stage_displays().await?;
        Ok(display)
    }

    pub async fn update_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
        draft: AndroidStageDisplayDraft,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<AndroidStageDisplay> {
        let display = self
            .repository
            .update_android_stage_display(id, &draft, source, actor)
            .await?;
        self.sync_android_stage_displays().await?;
        Ok(display)
    }

    pub async fn delete_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        self.repository
            .delete_android_stage_display(id, source, actor)
            .await?;
        self.sync_android_stage_displays().await
    }

    pub async fn launch_now_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
    ) -> anyhow::Result<()> {
        self.android_stage_registry.launch_now(id).await
    }

    pub(super) async fn sync_android_stage_displays(&self) -> anyhow::Result<()> {
        let displays = self.repository.list_android_stage_displays().await?;
        self.android_stage_registry.set_displays(displays).await;
        Ok(())
    }

    /// Populate + launch the Android stage displays. Called ONCE at startup from
    /// `main` AFTER the HTTP listener is bound (#423), not during `from_config`:
    /// firing the launcher before the server is serving made the on-device
    /// `am start` hit a connection-refused, the TV showed the browser error
    /// page, and the #419 foreground-aware keep-alive then skipped the relaunch
    /// forever (the browser was foreground on the error page). Triggering it once
    /// the listener is up means the startup launch always lands on a serving
    /// server, so a deploy/restart never strands a display.
    pub async fn start_android_stage_displays(&self) -> anyhow::Result<()> {
        self.sync_android_stage_displays().await
    }

    // Video source methods
    pub async fn list_video_sources(&self) -> anyhow::Result<Vec<VideoSource>> {
        self.repository.list_video_sources().await
    }

    /// Is each mapped NDI source actually working, right now? (#546)
    ///
    /// The three facts that answer that live in three different places — the DB rows,
    /// the NDI discovery list, and the pipeline map — and until now nothing joined
    /// them, so a mapped-but-absent source (the PP outage) was indistinguishable from
    /// a broken server. This is the join; the decision itself is the pure
    /// [`video_source_status::classify`].
    ///
    /// Fail-soft on purpose — but never into a LIE. A discovery failure degrades to
    /// [`Discovery::Blind`] ("we cannot see the network"), NOT to an empty network
    /// (which would make a broken server accuse every sending machine at the site);
    /// a busy pipeline lock degrades to [`PipelineFact::Unreadable`] ("the manager is
    /// busy starting it"), NOT to "no pipeline" (which would paint every normal
    /// activation as "sending nothing"). Both were deep-review findings on the first
    /// cut of #546. Neither degrades to an error page: the settings page must keep
    /// rendering when NDI is unhappy — that is exactly when the operator needs it.
    pub async fn video_source_status(&self) -> anyhow::Result<VideoSourceStatusSnapshot> {
        let rows = self.list_video_sources().await?;

        let Some(manager) = &self.ndi_manager else {
            // No SDK: we cannot see the network, and we say so rather than accusing a
            // sending machine that may be perfectly fine.
            return Ok(VideoSourceStatusSnapshot {
                ndi_available: false,
                discovered: Vec::new(),
                sources: rows
                    .into_iter()
                    .map(|r| VideoSourceStatusEntry::unknown(&r))
                    .collect(),
            });
        };

        // `None` = the finder has never completed a scan (it failed to start, or we have
        // only just booted). That is BLINDNESS, not an empty network — and the difference
        // is the whole ticket: an empty list reported as fact makes the page tell the
        // operator that every sending machine at the site is switched off.
        let discovered: Option<Vec<String>> = manager
            .discovery_snapshot()
            .map(|sources| sources.into_iter().map(|s| s.name).collect());
        if discovered.is_none() {
            tracing::warn!(
                "NDI finder has not completed a scan — reporting the network as unseen \
                 rather than empty (#546)"
            );
        }

        // `None` = the manager's lock was held past our budget (it is busy building a
        // pipeline), which is NOT the same fact as "there are no pipelines".
        let pipelines: Option<HashMap<String, (&'static str, Option<String>)>> =
            manager.pipeline_snapshots_checked().await.map(|snapshots| {
                snapshots
                    .into_iter()
                    .map(|(id, state)| (id, video_source_status::pipeline_state_str(&state)))
                    .collect()
            });

        let discovery = match &discovered {
            Some(names) => Discovery::Names(names),
            None => Discovery::Blind,
        };

        let sources = rows
            .into_iter()
            .map(|row| {
                let pipeline = pipelines
                    .as_ref()
                    .map(|map| map.get(&row.id.to_string()).cloned());
                let state = video_source_status::classify(
                    row.is_active,
                    &row.ndi_name,
                    discovery,
                    match &pipeline {
                        Some(entry) => PipelineFact::Known(entry.as_ref().map(|(s, _)| *s)),
                        None => PipelineFact::Unreadable,
                    },
                );
                VideoSourceStatusEntry {
                    id: row.id.to_string(),
                    ndi_name: row.ndi_name,
                    is_active: row.is_active,
                    state: state.as_str(),
                    detail: pipeline.flatten().and_then(|(_, err)| err),
                }
            })
            .collect();

        Ok(VideoSourceStatusSnapshot {
            // A discovery failure leaves us blind, exactly like a missing SDK — the UI
            // must not print "On the network now: nothing" off the back of it.
            ndi_available: discovered.is_some(),
            discovered: discovered.unwrap_or_default(),
            sources,
        })
    }

    pub async fn create_video_source(
        &self,
        draft: VideoSourceDraft,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<VideoSource> {
        self.repository
            .create_video_source(&draft, source, actor)
            .await
    }

    pub async fn update_video_source(
        &self,
        id: VideoSourceId,
        draft: VideoSourceDraft,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<VideoSource> {
        self.repository
            .update_video_source(id, &draft, source, actor)
            .await
    }

    pub async fn delete_video_source(
        &self,
        id: VideoSourceId,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        // #745(a): serialize with `activate_video_source`. Without this, the
        // `stop_pipeline` below can land in the window between a concurrent
        // activation's DB flip and its manager-lock acquisition — the stop no-ops
        // (nothing in the map yet), the activation then promotes and supervises a
        // pipeline, leaving DB-inactive-but-streaming with no reconciliation. Lock
        // order is strictly `activation_lock` → manager `active` (never inverted),
        // so it cannot deadlock with an activation.
        let _activation_guard = self.activation_lock.lock().await;
        // Stop the source's pipeline BEFORE deleting the row. Without this,
        // deleting an ACTIVE source leaked its encoder pipeline (it kept
        // streaming forever — observed as N zombie `ndi_pipelines` in
        // /healthz after repeated create→delete cycles).
        if let Some(manager) = &self.ndi_manager {
            manager.stop_pipeline(&id.to_string()).await;
        }
        self.repository.delete_video_source(id, source, actor).await
    }

    pub async fn activate_video_source(
        &self,
        id: VideoSourceId,
        audit_source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<VideoSource> {
        // #745(a): serialize activations across the WHOLE body (DB flip →
        // start_pipeline → sibling reap). Two concurrent activations otherwise
        // interleave their reaps independently of the DB "last write wins" order,
        // leaving the manager's single-active source out of step with the DB until
        // the next reconnect cycle. Held across the ~8 s start_pipeline wait, but
        // this is a server-side lock the status readers never take, so it does not
        // reintroduce the #741 status-poll stall.
        //
        // ACCEPTED trade-off: when a source is DOWN, the 30 s reconnect ticker
        // (`background_tasks.rs`) holds this lock across its full ~8 s futile
        // `start_pipeline` timeout, so an operator's switch-to-another-source click
        // can queue up to ~8 s behind it (these ran concurrently pre-#745). This is
        // the inherent price of serialization — and the ≤30 s WRONG-source-on-stage
        // mismatch it removes is worse. tokio's Mutex is FIFO-fair, so the operator
        // waits behind at most one such attempt.
        let _activation_guard = self.activation_lock.lock().await;
        self.activate_video_source_locked(id, audit_source, actor)
            .await
    }

    /// The body of [`Self::activate_video_source`], assuming the caller ALREADY
    /// holds `activation_lock`. Split out (#747) so
    /// [`Self::reconnect_active_video_source`] can hold the lock across BOTH its
    /// DB re-read AND the activation without re-entering the non-reentrant tokio
    /// `Mutex` — that atomicity is what closes the reconnect ticker's
    /// read-then-activate TOCTOU (a deactivate landing in the old read→activate
    /// gap could otherwise revive a just-deactivated source).
    async fn activate_video_source_locked(
        &self,
        id: VideoSourceId,
        audit_source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<VideoSource> {
        let source = self
            .repository
            .activate_video_source(id, audit_source, actor)
            .await?;
        self.live_hub.publish(LiveEvent::NdiSourceActivated {
            source_id: source.id.to_string(),
            ndi_name: source.ndi_name.clone(),
            label: source.label.clone(),
        });
        if let Some(manager) = &self.ndi_manager {
            if let Err(e) = manager
                .start_pipeline(&source.id.to_string(), &source.ndi_name)
                .await
            {
                // #741: a SUPERSEDED start (a concurrent deactivate/stop removed
                // the in-flight reservation, or an activate-switch replaced it) is
                // neither success nor failure — return Ok and publish NOTHING; the
                // concurrent op owns the source's real status. Mapping it to a
                // status would emit a stray `no-signal` after a deactivate.
                if matches!(e, PipelineStartError::Superseded) {
                    tracing::info!(
                        source_id = %source.id,
                        ndi_name = %source.ndi_name,
                        "NDI start superseded by a concurrent activation change (#741) — publishing nothing"
                    );
                    return Ok(source);
                }
                let classified = ndi_status_for_start_error(&e);
                if classified.is_hard_error {
                    // A GENUINE pipeline failure. Surface the reason to the
                    // stage view so the operator sees what's wrong instead of
                    // an endless "Connecting…" overlay. The DB row stays
                    // `is_active=true` so the operator can retry by toggling
                    // off+on once the issue is fixed.
                    tracing::error!(
                        error = %e,
                        source_id = %source.id,
                        ndi_name = %source.ndi_name,
                        "NDI pipeline start failed"
                    );
                    self.live_hub.publish(LiveEvent::NdiConnectionStatus {
                        status: classified.status,
                    });
                    return Err(anyhow::Error::new(e));
                }
                // #448: the source is configured but its broadcaster is silent /
                // not producing — an EXPECTED state, not a failure. The
                // activation SUCCEEDS (the source is genuinely active, just
                // waiting for signal); the stage shows a neutral `no-signal`
                // placeholder, not a red error.
                tracing::info!(
                    source_id = %source.id,
                    ndi_name = %source.ndi_name,
                    "NDI source activated but not yet producing — broadcaster silent (#448)"
                );
                self.live_hub.publish(LiveEvent::NdiConnectionStatus {
                    status: classified.status,
                });
                // Reap any sibling pipelines just as the success path does, so a
                // switch to a not-yet-live source still tears down the previous
                // source's encoder (the #370 single-active-source invariant).
                manager.stop_other_pipelines(&source.id.to_string()).await;
                return Ok(source);
            }
            // start_pipeline only returns Ok AFTER the webrtcsink video pad
            // has negotiated caps — at that point frames are flowing through
            // the pipeline. Flip the stage-view overlay from "Connecting…"
            // to "" (no overlay) by publishing `connected` status.
            self.live_hub.publish(LiveEvent::NdiConnectionStatus {
                status: "connected".to_string(),
            });
            // #370: the DB just flipped every sibling source to
            // `is_active=false` (repository.activate_video_source), but the
            // manager was never told to stop their pipelines. Without this,
            // switching the active source (deactivate A → activate B) leaked
            // A's pipeline + its nvh264enc encoder — two source pipelines (=
            // two NVENC encoders) kept running after every switch. Reap them
            // now that the new source is confirmed Streaming, so the operator
            // never sees a gap and exactly ONE source pipeline remains.
            manager.stop_other_pipelines(&source.id.to_string()).await;
        }
        Ok(source)
    }

    pub async fn deactivate_video_sources(
        &self,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        // #745(a): serialize with `activate_video_source` — same window as delete.
        // `stop_all()` must not interleave with a concurrent activation's DB-flip →
        // manager-lock gap (would leave DB-inactive-but-streaming). Lock order is
        // strictly `activation_lock` → manager `active`, so no deadlock.
        let _activation_guard = self.activation_lock.lock().await;
        self.repository
            .deactivate_all_video_sources(source, actor)
            .await?;
        self.live_hub.publish(LiveEvent::NdiSourceDeactivated);
        // Stop all NDI pipelines if manager is available
        if let Some(manager) = &self.ndi_manager {
            manager.stop_all().await;
        }
        Ok(())
    }

    /// The NDI 30 s auto-reconnect ticker's read-then-activate step
    /// (`background_tasks.rs`): (re)activate whatever source is currently
    /// `is_active` in the DB, restoring its pipeline after the sender comes back
    /// online. Returns the reconnected source, or `None` when no source is active.
    ///
    /// #747: acquire `activation_lock` BEFORE reading the active source and hold
    /// it across the activation, so an operator `deactivate_video_sources` /
    /// `delete_video_source` (which take the same lock) cannot commit in a
    /// read→activate gap and get its source revived by this ticker. The read
    /// used to run outside any activation critical section, so a deactivate could
    /// land in that gap and the 30 s ticker would flip the row back to active
    /// ~within one tick. Re-reading the *active* source under the lock (rather
    /// than a caller-supplied id) also means a concurrent SWITCH reconnects the
    /// NEW winner, never the stale one. Lock order stays `activation_lock` →
    /// manager `active`, so this cannot deadlock with an activation/teardown.
    pub async fn reconnect_active_video_source(
        &self,
        audit_source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<Option<VideoSource>> {
        let _activation_guard = self.activation_lock.lock().await;
        let Some(source) = self.repository.get_active_video_source().await? else {
            // No source is active: none was ever set, or the operator deactivated
            // it (possibly in the window the old read-then-activate race missed).
            // Skip — reviving it here is exactly the #747 bug.
            return Ok(None);
        };
        self.activate_video_source_locked(source.id, audit_source, actor)
            .await
            .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::{ndi_status_for_start_error, PipelineStartError};
    use crate::state::ndi_control::{NdiCall, NdiManagerHandle, StartOutcome};
    use crate::state::AppState;
    use presenter_core::{LiveEvent, ResolumeHostDraft, VideoSourceDraft, VideoSourceId};
    use presenter_persistence::SettingsAuditSource;

    /// #483: `sync_resolume_hosts` must load hosts from the DB and register them
    /// in the registry (and wire the audit writer). Guards against the body being
    /// short-circuited away (mutation: `-> Ok(())`).
    #[tokio::test]
    async fn sync_resolume_hosts_registers_hosts_from_db() {
        let state = AppState::in_memory().await.expect("state");
        // Registry starts empty (no resolume hosts seeded).
        assert!(state.resolume_status_snapshot().await.is_empty());

        let draft = ResolumeHostDraft::new("Arena", "127.0.0.1", 8090);
        state
            .repository()
            .create_resolume_host(&draft, SettingsAuditSource::HttpSetter, "test")
            .await
            .expect("create host");

        state.sync_resolume_hosts().await.expect("sync");

        assert_eq!(
            state.resolume_status_snapshot().await.len(),
            1,
            "sync must register the host that exists in the DB"
        );
    }

    // ── #406: GUARD the #370 source-switch reap WIRING ───────────────────────
    //
    // The #370 fix reaps stale sibling pipelines on a source switch by calling
    // `manager.stop_other_pipelines(new_id)` inside `activate_video_source`
    // AFTER `start_pipeline` returns Ok. The reap HELPER is unit-tested in the
    // NDI crate, but NOTHING tested the WIRING — that `activate_video_source`
    // actually CALLS the reap. A refactor could silently delete that call and
    // reintroduce the #370 two-encoder NVENC leak with all tests still green.
    //
    // These tests inject a recording `FakeNdiControl` (a stand-in for the
    // libndi/GPU hardware boundary — see `ndi_control` module docs) so the
    // hardware-gated `if let Some(manager) = &self.ndi_manager` branch is
    // reachable on the libndi-free `Rust Tests` CI host.
    //
    // ACCEPTANCE (#406): deleting the `manager.stop_other_pipelines(...)` call
    // in `activate_video_source` MUST make `activation_reaps_siblings_after_…`
    // FAIL — proving it guards the wiring, not just the helper.

    /// Build an in-memory AppState with a `FakeNdiControl` injected and one
    /// video source created. Returns the state, the new source id (and its
    /// string form, which is the key the reap is expected to keep), and the
    /// fake for assertions.
    async fn state_with_fake(
        outcome: StartOutcome,
    ) -> (
        AppState,
        VideoSourceId,
        String,
        std::sync::Arc<crate::state::ndi_control::FakeNdiControl>,
    ) {
        let mut state = AppState::in_memory().await.expect("in-memory AppState");
        let fake = crate::state::ndi_control::FakeNdiControl::with_outcome(outcome);
        state.set_ndi_handle(NdiManagerHandle::Fake(fake.clone()));
        let source = state
            .create_video_source(
                VideoSourceDraft::new("Cam 1", "STREAM-SNV (stream)"),
                SettingsAuditSource::HttpSetter,
                "test",
            )
            .await
            .expect("create video source");
        (state, source.id, source.id.to_string(), fake)
    }

    #[tokio::test]
    async fn activation_reaps_siblings_after_successful_start() {
        let (state, source_id, id, fake) = state_with_fake(StartOutcome::Ok).await;

        let activated = state
            .activate_video_source(source_id, SettingsAuditSource::HttpSetter, "test")
            .await
            .expect("activation succeeds");
        assert_eq!(activated.id.to_string(), id);

        let calls = fake.calls();
        // start_pipeline must have been called for the new source…
        assert!(
            matches!(
                calls.first(),
                Some(NdiCall::StartPipeline { source_id, .. }) if *source_id == id
            ),
            "activate_video_source must call start_pipeline(new_id); calls = {calls:?}",
        );
        // …and the reap must have been called for the SAME id, AFTER the start.
        // This is the line guarded by #406: deleting the reap call in
        // activate_video_source makes this assertion fail.
        assert!(
            fake.reaped(&id),
            "after a successful start, activate_video_source MUST reap siblings via \
             stop_other_pipelines(new_id) (#370 single-active-source invariant); calls = {calls:?}",
        );
        assert_eq!(
            calls,
            vec![
                NdiCall::StartPipeline {
                    source_id: id.clone(),
                    ndi_name: "STREAM-SNV (stream)".to_string(),
                },
                NdiCall::StopOtherPipelines {
                    keep_id: id.clone()
                },
            ],
            "the reap must run exactly once, AFTER start_pipeline, keeping the new id",
        );
    }

    /// #745(a) RED: two concurrent `activate_video_source` calls must SERIALIZE so
    /// the manager's single-active source (the LAST reap) matches the DB's
    /// last-write winner. Without serialization the two reaps interleave
    /// independently of the DB "last write wins" order, leaving the manager out of
    /// step with the DB until the next reconnect cycle (the WRONG source on stage).
    ///
    /// Deterministic via the fake's per-source start-gate: A is parked mid-start
    /// (its DB row already written) while B races. Without the lock B runs to
    /// completion (reap B) while A is parked, then A's reap lands LAST → the manager
    /// keeps A while the DB winner is B. With the lock B cannot even begin until A
    /// releases → strictly grouped, the manager keeps B == DB winner.
    #[tokio::test]
    async fn concurrent_activations_serialize_manager_to_db_winner() {
        let (state, a_id, a_str, fake) = state_with_fake(StartOutcome::Ok).await;
        let b = state
            .create_video_source(
                VideoSourceDraft::new("Cam 2", "STREAM-B (stream)"),
                SettingsAuditSource::HttpSetter,
                "test",
            )
            .await
            .expect("create source B");
        let b_id = b.id;
        let b_str = b_id.to_string();

        // Park A's start; it holds any serialization lock while B races.
        let (release_a, parked_a) = fake.gate_start(&a_str);

        let s_a = state.clone();
        let ta = tokio::spawn(async move {
            s_a.activate_video_source(a_id, SettingsAuditSource::HttpSetter, "test")
                .await
        });
        // A has written its DB row (is_active=A) and is now parked inside start.
        // Bounded so a regression that returns before start_pipeline (e.g. an early
        // Err) fails loudly here instead of hanging until the CI-job timeout.
        tokio::time::timeout(std::time::Duration::from_secs(5), parked_a.notified())
            .await
            .expect("activation A never reached the parked start-gate within 5s");

        // Race B. Without the lock B runs to completion while A is parked; with the
        // lock B blocks on the lock until A releases.
        let s_b = state.clone();
        let tb = tokio::spawn(async move {
            s_b.activate_video_source(b_id, SettingsAuditSource::HttpSetter, "test")
                .await
        });

        // Bounded window: B either COMPLETES its reap (no lock) or is BLOCKED (lock).
        // 500 ms >> scheduling latency → a reliable "B is serialized" probe, not a
        // flaky sleep.
        let b_reaped_while_a_parked =
            tokio::time::timeout(std::time::Duration::from_millis(500), async {
                loop {
                    if fake.reaped(&b_str) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .is_ok();

        // Release A; let both finish (timeout-guarded — a lock regression that
        // deadlocks fails loudly instead of hanging the suite).
        release_a.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), ta)
            .await
            .expect("activation A did not finish within 5s — deadlock?")
            .expect("activation A task panicked")
            .expect("activation A returned Err");
        tokio::time::timeout(std::time::Duration::from_secs(5), tb)
            .await
            .expect("activation B did not finish within 5s — deadlock?")
            .expect("activation B task panicked")
            .expect("activation B returned Err");

        // B is the DB's last-write winner (A wrote its row before parking, B after).
        // Serialization means B cannot reap while A holds the lock…
        assert!(
            !b_reaped_while_a_parked,
            "#745(a): a second activation must NOT proceed while the first holds the \
             activation lock; B reaped while A was parked → activations interleaved. \
             ledger = {:?}",
            fake.calls(),
        );
        // …and the manager's final single-active source (the LAST reap) must equal
        // the DB winner B.
        let last_reap = fake.calls().into_iter().rev().find_map(|c| match c {
            NdiCall::StopOtherPipelines { keep_id } => Some(keep_id),
            _ => None,
        });
        assert_eq!(
            last_reap.as_deref(),
            Some(b_str.as_str()),
            "#745(a): the manager's final single-active source (last reap) must equal \
             the DB last-write winner (B); ledger = {:?}",
            fake.calls(),
        );
    }

    // #745(a): `deactivate_video_sources` tears down ALL pipelines via `stop_all()`.
    // If it does NOT take the activation lock, that `stop_all()` can land in the
    // window between a concurrent activation's DB flip and its manager-lock
    // acquisition — the stop no-ops (nothing in the map yet), the activation then
    // promotes and supervises a pipeline, and the final state is: DB says inactive
    // but the source streams to the stage forever, never reconciled by the ticker
    // (its `Ok(None)` arm). Parks an activation mid-start (holding the lock) and
    // proves a racing deactivate CANNOT run its `stop_all()` until it releases.
    #[tokio::test]
    async fn deactivate_serializes_behind_a_parked_activation() {
        let (state, a_id, a_str, fake) = state_with_fake(StartOutcome::Ok).await;

        let (release_a, parked_a) = fake.gate_start(&a_str);
        let s_a = state.clone();
        let ta = tokio::spawn(async move {
            s_a.activate_video_source(a_id, SettingsAuditSource::HttpSetter, "test")
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), parked_a.notified())
            .await
            .expect("activation never reached the parked start-gate within 5s");

        let s_d = state.clone();
        let td = tokio::spawn(async move {
            s_d.deactivate_video_sources(SettingsAuditSource::HttpSetter, "test")
                .await
        });

        // Without the lock the deactivate runs `stop_all()` immediately; with the
        // lock it blocks until the activation releases. 500 ms >> scheduling latency.
        let stopped_all_while_parked =
            tokio::time::timeout(std::time::Duration::from_millis(500), async {
                loop {
                    if fake.stopped_all() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .is_ok();

        release_a.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), ta)
            .await
            .expect("activation did not finish within 5s — deadlock?")
            .expect("activation task panicked")
            .expect("activation returned Err");
        tokio::time::timeout(std::time::Duration::from_secs(5), td)
            .await
            .expect("deactivate did not finish within 5s — deadlock?")
            .expect("deactivate task panicked")
            .expect("deactivate returned Err");

        assert!(
            !stopped_all_while_parked,
            "#745(a): deactivate_video_sources must take the activation lock — its \
             stop_all() ran while an activation held the lock, so a deactivate can \
             tear down (no-op) mid-activation and leave DB-inactive-but-streaming. \
             ledger = {:?}",
            fake.calls(),
        );
        assert!(
            fake.stopped_all(),
            "#745(a): deactivate must still stop all pipelines once it acquires the lock",
        );
    }

    // #745(a): `delete_video_source` tears down the source's pipeline via
    // `stop_pipeline()` BEFORE deleting the row — the same activation-window race as
    // deactivate. Proves delete now serializes behind a parked activation.
    #[tokio::test]
    async fn delete_serializes_behind_a_parked_activation() {
        let (state, a_id, a_str, fake) = state_with_fake(StartOutcome::Ok).await;
        let b = state
            .create_video_source(
                VideoSourceDraft::new("Cam 2", "STREAM-B (stream)"),
                SettingsAuditSource::HttpSetter,
                "test",
            )
            .await
            .expect("create source B");
        let b_id = b.id;
        let b_str = b_id.to_string();

        let (release_a, parked_a) = fake.gate_start(&a_str);
        let s_a = state.clone();
        let ta = tokio::spawn(async move {
            s_a.activate_video_source(a_id, SettingsAuditSource::HttpSetter, "test")
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), parked_a.notified())
            .await
            .expect("activation never reached the parked start-gate within 5s");

        let s_d = state.clone();
        let td = tokio::spawn(async move {
            s_d.delete_video_source(b_id, SettingsAuditSource::HttpSetter, "test")
                .await
        });

        let stopped_while_parked =
            tokio::time::timeout(std::time::Duration::from_millis(500), async {
                loop {
                    if fake.stopped(&b_str) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .is_ok();

        release_a.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), ta)
            .await
            .expect("activation did not finish within 5s — deadlock?")
            .expect("activation task panicked")
            .expect("activation returned Err");
        tokio::time::timeout(std::time::Duration::from_secs(5), td)
            .await
            .expect("delete did not finish within 5s — deadlock?")
            .expect("delete task panicked")
            .expect("delete returned Err");

        assert!(
            !stopped_while_parked,
            "#745(a): delete_video_source must take the activation lock — its \
             stop_pipeline() ran while an activation held the lock. ledger = {:?}",
            fake.calls(),
        );
        assert!(
            fake.stopped(&b_str),
            "#745(a): delete must still stop the source's pipeline once it holds the lock",
        );
    }

    // #747: the 30 s NDI auto-reconnect ticker (`background_tasks.rs`) reads the
    // active source and then re-activates it. If that read happens OUTSIDE the
    // `activation_lock`, an operator deactivate committing in the read→activate gap
    // is UNDONE — the ticker revives a source the operator just turned off (~within
    // one tick). The fix holds `activation_lock` across the ticker's DB re-read AND
    // its activation, so the deactivate is seen and the reconnect skips (Ok(None)).
    //
    // A TOCTOU can only be shown with concurrency. This models the read→activate
    // window deterministically: the test HOLDS `activation_lock` (white-box — the
    // test module is a descendant of `crate::state`) while the operator's deactivate
    // commits at the repository (the state-level `deactivate_video_sources` would
    // deadlock on the lock we hold). Pre-fix the reconnect's read races ahead of the
    // lock and revives the source (RED); post-fix its read is UNDER the lock, sees
    // the source inactive, and returns None (GREEN). Outcome is asserted on committed
    // DB state after the join; the bounded settle only lets the spawned task reach its
    // steady blocking state (>> the sub-ms in-memory read).
    #[tokio::test]
    async fn reconnect_must_not_revive_a_source_deactivated_in_the_read_window() {
        let (state, a_id, _a_str, fake) = state_with_fake(StartOutcome::Ok).await;
        // A is the active source the ticker will try to (re)connect.
        state
            .activate_video_source(a_id, SettingsAuditSource::HttpSetter, "test")
            .await
            .expect("activate A");
        assert_eq!(
            state
                .repository()
                .get_active_video_source()
                .await
                .expect("read active")
                .map(|s| s.id),
            Some(a_id),
            "precondition: A is the active source",
        );

        // Hold the activation lock: this IS the read→activate window the ticker races.
        let guard = state.activation_lock.lock().await;

        // The reconnect ticker fires now.
        let s = state.clone();
        let tr = tokio::spawn(async move {
            s.reconnect_active_video_source(SettingsAuditSource::StartupDefault, "system")
                .await
        });

        // Let the reconnect reach its steady blocking state:
        //   fixed  — blocked on the lock (its read is UNDER the lock, not taken yet);
        //   pre-fix — has already read A (active) OUTSIDE the lock, now blocked on it.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // The operator deactivates A while we hold the lock — the deactivate landing
        // in the reconnect's read→activate gap. Committed at the repository (the
        // state-level deactivate would deadlock on the lock we hold).
        state
            .repository()
            .deactivate_all_video_sources(SettingsAuditSource::HttpSetter, "test")
            .await
            .expect("deactivate A");
        assert_eq!(
            state
                .repository()
                .get_active_video_source()
                .await
                .expect("read active"),
            None,
            "A is inactive immediately after the operator's deactivate",
        );

        // Release the lock; the reconnect proceeds.
        drop(guard);
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), tr)
            .await
            .expect("reconnect did not finish within 5s — deadlock?")
            .expect("reconnect task panicked")
            .expect("reconnect returned Err");

        // The fix: the reconnect re-read the active source UNDER the lock, saw A
        // inactive, and skipped — no revive.
        assert!(
            result.is_none(),
            "#747: reconnect must return None when the source was deactivated in the \
             read window (it must re-read under the activation lock); returned {result:?}",
        );
        assert_eq!(
            state
                .repository()
                .get_active_video_source()
                .await
                .expect("read active"),
            None,
            "#747: the NDI auto-reconnect ticker revived a source the operator just \
             deactivated — its DB read must happen under the activation lock so a \
             concurrent deactivate is seen. ledger = {:?}",
            fake.calls(),
        );
    }

    // #741: a SUPERSEDED start (a concurrent deactivate/stop removed the in-flight
    // reservation, or an activate-switch replaced it) must make activate_video_source
    // return Ok WITHOUT reaping siblings and WITHOUT publishing a stage status — the
    // concurrent op owns the source's real status. The early-return path is guarded
    // here: the reap runs only on the Ok/silent paths, never on Superseded.
    #[tokio::test]
    async fn activation_superseded_returns_ok_without_reap() {
        let (state, source_id, id, fake) = state_with_fake(StartOutcome::Superseded).await;
        let mut rx = state.live_hub().subscribe();

        let activated = state
            .activate_video_source(source_id, SettingsAuditSource::HttpSetter, "test")
            .await
            .expect("a superseded start must still return Ok(source)");
        assert_eq!(activated.id.to_string(), id);

        let calls = fake.calls();
        assert_eq!(
            calls,
            vec![NdiCall::StartPipeline {
                source_id: id.clone(),
                ndi_name: "STREAM-SNV (stream)".to_string(),
            }],
            "a superseded start returns Ok early — only start_pipeline, no reap; calls = {calls:?}",
        );
        assert!(
            !fake.reaped(&id),
            "a superseded start must NOT reap siblings — it returns before the reap (#741)",
        );

        // Drain the live hub: a superseded start must NOT publish any stage status
        // (NdiConnectionStatus). It DOES publish NdiSourceActivated up front (the DB
        // row was activated) — that is expected; only the stray stage status is the bug.
        let mut published = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            published.push(ev);
        }
        assert!(
            !published
                .iter()
                .any(|e| matches!(e, LiveEvent::NdiConnectionStatus { .. })),
            "a superseded start must publish NO stage status; got {published:?}",
        );
    }

    #[tokio::test]
    async fn activation_does_not_reap_when_start_hard_errors() {
        let (state, source_id, id, fake) = state_with_fake(StartOutcome::HardError).await;

        let result = state
            .activate_video_source(source_id, SettingsAuditSource::HttpSetter, "test")
            .await;
        assert!(
            result.is_err(),
            "a hard start_pipeline failure must fail the activation",
        );

        // start was attempted, but the reap must NOT run on a hard error —
        // there is no new active pipeline to keep, so reaping siblings would
        // be wrong.
        assert!(
            !fake.reaped(&id),
            "on a hard start failure the reap MUST NOT run; calls = {:?}",
            fake.calls(),
        );
        assert_eq!(
            fake.calls(),
            vec![NdiCall::StartPipeline {
                source_id: id.clone(),
                ndi_name: "STREAM-SNV (stream)".to_string(),
            }],
            "only start_pipeline should have been attempted on a hard error",
        );
    }

    #[tokio::test]
    async fn activation_reaps_siblings_for_silent_source() {
        // #448 path: a silent broadcaster is an Ok-returning activation, and it
        // must STILL reap siblings (a switch to a not-yet-live source still
        // tears down the previous source's encoder).
        let (state, source_id, id, fake) = state_with_fake(StartOutcome::SilentSource).await;

        state
            .activate_video_source(source_id, SettingsAuditSource::HttpSetter, "test")
            .await
            .expect("silent-source activation still succeeds (#448)");

        assert!(
            fake.reaped(&id),
            "a silent-source (Ok) activation MUST also reap siblings (#370 + #448); calls = {:?}",
            fake.calls(),
        );
    }

    // ── #448: an off/silent source is NOT a hard error / red overlay ─────────
    //
    // Live on prod 2026-06-22 (Resolume 'cg' OFF), activating a source whose
    // broadcaster is silent published `failed: … broadcaster is silent`, which
    // the stage painted RED. A silent source is an expected state — it must
    // publish the neutral `no-signal` status and NOT fail the activation.

    #[test]
    fn silent_source_maps_to_neutral_no_signal_and_is_not_a_hard_error() {
        let err = PipelineStartError::SourceSilent {
            ndi_name: "RESOLUME-SNV (cg-obs)".to_string(),
        };
        let classified = ndi_status_for_start_error(&err);
        assert_eq!(
            classified.status, "no-signal",
            "a silent broadcaster must publish the neutral `no-signal` status (#448)",
        );
        assert!(
            !classified.is_hard_error,
            "a silent broadcaster must NOT fail the activation (#448)",
        );
    }

    #[test]
    fn genuine_failure_maps_to_red_failed_status_and_is_a_hard_error() {
        let err =
            PipelineStartError::Failed(anyhow::anyhow!("no hardware H264 encoder registered"));
        let classified = ndi_status_for_start_error(&err);
        assert_eq!(
            classified.status, "failed: no hardware H264 encoder registered",
            "a genuine failure must publish `failed: <reason>` so the operator sees it",
        );
        assert!(
            classified.is_hard_error,
            "a genuine pipeline failure must fail the activation",
        );
    }

    // #741: the defensive Superseded arm keeps `ndi_status_for_start_error` total
    // and never surfaces a red overlay (activate_video_source handles Superseded
    // first, so this arm is a safety net only).
    #[test]
    fn superseded_maps_to_neutral_and_is_not_a_hard_error() {
        let classified = ndi_status_for_start_error(&PipelineStartError::Superseded);
        assert!(
            !classified.is_hard_error,
            "a superseded start must never be a hard error (#741)",
        );
        assert_eq!(
            classified.status, "no-signal",
            "the defensive Superseded arm stays neutral",
        );
    }

    // ── #546: does the server actually JOIN the three facts? ────────────────────
    //
    // The classifier's rules are unit-tested in `state::video_source_status`. What
    // these two guard is the join itself: that the state method really reads the NDI
    // discovery list and the pipeline map, and really reports what it finds. Without
    // them the classifier could be perfect while the server fed it nothing.

    #[tokio::test]
    async fn video_source_status_reports_the_pp_incident_as_not_found() {
        let (state, source_id, _id, fake) = state_with_fake(StartOutcome::SilentSource).await;
        // The network carries a DIFFERENT name than the one the operator mapped —
        // the source is `STREAM-SNV (stream)` (see `state_with_fake`).
        fake.set_discovered(&["SOMETHING-ELSE (stream)"]);
        state
            .activate_video_source(source_id, SettingsAuditSource::HttpSetter, "test")
            .await
            .expect("a silent source still activates (#448)");

        let snapshot = state.video_source_status().await.expect("status snapshot");

        assert!(snapshot.ndi_available);
        assert_eq!(snapshot.discovered, vec!["SOMETHING-ELSE (stream)"]);
        let entry = snapshot
            .sources
            .first()
            .expect("the created source is in the snapshot");
        assert_eq!(
            entry.state, "not-found",
            "a mapped name that is not on the network must say so — the operator was \
             left staring at a blank stage with no clue why (#546)",
        );
        assert!(entry.is_active, "the row IS activated — that is the trap");
    }

    #[tokio::test]
    async fn video_source_status_reports_live_when_the_pipeline_is_streaming() {
        let (state, source_id, id, fake) = state_with_fake(StartOutcome::Ok).await;
        fake.set_discovered(&["STREAM-SNV (stream)"]);
        fake.set_pipeline(&id, presenter_ndi::pipeline::PipelineState::Streaming);
        state
            .activate_video_source(source_id, SettingsAuditSource::HttpSetter, "test")
            .await
            .expect("activation succeeds");

        let snapshot = state.video_source_status().await.expect("status snapshot");
        let entry = snapshot.sources.first().expect("one source");
        assert_eq!(entry.state, "live");
        assert_eq!(entry.detail, None);
    }

    /// THE ACTIVATION WINDOW (deep review 🟡 #1). `start_pipeline` holds the manager's
    /// lock across its 8 s caps-wait, so a status poll landing in that window cannot read
    /// the snapshot map at all. Reading "cannot look" as "no pipeline" painted the HAPPY
    /// path amber and told the operator to go start an NDI output that was already on.
    #[tokio::test]
    async fn video_source_status_says_connecting_while_the_manager_is_busy_starting() {
        let (state, source_id, _id, fake) = state_with_fake(StartOutcome::Ok).await;
        fake.set_discovered(&["STREAM-SNV (stream)"]);
        state
            .activate_video_source(source_id, SettingsAuditSource::HttpSetter, "test")
            .await
            .expect("activation succeeds");
        // The manager is mid-start: its lock is held, so the snapshot times out.
        fake.set_snapshots_unreadable();

        let snapshot = state.video_source_status().await.expect("status snapshot");
        let entry = snapshot.sources.first().expect("one source");
        assert_eq!(
            entry.state, "connecting",
            "a busy manager means the pipeline is coming up — NOT that the sending \
             machine is silent",
        );
    }

    /// Deep review 🟡 #2: a blind finder leaves us blind. Degrading it to an empty network
    /// would make a broken server tell the operator that every sending machine at the site
    /// is off — the exact false accusation this module exists to prevent.
    ///
    /// The FIRST cut of this fix keyed on `discover_sources()` returning `Err` — which the
    /// real `NdiManager` never does (it is `Ok(self.source_list.read())`). So the blindness
    /// production ACTUALLY has — the finder thread never started (`NDIlib_find_create_v2`
    /// returned null), or has not completed its first scan yet (every restart) — still read
    /// as an empty network. The seam now asks the finder whether it has ever scanned.
    #[tokio::test]
    async fn video_source_status_says_unknown_when_the_finder_has_never_scanned() {
        let (state, _source_id, _id, fake) = state_with_fake(StartOutcome::Ok).await;
        fake.finder_never_scanned();

        let snapshot = state.video_source_status().await.expect("status snapshot");

        assert!(
            !snapshot.ndi_available,
            "a finder we cannot query is a network we cannot see",
        );
        assert!(snapshot.discovered.is_empty());
        assert_eq!(
            snapshot.sources.first().map(|s| s.state),
            Some("unknown"),
            "a discovery failure must never render as 'not found on the network'",
        );
    }

    /// The other half of the same rule: once the finder HAS scanned, an empty list is a
    /// fact about the network — nothing is broadcasting — and the mapped source really is
    /// not found. Blindness must not swallow the genuine PP answer.
    #[tokio::test]
    async fn video_source_status_still_says_not_found_when_a_scanned_network_is_empty() {
        let (state, source_id, _id, fake) = state_with_fake(StartOutcome::SilentSource).await;
        fake.set_discovered(&[]); // the finder looked; nobody is on the air
        state
            .activate_video_source(source_id, SettingsAuditSource::HttpSetter, "test")
            .await
            .expect("a silent source still activates (#448)");

        let snapshot = state.video_source_status().await.expect("status snapshot");

        assert!(
            snapshot.ndi_available,
            "we CAN see the network — it is just empty"
        );
        assert_eq!(
            snapshot.sources.first().map(|s| s.state),
            Some("not-found"),
            "a scanned, empty network means the mapped name really is not there",
        );
    }

    #[tokio::test]
    async fn video_source_status_says_unknown_when_there_is_no_ndi_sdk() {
        // The libndi-free host (GH runners, and any server without the SDK): we cannot
        // see the network, so we must not accuse the sending machine. Cleared
        // explicitly — `AppState::new` picks the handle up from the HOST, and dev2 has
        // libndi while the CI runners do not.
        let mut state = AppState::in_memory().await.expect("in-memory AppState");
        state.clear_ndi_handle();
        state
            .create_video_source(
                VideoSourceDraft::new("Cam 1", "STREAM-SNV (stream)"),
                SettingsAuditSource::HttpSetter,
                "test",
            )
            .await
            .expect("create video source");

        let snapshot = state.video_source_status().await.expect("status snapshot");

        assert!(!snapshot.ndi_available);
        assert!(snapshot.discovered.is_empty());
        assert_eq!(
            snapshot.sources.first().map(|s| s.state),
            Some("unknown"),
            "without the SDK the honest answer is 'we cannot see', never 'not found'",
        );
    }
}
