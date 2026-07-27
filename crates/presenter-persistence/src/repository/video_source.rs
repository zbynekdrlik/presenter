//! Video source CRUD + activation. Split out of `repository/mod.rs` (#590 —
//! the file crossed the 800-line warning cap) — same pattern as
//! `resolume.rs`/`audit.rs`: a self-contained `impl Repository` block in its
//! own file, calling the shared `Self::record_settings_audit_on` helper
//! defined in `audit.rs`.

use super::util::{video_source_model_to_domain, RepositoryError};
use super::Repository;
use crate::audit::SettingsAuditSource;
use crate::entities::video_source;
use anyhow::anyhow;
use chrono::Utc;
use presenter_core::{VideoSource, VideoSourceDraft, VideoSourceId};
use sea_orm::{
    sea_query::Expr, ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set, TransactionTrait,
};
use tracing::instrument;

impl Repository {
    pub async fn list_video_sources(&self) -> anyhow::Result<Vec<VideoSource>> {
        let models = video_source::Entity::find()
            .order_by_asc(video_source::Column::Label)
            .all(&self.db)
            .await?;
        models
            .into_iter()
            .map(video_source_model_to_domain)
            .collect()
    }

    pub async fn get_active_video_source(&self) -> anyhow::Result<Option<VideoSource>> {
        let model = video_source::Entity::find()
            .filter(video_source::Column::IsActive.eq(true))
            .one(&self.db)
            .await?;
        model.map(video_source_model_to_domain).transpose()
    }

