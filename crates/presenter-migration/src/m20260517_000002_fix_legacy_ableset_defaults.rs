use sea_orm_migration::prelude::*;

const NEW_HTTP_PORT: i32 = 80;
const NEW_OSC_PORT: i32 = 39051;
const NEW_LIBRARY_NAME: &str = "NEW LEVEL";

const AUDIT_SOURCE: &str = "schema_migration";
const AUDIT_ACTOR: &str = "migration";
const AUDIT_TABLE: &str = "ableset_settings";

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Snapshot of an `ableset_settings` row used to compute before/after JSON for
/// the audit log. Only includes columns this migration may rewrite plus the
/// primary key.
struct AbleSetRow {
    id: String,
    enabled: bool,
    host: String,
    osc_port: i32,
    http_port: i32,
    library_name: String,
    song_prefix_length: i32,
}

impl AbleSetRow {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "enabled": self.enabled,
            "host": self.host,
            "oscPort": self.osc_port,
            "httpPort": self.http_port,
            "libraryName": self.library_name,
            "songPrefixLength": self.song_prefix_length,
        })
    }

    fn rewrite(&self) -> Option<AbleSetRow> {
        let mut changed = false;
        let mut next = AbleSetRow {
            id: self.id.clone(),
            enabled: self.enabled,
            host: self.host.clone(),
            osc_port: self.osc_port,
            http_port: self.http_port,
            library_name: self.library_name.clone(),
            song_prefix_length: self.song_prefix_length,
        };
        if next.http_port == 5950 {
            next.http_port = NEW_HTTP_PORT;
            changed = true;
        }
        if next.osc_port == 5950 {
            next.osc_port = NEW_OSC_PORT;
            changed = true;
        }
        if next.library_name.eq_ignore_ascii_case("NEWLEVEL") {
            next.library_name = NEW_LIBRARY_NAME.to_string();
            changed = true;
        }
        if changed {
            Some(next)
        } else {
            None
        }
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let backend = conn.get_database_backend();

        // The `ableset_settings` table is created lazily by the repository on
        // first access (see `Repository::ensure_ableset_settings_table`). On a
        // fresh DB the table will not exist yet — no legacy data to fix.
        let table_check = conn
            .query_one(sea_orm::Statement::from_string(
                backend,
                "SELECT name FROM sqlite_master WHERE type='table' AND name='ableset_settings'"
                    .to_string(),
            ))
            .await?;
        if table_check.is_none() {
            return Ok(());
        }

        // Select rows that still match ANY of the legacy values. Other rows
        // already have the new defaults and require no rewrite (idempotent).
        let candidates = conn
            .query_all(sea_orm::Statement::from_string(
                backend,
                "SELECT id, enabled, host, osc_port, http_port, library_name, song_prefix_length \
                 FROM ableset_settings \
                 WHERE http_port = 5950 OR osc_port = 5950 OR UPPER(library_name) = 'NEWLEVEL'"
                    .to_string(),
            ))
            .await?;

        for row in candidates {
            let before = AbleSetRow {
                id: row.try_get::<String>("", "id")?,
                enabled: row.try_get::<bool>("", "enabled")?,
                host: row.try_get::<String>("", "host")?,
                osc_port: row.try_get::<i32>("", "osc_port")?,
                http_port: row.try_get::<i32>("", "http_port")?,
                library_name: row.try_get::<String>("", "library_name")?,
                song_prefix_length: row.try_get::<i32>("", "song_prefix_length")?,
            };

            let Some(after) = before.rewrite() else {
                // Row matched on a column we don't touch in `rewrite()` —
                // skip without UPDATE so we never write a no-op audit row.
                continue;
            };

            // Update only the columns this migration owns. `updated_at` is
            // bumped so consumers see the row was touched.
            conn.execute(sea_orm::Statement::from_sql_and_values(
                backend,
                "UPDATE ableset_settings \
                 SET http_port = ?1, osc_port = ?2, library_name = ?3, updated_at = ?4 \
                 WHERE id = ?5",
                [
                    after.http_port.into(),
                    after.osc_port.into(),
                    after.library_name.clone().into(),
                    chrono::Utc::now().to_rfc3339().into(),
                    after.id.clone().into(),
                ],
            ))
            .await?;

            // Insert one audit row recording the rewrite, source = migration.
            let audit_id = uuid::Uuid::new_v4().to_string();
            let before_json = before.to_json().to_string();
            let after_json = after.to_json().to_string();
            conn.execute(sea_orm::Statement::from_sql_and_values(
                backend,
                "INSERT INTO settings_audit \
                 (id, setting_table, setting_id, source, actor, before_json, after_json, changed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                [
                    audit_id.into(),
                    AUDIT_TABLE.into(),
                    after.id.into(),
                    AUDIT_SOURCE.into(),
                    AUDIT_ACTOR.into(),
                    before_json.into(),
                    after_json.into(),
                    chrono::Utc::now().to_rfc3339().into(),
                ],
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // No down — legacy values cannot be safely restored.
        Ok(())
    }
}
