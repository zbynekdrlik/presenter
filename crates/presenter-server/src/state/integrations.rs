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

    pub async fn create_resolume_host(
        &self,
        draft: ResolumeHostDraft,
    ) -> anyhow::Result<ResolumeHost> {
        let host = self.repository.create_resolume_host(&draft).await?;
        self.sync_resolume_hosts().await?;
        Ok(host)
    }

    pub async fn update_resolume_host(
        &self,
        id: ResolumeHostId,
        draft: ResolumeHostDraft,
    ) -> anyhow::Result<ResolumeHost> {
        let host = self.repository.update_resolume_host(id, &draft).await?;
        self.sync_resolume_hosts().await?;
        Ok(host)
    }

    pub async fn delete_resolume_host(&self, id: ResolumeHostId) -> anyhow::Result<()> {
        self.repository.delete_resolume_host(id).await?;
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
    ) -> anyhow::Result<AndroidStageDisplay> {
        let display = self.repository.create_android_stage_display(&draft).await?;
        self.sync_android_stage_displays().await?;
        Ok(display)
    }

    pub async fn update_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
        draft: AndroidStageDisplayDraft,
    ) -> anyhow::Result<AndroidStageDisplay> {
        let display = self
            .repository
            .update_android_stage_display(id, &draft)
            .await?;
        self.sync_android_stage_displays().await?;
        Ok(display)
    }

    pub async fn delete_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
    ) -> anyhow::Result<()> {
        self.repository.delete_android_stage_display(id).await?;
        self.sync_android_stage_displays().await
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
    ) -> anyhow::Result<VideoSource> {
        self.repository.create_video_source(&draft).await
    }

    pub async fn update_video_source(
        &self,
        id: VideoSourceId,
        draft: VideoSourceDraft,
    ) -> anyhow::Result<VideoSource> {
        self.repository.update_video_source(id, &draft).await
    }

    pub async fn delete_video_source(&self, id: VideoSourceId) -> anyhow::Result<()> {
        self.repository.delete_video_source(id).await
    }

    pub async fn activate_video_source(&self, id: VideoSourceId) -> anyhow::Result<VideoSource> {
        let source = self.repository.activate_video_source(id).await?;
        self.live_hub.publish(LiveEvent::NdiSourceActivated {
            ndi_name: source.ndi_name.clone(),
            label: source.label.clone(),
        });
        Ok(source)
    }

    pub async fn deactivate_video_sources(&self) -> anyhow::Result<()> {
        self.repository.deactivate_all_video_sources().await?;
        self.live_hub.publish(LiveEvent::NdiSourceDeactivated);
        Ok(())
    }
}
