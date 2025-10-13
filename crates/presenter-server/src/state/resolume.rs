use std::collections::HashMap;

use presenter_core::{ResolumeHost, ResolumeHostDraft, ResolumeHostId};

use crate::resolume::ResolumeConnectionSnapshot;

use super::AppState;

impl AppState {
    pub async fn list_resolume_hosts(&self) -> anyhow::Result<Vec<ResolumeHost>> {
        self.repository.list_resolume_hosts().await
    }

    pub async fn resolume_status_snapshot(
        &self,
    ) -> HashMap<ResolumeHostId, ResolumeConnectionSnapshot> {
        self.resolume_client.snapshot().await
    }

    pub async fn resolume_status_for(&self, id: ResolumeHostId) -> ResolumeConnectionSnapshot {
        self.resolume_client.snapshot_for(id).await
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
        self.resolume_client.set_hosts(hosts).await;
        Ok(())
    }
}
