use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryOrder, Set};
use tracing::instrument;

use crate::entities::android_stage_display;
use presenter_core::{AndroidStageDisplay, AndroidStageDisplayDraft, AndroidStageDisplayId};

use super::util::android_stage_display_model_to_domain;
use super::Repository;

impl Repository {
    pub async fn list_android_stage_displays(&self) -> anyhow::Result<Vec<AndroidStageDisplay>> {
        let models = android_stage_display::Entity::find()
            .order_by_asc(android_stage_display::Column::Label)
            .all(&self.db)
            .await?;
        models
            .into_iter()
            .map(android_stage_display_model_to_domain)
            .collect()
    }

    #[instrument(skip_all)]
    pub async fn create_android_stage_display(
        &self,
        draft: &AndroidStageDisplayDraft,
    ) -> anyhow::Result<AndroidStageDisplay> {
        draft.validate().map_err(|err| anyhow::anyhow!(err))?;
        let id = AndroidStageDisplayId::new();
        let now = chrono::Utc::now();
        let model = android_stage_display::ActiveModel {
            id: Set(id.to_string()),
            label: Set(draft.label.trim().to_string()),
            host: Set(draft.host.trim().to_string()),
            port: Set(draft.port as i32),
            launch_component: Set(draft.launch_component.trim().to_string()),
            is_enabled: Set(draft.is_enabled),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };
        android_stage_display::Entity::insert(model)
            .exec(&self.db)
            .await?;
        let inserted = android_stage_display::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Failed to find inserted item"))?;
        android_stage_display_model_to_domain(inserted)
    }

    #[instrument(skip_all)]
    pub async fn update_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
        draft: &AndroidStageDisplayDraft,
    ) -> anyhow::Result<AndroidStageDisplay> {
        draft.validate().map_err(|err| anyhow::anyhow!(err))?;
        let now = chrono::Utc::now();
        let existing = android_stage_display::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow::anyhow!("android stage display not found"))?;
        let mut model = existing.into_active_model();
        model.label = Set(draft.label.trim().to_string());
        model.host = Set(draft.host.trim().to_string());
        model.port = Set(draft.port as i32);
        model.launch_component = Set(draft.launch_component.trim().to_string());
        model.is_enabled = Set(draft.is_enabled);
        model.updated_at = Set(now.into());
        let updated = model.update(&self.db).await?;
        android_stage_display_model_to_domain(updated)
    }

    #[instrument(skip_all)]
    pub async fn delete_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
    ) -> anyhow::Result<()> {
        android_stage_display::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
