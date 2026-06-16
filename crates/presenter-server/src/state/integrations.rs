use std::collections::HashMap;

use presenter_core::{
    AndroidStageDisplay, AndroidStageDisplayDraft, AndroidStageDisplayId, LiveEvent, ResolumeHost,
    ResolumeHostDraft, ResolumeHostId, VideoSource, VideoSourceDraft, VideoSourceId,
};

use super::AppState;
use crate::android_stage::AndroidStageDisplayStatusSnapshot;
use crate::resolume::ResolumeConnectionSnapshot;

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
                // Surface the failure to the stage view so the operator sees
                // the actual reason instead of an endless "Connecting…"
                // overlay. The DB row stays `is_active=true` so the operator
                // can retry by toggling off+on once the issue is fixed.
                tracing::error!(
                    error = %e,
                    source_id = %source.id,
                    ndi_name = %source.ndi_name,
                    "NDI pipeline start failed"
                );
                self.live_hub.publish(LiveEvent::NdiConnectionStatus {
                    status: format!("failed: {e}"),
                });
                return Err(e);
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
