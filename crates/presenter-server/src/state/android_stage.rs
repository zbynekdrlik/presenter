use std::collections::HashMap;

use presenter_core::{AndroidStageDisplay, AndroidStageDisplayDraft, AndroidStageDisplayId};

use crate::android_stage::AndroidStageDisplayStatusSnapshot;

use super::AppState;

impl AppState {
    pub async fn list_android_stage_displays(&self) -> anyhow::Result<Vec<AndroidStageDisplay>> {
        self.repository.list_android_stage_displays().await
    }

    pub async fn android_stage_status_snapshot(
        &self,
    ) -> HashMap<AndroidStageDisplayId, AndroidStageDisplayStatusSnapshot> {
        self.android_stage_client.snapshot().await
    }

    pub async fn android_stage_status_for(
        &self,
        id: AndroidStageDisplayId,
    ) -> AndroidStageDisplayStatusSnapshot {
        self.android_stage_client.snapshot_for(id).await
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
        self.android_stage_client.set_displays(displays).await;
        Ok(())
    }
}
