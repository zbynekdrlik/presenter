use std::collections::HashMap;

use presenter_core::{
    AndroidStageDisplay, AndroidStageDisplayDraft, AndroidStageDisplayId, LiveEvent, ResolumeHost,
    ResolumeHostDraft, ResolumeHostId, VideoSource, VideoSourceDraft, VideoSourceId,
};

use presenter_ndi::PipelineStartError;

use super::AppState;
use crate::android_stage::AndroidStageDisplayStatusSnapshot;
use crate::resolume::ResolumeConnectionSnapshot;

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
            .ok_or_else(|| anyhow::anyhow!("Resolume host not found"))?;
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
}

#[cfg(test)]
mod tests {
    use super::{ndi_status_for_start_error, PipelineStartError};

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
}
