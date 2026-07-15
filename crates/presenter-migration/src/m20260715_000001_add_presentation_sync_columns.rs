//! #555 song sync: adds `updated_at` (LWW clock, backfilled from `created_at`),
//! `sync_id` (cross-instance identity, deterministic UUIDv5 backfill), and
//! `deleted_at` (soft-delete trash marker) to `presentations`.
use presenter_core::sync_id_for_name;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;
use std::collections::HashSet;

#[derive(DeriveMigrationName)]
pub struct Migration;

async fn column_missing<C: ConnectionTrait>(db: &C, column: &str) -> Result<bool, DbErr> {
    let sql = format!(
        "SELECT COUNT(*) AS cnt FROM pragma_table_info('presentations') WHERE name='{column}'"
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

        // 1. updated_at — nullable at DB level (SQLite can't ADD NOT NULL without a
        //    default), backfilled from created_at, always set by every insert going
        //    forward, so no NULL row ever exists (entity type is NOT NULL).
        if column_missing(db, "updated_at").await? {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE presentations ADD COLUMN updated_at TEXT",
            ))
            .await?;
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "UPDATE presentations SET updated_at = created_at WHERE updated_at IS NULL",
            ))
            .await?;
        }

        // 2. deleted_at — genuinely nullable (the trash marker).
        if column_missing(db, "deleted_at").await? {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE presentations ADD COLUMN deleted_at TEXT",
            ))
            .await?;
        }

        // 3. sync_id — cross-instance identity. Backfill deterministically; on the
        //    rare same-library+same-name in-DB collision, fall back to a fresh v4 so
        //    the unique index below always holds.
        if column_missing(db, "sync_id").await? {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE presentations ADD COLUMN sync_id TEXT",
            ))
            .await?;

            let rows = db
                .query_all(Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    "SELECT p.id AS id, COALESCE(l.name, '') AS lib_name, p.name AS name \
                     FROM presentations p LEFT JOIN libraries l ON l.id = p.library_id",
                ))
                .await?;
            let mut used: HashSet<String> = HashSet::new();
            for row in rows {
                let id: String = row.try_get("", "id")?;
                let lib_name: String = row.try_get("", "lib_name")?;
                let name: String = row.try_get("", "name")?;
                let mut sid = sync_id_for_name(&lib_name, &name);
                if !used.insert(sid.clone()) {
                    sid = uuid::Uuid::new_v4().to_string();
                    used.insert(sid.clone());
                }
                db.execute(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Sqlite,
                    "UPDATE presentations SET sync_id = ? WHERE id = ? AND sync_id IS NULL",
                    [sid.into(), id.into()],
                ))
                .await?;
            }
        }

        // Unique index — enforces one row per identity. Idempotent (IF NOT EXISTS).
        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_presentations_sync_id \
             ON presentations(sync_id)",
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Non-destructive additive migration; SQLite pre-3.35 has no DROP COLUMN. No-op.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DatabaseConnection};
    use sea_orm_migration::SchemaManager;

    async fn setup_pre_migration_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        for sql in [
            "CREATE TABLE libraries (id TEXT PRIMARY KEY, name TEXT NOT NULL, \
             search_name TEXT NOT NULL, created_at TEXT NOT NULL)",
            "CREATE TABLE presentations (id TEXT PRIMARY KEY, library_id TEXT NOT NULL, \
             name TEXT NOT NULL, search_name TEXT NOT NULL, created_at TEXT NOT NULL)",
            "INSERT INTO libraries VALUES ('lib1', 'Songs', 'songs', '2026-01-01T00:00:00+00:00')",
            "INSERT INTO presentations VALUES \
             ('p1', 'lib1', 'Twin', 'twin', '2026-01-02T00:00:00+00:00'), \
             ('p2', 'lib1', 'Twin', 'twin', '2026-01-03T00:00:00+00:00'), \
             ('p3', 'lib1', 'Solo', 'solo', '2026-01-04T00:00:00+00:00')",
        ] {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                sql,
            ))
            .await
            .expect("seed");
        }
        db
    }

    async fn all_rows(db: &DatabaseConnection) -> Vec<(String, String, Option<String>, String)> {
        let rows = db
            .query_all(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT id, created_at, updated_at, sync_id FROM presentations ORDER BY id",
            ))
            .await
            .expect("select");
        rows.into_iter()
            .map(|r| {
                (
                    r.try_get::<String>("", "id").expect("id"),
                    r.try_get::<String>("", "created_at").expect("created_at"),
                    r.try_get::<Option<String>>("", "updated_at")
                        .expect("updated_at"),
                    r.try_get::<String>("", "sync_id").expect("sync_id"),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn backfills_updated_at_and_deterministic_sync_id_idempotently() {
        let db = setup_pre_migration_db().await;
        let manager = SchemaManager::new(&db);

        Migration.up(&manager).await.expect("first up");

        let rows = all_rows(&db).await;
        assert_eq!(rows.len(), 3);
        for (id, created, updated, sync_id) in &rows {
            assert_eq!(
                updated.as_deref(),
                Some(created.as_str()),
                "{id}: updated_at backfilled from created_at"
            );
            assert!(!sync_id.is_empty(), "{id}: sync_id assigned");
        }

        // Determinism for the non-colliding row.
        let solo = rows.iter().find(|r| r.0 == "p3").expect("p3");
        assert_eq!(solo.3, sync_id_for_name("Songs", "Solo"));

        // The two colliding rows must still get DISTINCT sync_ids (v4 fallback).
        let twin_a = rows.iter().find(|r| r.0 == "p1").expect("p1");
        let twin_b = rows.iter().find(|r| r.0 == "p2").expect("p2");
        assert_ne!(twin_a.3, twin_b.3, "collision fallback keeps ids unique");

        // Re-running the migration must be a no-op (idempotent).
        let before = all_rows(&db).await;
        Migration.up(&manager).await.expect("second up");
        assert_eq!(before, all_rows(&db).await, "second run changed nothing");
    }
}
