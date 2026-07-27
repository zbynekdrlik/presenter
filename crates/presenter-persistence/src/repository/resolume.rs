//! Resolume host CRUD + the #564 port-drift persistence. Split out of
//! `repository/mod.rs` (which crossed the 1000-line hard cap once the
//! two-port model landed) — same pattern as `audit.rs`/`stage_state.rs`:
//! a self-contained `impl Repository` block in its own file.

use super::util::{resolume_model_to_domain, RepositoryError};
use super::Repository;
use crate::audit::SettingsAuditSource;
use crate::entities::resolume_host;
use anyhow::anyhow;
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostDraft, ResolumeHostId};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, QueryOrder, Set, TransactionTrait};
use tracing::instrument;

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
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<ResolumeHost> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let id = ResolumeHostId::new();
        let now = Utc::now();
        let model = resolume_host::ActiveModel {
            id: Set(id.to_string()),
            label: Set(draft.label.trim().to_string()),
            host: Set(draft.host.trim().to_string()),
            port: Set(draft.port as i32),
            is_enabled: Set(draft.is_enabled),
            active_port: Set(None),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };

        let txn = self.db.begin().await?;
        resolume_host::Entity::insert(model).exec(&txn).await?;

        let inserted = resolume_host::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("resolume host missing after insert"))?;
        let host = resolume_model_to_domain(inserted)?;
        let after_json = serde_json::to_value(&host)?;
        Self::record_settings_audit_on(
            &txn,
            "resolume_host",
            &id.to_string(),
            source,
            actor,
            None,
            after_json,
        )
        .await?;
        txn.commit().await?;
        Ok(host)
    }

    #[instrument(skip_all)]
    pub async fn update_resolume_host(
        &self,
        id: ResolumeHostId,
        draft: &ResolumeHostDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<ResolumeHost> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let txn = self.db.begin().await?;
        let existing = resolume_host::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
            // #586: typed refusal — the router downcasts to
            // `RepositoryError` and maps `NotFound` to 404 instead of
            // matching a string (#584 pattern).
            .ok_or(RepositoryError::NotFound("resolume host not found"))?;
        let before = resolume_model_to_domain(existing.clone())?;
        let before_json = serde_json::to_value(&before)?;
        // #564: an explicit host/port edit invalidates any previously
        // DISCOVERED active_port — it was learned against the OLD
        // host/port pair, so it is stale (and possibly actively wrong) once
        // the admin repoints the config. Editing only label/is_enabled
        // leaves a legitimately-discovered drift in place.
        let clears_active_port =
            existing.host != draft.host.trim() || existing.port != draft.port as i32;

        let mut model = existing.into_active_model();
        model.label = Set(draft.label.trim().to_string());
        model.host = Set(draft.host.trim().to_string());
        model.port = Set(draft.port as i32);
        model.is_enabled = Set(draft.is_enabled);
        if clears_active_port {
            model.active_port = Set(None);
        }
        model.updated_at = Set(Utc::now().into());

        let updated = model.update(&txn).await?;
        let host = resolume_model_to_domain(updated)?;
        let after_json = serde_json::to_value(&host)?;
        Self::record_settings_audit_on(
            &txn,
            "resolume_host",
            &id.to_string(),
            source,
            actor,
            Some(before_json),
            after_json,
        )
        .await?;
        txn.commit().await?;
        Ok(host)
    }

    /// #564: persist a runtime port-drift discovery (or a heal-back to the
    /// configured port when `active_port` is `None`). Always audited with
    /// `source = SettingsAuditSource::PortDriftDiscovery` regardless of the
    /// caller-supplied `source` — the CALLER is the driver's own background
    /// probe, never a human actor, so the audit trail must say so honestly
    /// rather than inherit whatever the last human-facing source happened
    /// to be.
    #[instrument(skip(self))]
    pub async fn update_resolume_host_active_port(
        &self,
        id: ResolumeHostId,
        active_port: Option<u16>,
    ) -> anyhow::Result<ResolumeHost> {
        let txn = self.db.begin().await?;
        let existing = resolume_host::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("resolume host not found"))?;
        let before = resolume_model_to_domain(existing.clone())?;
        let before_json = serde_json::to_value(&before)?;

        let mut model = existing.into_active_model();
        model.active_port = Set(active_port.map(|p| p as i32));
        model.updated_at = Set(Utc::now().into());

        let updated = model.update(&txn).await?;
        let host = resolume_model_to_domain(updated)?;
        let after_json = serde_json::to_value(&host)?;
        Self::record_settings_audit_on(
            &txn,
            "resolume_host",
            &id.to_string(),
            SettingsAuditSource::PortDriftDiscovery,
            "resolume-driver",
            Some(before_json),
            after_json,
        )
        .await?;
        txn.commit().await?;
        Ok(host)
    }

    #[instrument(skip_all)]
    pub async fn delete_resolume_host(
        &self,
        id: ResolumeHostId,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let existing = resolume_host::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?;
        let before_json = existing
            .map(|m| {
                let host = resolume_model_to_domain(m)?;
                serde_json::to_value(&host).map_err(anyhow::Error::from)
            })
            .transpose()?;

        let result = resolume_host::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        if result.rows_affected == 0 {
            // #586: typed refusal (#584 pattern) — see update_resolume_host above.
            return Err(RepositoryError::NotFound("resolume host not found").into());
        }
        Self::record_settings_audit_on(
            &txn,
            "resolume_host",
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
