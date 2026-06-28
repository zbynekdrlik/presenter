use super::Repository;
use crate::audit::{ResolumePushAuditEntry, SettingsAuditSource};
use chrono::Utc;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
use tracing::instrument;

impl Repository {
    #[instrument(skip(self, before, after))]
    pub async fn record_settings_audit(
        &self,
        setting_table: &'static str,
        setting_id: &str,
        source: SettingsAuditSource,
        actor: &str,
        before: Option<serde_json::Value>,
        after: serde_json::Value,
    ) -> anyhow::Result<()> {
        Self::record_settings_audit_on(
            &self.db,
            setting_table,
            setting_id,
            source,
            actor,
            before,
            after,
        )
        .await
    }

    /// Audit-row insert that runs on any `ConnectionTrait` — used both by the
    /// public helper above (which passes `&self.db`) and by audited setters
    /// that wrap the settings write + audit insert in a single transaction.
    pub(super) async fn record_settings_audit_on<C: ConnectionTrait>(
        conn: &C,
        setting_table: &str,
        setting_id: &str,
        source: SettingsAuditSource,
        actor: &str,
        before: Option<serde_json::Value>,
        after: serde_json::Value,
    ) -> anyhow::Result<()> {
        use crate::entities::settings_audit;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let active = settings_audit::ActiveModel {
            id: sea_orm::ActiveValue::set(id),
            setting_table: sea_orm::ActiveValue::set(setting_table.to_string()),
            setting_id: sea_orm::ActiveValue::set(setting_id.to_string()),
            source: sea_orm::ActiveValue::set(source.as_str().to_string()),
            actor: sea_orm::ActiveValue::set(actor.to_string()),
            before_json: sea_orm::ActiveValue::set(before.map(|v| v.to_string())),
            after_json: sea_orm::ActiveValue::set(after.to_string()),
            changed_at: sea_orm::ActiveValue::set(now.into()),
        };
        crate::entities::settings_audit::Entity::insert(active)
            .exec(conn)
            .await?;
        Ok(())
    }

    /// #483: append one Resolume push-audit row. The `id` is generated here;
    /// callers leave `entry` id-less. Append-only — never updated or deleted.
    #[instrument(skip(self, entry), fields(host = %entry.host))]
    pub async fn record_resolume_push_audit(
        &self,
        entry: &ResolumePushAuditEntry,
    ) -> anyhow::Result<()> {
        use crate::entities::resolume_push_audit;
        let active = resolume_push_audit::ActiveModel {
            id: sea_orm::ActiveValue::set(uuid::Uuid::new_v4().to_string()),
            correlation_id: sea_orm::ActiveValue::set(entry.correlation_id.clone()),
            host: sea_orm::ActiveValue::set(entry.host.clone()),
            t_queue_wait_ms: sea_orm::ActiveValue::set(entry.t_queue_wait_ms),
            t_ensure_mapping_ms: sea_orm::ActiveValue::set(entry.t_ensure_mapping_ms),
            t_total_ms: sea_orm::ActiveValue::set(entry.t_total_ms),
            refetched: sea_orm::ActiveValue::set(entry.refetched),
            outcome: sea_orm::ActiveValue::set(entry.outcome.clone()),
            created_at: sea_orm::ActiveValue::set(entry.created_at.into()),
        };
        resolume_push_audit::Entity::insert(active)
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// #483: most-recent push-audit rows (newest first), optionally scoped to a
    /// host and/or a `since` cutoff. For post-event latency analysis.
    #[instrument(skip(self))]
    pub async fn list_resolume_push_audit(
        &self,
        host: Option<&str>,
        since: Option<chrono::DateTime<chrono::Utc>>,
        limit: u64,
    ) -> anyhow::Result<Vec<ResolumePushAuditEntry>> {
        use crate::entities::resolume_push_audit;
        use sea_orm::QuerySelect;
        let mut q = resolume_push_audit::Entity::find()
            .order_by_desc(resolume_push_audit::Column::CreatedAt)
            .limit(limit);
        if let Some(h) = host {
            q = q.filter(resolume_push_audit::Column::Host.eq(h));
        }
        if let Some(t) = since {
            let stamp: chrono::DateTime<chrono::FixedOffset> = t.into();
            q = q.filter(resolume_push_audit::Column::CreatedAt.gte(stamp));
        }
        let rows = q.all(&self.db).await?;
        Ok(rows
            .into_iter()
            .map(|m| ResolumePushAuditEntry {
                correlation_id: m.correlation_id,
                host: m.host,
                t_queue_wait_ms: m.t_queue_wait_ms,
                t_ensure_mapping_ms: m.t_ensure_mapping_ms,
                t_total_ms: m.t_total_ms,
                refetched: m.refetched,
                outcome: m.outcome,
                created_at: m.created_at.into(),
            })
            .collect())
    }

    #[instrument(skip(self))]
    pub async fn list_settings_audit(
        &self,
        setting_table: Option<&str>,
        setting_id: Option<&str>,
        since: Option<chrono::DateTime<chrono::Utc>>,
        limit: u64,
    ) -> anyhow::Result<Vec<crate::audit::SettingsAuditEntry>> {
        use crate::entities::settings_audit;
        use sea_orm::QuerySelect;
        let mut q = settings_audit::Entity::find()
            .order_by_desc(settings_audit::Column::ChangedAt)
            .limit(limit);
        if let Some(t) = setting_table {
            q = q.filter(settings_audit::Column::SettingTable.eq(t));
        }
        if let Some(id) = setting_id {
            q = q.filter(settings_audit::Column::SettingId.eq(id));
        }
        if let Some(t) = since {
            let stamp: chrono::DateTime<chrono::FixedOffset> = t.into();
            q = q.filter(settings_audit::Column::ChangedAt.gte(stamp));
        }
        let rows = q.all(&self.db).await?;
        rows.into_iter()
            .map(|m| {
                let source = match m.source.as_str() {
                    "http_setter" => SettingsAuditSource::HttpSetter,
                    "companion_setter" => SettingsAuditSource::CompanionSetter,
                    "startup_default" => SettingsAuditSource::StartupDefault,
                    "schema_migration" => SettingsAuditSource::SchemaMigration,
                    other => anyhow::bail!("unknown source: {other}"),
                };
                Ok(crate::audit::SettingsAuditEntry {
                    id: m.id,
                    setting_table: m.setting_table,
                    setting_id: m.setting_id,
                    source,
                    actor: m.actor,
                    before_json: m
                        .before_json
                        .map(|s| serde_json::from_str(&s))
                        .transpose()?,
                    after_json: serde_json::from_str(&m.after_json)?,
                    changed_at: m.changed_at.into(),
                })
            })
            .collect()
    }
}