    #[instrument(skip_all)]
    pub async fn create_video_source(
        &self,
        draft: &VideoSourceDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<VideoSource> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let id = VideoSourceId::new();
        let now = Utc::now();
        let model = video_source::ActiveModel {
            id: Set(id.to_string()),
            label: Set(draft.label.trim().to_string()),
            ndi_name: Set(draft.ndi_name.trim().to_string()),
            is_active: Set(false),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };

        let txn = self.db.begin().await?;
        video_source::Entity::insert(model).exec(&txn).await?;

        let inserted = video_source::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("video source missing after insert"))?;
        let domain = video_source_model_to_domain(inserted)?;
        let after_json = serde_json::to_value(&domain)?;
        Self::record_settings_audit_on(
            &txn,
            "video_source",
            &id.to_string(),
            source,
            actor,
            None,
            after_json,
        )
        .await?;
        txn.commit().await?;
        Ok(domain)
    }

    #[instrument(skip_all)]
    pub async fn update_video_source(
        &self,
        id: VideoSourceId,
        draft: &VideoSourceDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<VideoSource> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let txn = self.db.begin().await?;
        let existing = video_source::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
            // #586: typed refusal (#584 pattern).
            .ok_or(RepositoryError::NotFound("video source not found"))?;
        let before = video_source_model_to_domain(existing.clone())?;
        let before_json = serde_json::to_value(&before)?;

        let mut model = existing.into_active_model();
        model.label = Set(draft.label.trim().to_string());
        model.ndi_name = Set(draft.ndi_name.trim().to_string());
        model.updated_at = Set(Utc::now().into());

        let updated = model.update(&txn).await?;
        let domain = video_source_model_to_domain(updated)?;
        let after_json = serde_json::to_value(&domain)?;
        Self::record_settings_audit_on(
            &txn,
            "video_source",
            &id.to_string(),
            source,
            actor,
            Some(before_json),
            after_json,
        )
        .await?;
        txn.commit().await?;
        Ok(domain)
    }

    #[instrument(skip_all)]
    pub async fn delete_video_source(
        &self,
        id: VideoSourceId,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let existing = video_source::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?;
        let before_json = existing
            .map(|m| {
                let domain = video_source_model_to_domain(m)?;
                serde_json::to_value(&domain).map_err(anyhow::Error::from)
            })
            .transpose()?;

        let result = video_source::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        if result.rows_affected == 0 {
            return Err(RepositoryError::NotFound("video source not found").into());
        }
        Self::record_settings_audit_on(
            &txn,
            "video_source",
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

    #[instrument(skip_all)]
    pub async fn activate_video_source(
        &self,
        id: VideoSourceId,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<VideoSource> {
        // Activating one row deactivates every other currently-active row.
        // Each deactivation MUST produce its own audit row so the forensic
        // log can attribute who turned what off, when. Wrap the whole
        // sequence (sibling deactivations + target activation + all audit
        // inserts) in a single transaction so a mid-flight failure can't
        // leave the DB with deactivated rows whose audit rows are missing.
        let txn = self.db.begin().await?;

        // Capture before state of the target row.
        let existing = video_source::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
            // #586: typed refusal (#584 pattern).
            .ok_or(RepositoryError::NotFound("video source not found"))?;
        let target_before = video_source_model_to_domain(existing.clone())?;
        let target_before_json = serde_json::to_value(&target_before)?;

        // Collect every CURRENTLY-active sibling (excluding the target).
        let target_id_str = id.to_string();
        let siblings: Vec<_> = video_source::Entity::find()
            .filter(video_source::Column::IsActive.eq(true))
            .filter(video_source::Column::Id.ne(target_id_str.clone()))
            .all(&txn)
            .await?;

        // #375: idempotency guard. When the target is ALREADY active and there
        // is no sibling to deactivate, this activation changes nothing (before
        // == after). Returning early — without bumping `updated_at` and without
        // writing an audit row — keeps the settings_audit log a record of real
        // CHANGES, not no-ops. The NDI 30s auto-reconnect loop re-calls this
        // every tick while an active source is unreachable (its pipeline start
        // fails but the DB row stays `is_active=true`); without this guard that
        // wrote one audit row every ~30s forever. A genuine activate (the row
        // was inactive) or switch (a sibling is active) still falls through and
        // writes exactly the audit rows it always did.
        if target_before.is_active && siblings.is_empty() {
            txn.commit().await?;
            return Ok(target_before);
        }

        // Deactivate each sibling individually so we can record per-row
        // before/after JSON for the audit log. Using `update_many` would
        // erase the per-row identity we need to log.
        let now = Utc::now();
        for sibling in siblings {
            let sibling_id = sibling.id.clone();
            let sibling_before = video_source_model_to_domain(sibling.clone())?;
            let sibling_before_json = serde_json::to_value(&sibling_before)?;

            let mut active_model = sibling.into_active_model();
            active_model.is_active = Set(false);
            active_model.updated_at = Set(now.into());
            let updated_sibling = active_model.update(&txn).await?;
            let sibling_after = video_source_model_to_domain(updated_sibling)?;
            let sibling_after_json = serde_json::to_value(&sibling_after)?;

            Self::record_settings_audit_on(
                &txn,
                "video_source",
                &sibling_id,
                source,
                actor,
                Some(sibling_before_json),
                sibling_after_json,
            )
            .await?;
        }

        // Now activate the target. Audit row reflects the target's own
        // before/after.
        let mut model = existing.into_active_model();
        model.is_active = Set(true);
        model.updated_at = Set(now.into());
        let updated = model.update(&txn).await?;
        let domain = video_source_model_to_domain(updated)?;
        let after_json = serde_json::to_value(&domain)?;
        Self::record_settings_audit_on(
            &txn,
            "video_source",
            &target_id_str,
            source,
            actor,
            Some(target_before_json),
            after_json,
        )
        .await?;

        txn.commit().await?;
        Ok(domain)
    }

    pub async fn deactivate_all_video_sources(
        &self,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        // Wrap deactivation + audit insert in one transaction.
        let txn = self.db.begin().await?;

        // Capture before state — list of active sources.
        let active_rows: Vec<_> = video_source::Entity::find()
            .filter(video_source::Column::IsActive.eq(true))
            .all(&txn)
            .await?;
        let before_list: Vec<serde_json::Value> = active_rows
            .iter()
            .cloned()
            .map(|m| {
                let domain = video_source_model_to_domain(m)?;
                serde_json::to_value(&domain).map_err(anyhow::Error::from)
            })
            .collect::<anyhow::Result<_>>()?;

        video_source::Entity::update_many()
            .col_expr(video_source::Column::IsActive, Expr::value(false))
            .col_expr(
                video_source::Column::UpdatedAt,
                Expr::value(Into::<sea_orm::prelude::DateTimeWithTimeZone>::into(
                    Utc::now(),
                )),
            )
            .filter(video_source::Column::IsActive.eq(true))
            .exec(&txn)
            .await?;

        if !active_rows.is_empty() {
            Self::record_settings_audit_on(
                &txn,
                "video_source",
                "deactivate_all",
                source,
                actor,
                Some(serde_json::Value::Array(before_list)),
                serde_json::json!({"deactivated_all": true}),
            )
            .await?;
        }
        txn.commit().await?;
        Ok(())
    }
}
