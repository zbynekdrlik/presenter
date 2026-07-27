//! Android stage display CRUD. Split out of `repository/mod.rs` (#590 — the
//! file crossed the 800-line warning cap) — same pattern as
//! `resolume.rs`/`audit.rs`: a self-contained `impl Repository` block in its
//! own file, calling the shared `Self::record_settings_audit_on` helper
//! defined in `audit.rs`.

use super::util::{android_stage_display_model_to_domain, RepositoryError};
use super::Repository;
use crate::audit::SettingsAuditSource;
use crate::entities::android_stage_display;
use anyhow::anyhow;
use chrono::Utc;
use presenter_core::{AndroidStageDisplay, AndroidStageDisplayDraft, AndroidStageDisplayId};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, QueryOrder, Set, TransactionTrait};

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

    pub async fn create_android_stage_display(
        &self,
        draft: &AndroidStageDisplayDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<AndroidStageDisplay> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let id = AndroidStageDisplayId::new();
        let now = Utc::now();
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

        let txn = self.db.begin().await?;
        android_stage_display::Entity::insert(model)
            .exec(&txn)
            .await?;

        let inserted = android_stage_display::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("android stage display missing after insert"))?;
        let display = android_stage_display_model_to_domain(inserted)?;
        let after_json = serde_json::to_value(&display)?;
        Self::record_settings_audit_on(
            &txn,
            "android_stage_display",
            &id.to_string(),
            source,
            actor,
            None,
            after_json,
        )
        .await?;
        txn.commit().await?;
        Ok(display)
    }

    pub async fn update_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
        draft: &AndroidStageDisplayDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<AndroidStageDisplay> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let txn = self.db.begin().await?;
        let existing = android_stage_display::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
            // #586: typed refusal (#584 pattern).
            .ok_or(RepositoryError::NotFound("android stage display not found"))?;
        let before = android_stage_display_model_to_domain(existing.clone())?;
        let before_json = serde_json::to_value(&before)?;

        let mut model = existing.into_active_model();
        model.label = Set(draft.label.trim().to_string());
        model.host = Set(draft.host.trim().to_string());
        model.port = Set(draft.port as i32);
        model.launch_component = Set(draft.launch_component.trim().to_string());
        model.is_enabled = Set(draft.is_enabled);
        model.updated_at = Set(Utc::now().into());

        let updated = model.update(&txn).await?;
        let display = android_stage_display_model_to_domain(updated)?;
        let after_json = serde_json::to_value(&display)?;
        Self::record_settings_audit_on(
            &txn,
            "android_stage_display",
            &id.to_string(),
            source,
            actor,
            Some(before_json),
            after_json,
        )
        .await?;
        txn.commit().await?;
        Ok(display)
    }

    pub async fn delete_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let existing = android_stage_display::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?;
        let before_json = existing
            .map(|m| {
                let display = android_stage_display_model_to_domain(m)?;
                serde_json::to_value(&display).map_err(anyhow::Error::from)
            })
            .transpose()?;

        let result = android_stage_display::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        if result.rows_affected == 0 {
            return Err(RepositoryError::NotFound("android stage display not found").into());
        }
        Self::record_settings_audit_on(
            &txn,
            "android_stage_display",
            &id.to_string(),
            source,
            actor,
            before_json,
            serde_json::json!({"deleted": true, "id": id.to_string()}),
        )
        .await?;
        txn.commit().await?;
        Ok(())
    }
}
