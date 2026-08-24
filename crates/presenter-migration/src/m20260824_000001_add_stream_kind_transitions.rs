//! #752: LAYER-level (kind-level) scene-transition durations for stream
//! outputs. Adds two nullable columns to `stream_outputs`:
//!
//! - `base_transition_ms`    — crossfade for ALL base scene switches (NULL =
//!   inherit `default_transition_ms`; 0 = cut, per #716).
//! - `overlay_transition_ms` — fade for ALL overlay on/off toggles (same NULL /
//!   0 semantics).
//!
//! They sit BETWEEN the per-scene `stream_scenes.transition_ms` override and the
//! per-output `default_transition_ms` fallback in the output-page resolution
//! order: `scene.transition_ms ?? kind-level ?? default_transition_ms`.
//!
//! Additive + idempotent (`column_missing` guard) per the repo's DB policy —
//! `stream_outputs` already holds prod data (configRevision ~76), so this is an
//! incremental `ALTER TABLE ADD COLUMN`, never an edit of the applied
//! `m20260820_000001_create_stream_tables` migration. Mirrors the two-port
//! `m20260717_000001_add_resolume_active_port.rs` idiom exactly.
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

async fn column_missing<C: ConnectionTrait>(db: &C, column: &str) -> Result<bool, DbErr> {
    let sql = format!(
        "SELECT COUNT(*) AS cnt FROM pragma_table_info('stream_outputs') WHERE name='{column}'"
    );
    let row = db
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await?;
    Ok(row
        .map(|r| r.try_get::<i32>("", "cnt").unwrap_or(0) == 0)
        .unwrap_or(true))
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for column in ["base_transition_ms", "overlay_transition_ms"] {
            if column_missing(db, column).await? {
                db.execute(Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    format!("ALTER TABLE stream_outputs ADD COLUMN {column} INTEGER"),
                ))
                .await?;
            }
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Non-destructive additive migration; SQLite pre-3.35 has no DROP COLUMN
        // and dropping would discard legitimately-configured kind transitions.
        // No-op.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DbBackend};

    /// A `stream_outputs` table shaped like the create-tables migration, but
    /// WITHOUT the two new kind-transition columns (the pre-migration state).
    async fn setup_pre_migration_db() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE stream_outputs (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, slug TEXT NOT NULL, name TEXT NOT NULL, \
                default_transition_ms INTEGER NOT NULL DEFAULT 400, active_scene_id INTEGER, \
                config_revision INTEGER NOT NULL DEFAULT 0, \
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL)"
                .to_string(),
        ))
        .await
        .expect("seed table predating the kind-transition columns");
        db
    }

    /// RED before the migration (neither column exists), GREEN after.
    #[tokio::test]
    async fn adds_both_kind_transition_columns() {
        let db = setup_pre_migration_db().await;

        for column in ["base_transition_ms", "overlay_transition_ms"] {
            let before = db
                .execute(Statement::from_string(
                    DbBackend::Sqlite,
                    format!("SELECT {column} FROM stream_outputs"),
                ))
                .await;
            assert!(
                before.is_err(),
                "precondition: {column} must not exist before the migration"
            );
        }

        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("migration up");

        let after = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT base_transition_ms, overlay_transition_ms FROM stream_outputs".to_string(),
            ))
            .await;
        assert!(
            after.is_ok(),
            "both kind-transition columns queryable after up: {after:?}"
        );
    }

    /// Re-running on a DB that already has the columns is a no-op and never
    /// touches existing rows (idempotent — SQLite errors re-adding a column).
    #[tokio::test]
    async fn is_idempotent_and_preserves_existing_rows() {
        let db = setup_pre_migration_db().await;
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("first up");

        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO stream_outputs \
             (slug, name, default_transition_ms, base_transition_ms, overlay_transition_ms, \
              config_revision, created_at, updated_at) \
             VALUES ('stream', 'Stream', 400, 0, 800, 3, \
             '2026-08-24T00:00:00+00:00', '2026-08-24T00:00:00+00:00')"
                .to_string(),
        ))
        .await
        .expect("insert");

        Migration
            .up(&manager)
            .await
            .expect("second up must be a no-op");

        let row = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT base_transition_ms, overlay_transition_ms FROM stream_outputs \
                 WHERE slug = 'stream'"
                    .to_string(),
            ))
            .await
            .expect("query")
            .expect("row");
        let base: i32 = row.try_get_by("base_transition_ms").expect("base");
        let overlay: i32 = row.try_get_by("overlay_transition_ms").expect("overlay");
        assert_eq!(base, 0, "re-run preserves base (0 = cut)");
        assert_eq!(overlay, 800, "re-run preserves overlay");
    }
}
