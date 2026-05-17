use super::Repository;
use crate::audit::SettingsAuditSource;
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
