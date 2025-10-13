use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, QueryOrder, Set};
use tracing::instrument;

use crate::entities::resolume_host;
use presenter_core::{ResolumeHost, ResolumeHostDraft, ResolumeHostId};

use super::util::resolume_model_to_domain;
use super::Repository;

impl Repository {
    pub async fn list_resolume_hosts(&self) -> anyhow::Result<Vec<ResolumeHost>> {
        let models = resolume_host::Entity::find()
            .order_by_asc(resolume_host::Column::Label)
            .all(&self.db)
            .await?;
        models.into_iter().map(resolume_model_to_domain).collect()
    }

    #[instrument(skip_all)]
    pub async fn create_resolume_host(
        &self,
        draft: &ResolumeHostDraft,
    ) -> anyhow::Result<ResolumeHost> {
        draft.validate().map_err(|err| anyhow::anyhow!(err))?;
        let id = ResolumeHostId::new();
        let now = chrono::Utc::now();
        let model = resolume_host::ActiveModel {
            id: Set(id.to_string()),
            label: Set(draft.label.trim().to_string()),
            host: Set(draft.host.trim().to_string()),
            port: Set(draft.port as i32),
            is_enabled: Set(draft.is_enabled),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };
        resolume_host::Entity::insert(model).exec(&self.db).await?;
        let inserted = resolume_host::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Failed to find inserted item"))?;
        resolume_model_to_domain(inserted)
    }

    #[instrument(skip_all)]
    pub async fn update_resolume_host(
        &self,
        id: ResolumeHostId,
        draft: &ResolumeHostDraft,
    ) -> anyhow::Result<ResolumeHost> {
        draft.validate().map_err(|err| anyhow::anyhow!(err))?;
        let now = chrono::Utc::now();
        let existing = resolume_host::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow::anyhow!("resolume host not found"))?;
        let mut model = existing.into_active_model();
        model.label = Set(draft.label.trim().to_string());
        model.host = Set(draft.host.trim().to_string());
        model.port = Set(draft.port as i32);
        model.is_enabled = Set(draft.is_enabled);
        model.updated_at = Set(now.into());
        let updated = model.update(&self.db).await?;
        resolume_model_to_domain(updated)
    }

    #[instrument(skip_all)]
    pub async fn delete_resolume_host(&self, id: ResolumeHostId) -> anyhow::Result<()> {
        resolume_host::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
