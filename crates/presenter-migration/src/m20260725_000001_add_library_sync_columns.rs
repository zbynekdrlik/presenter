//! #578 library sync: adds `updated_at` (LWW clock), `sync_id` (cross-instance
//! identity, random-UUID backfill — peers converge via the name-match adopt on
//! first sync), and `deleted_at` (soft-delete tombstone) to `libraries`,
//! mirroring the #555 presentation sync columns. Without these a
//! library-cascade delete hard-removed presentations with no tombstone, so the
//! peer's still-live copy resurrected the whole library+song on the next sync
//! cycle (a permanent loop). It also replaces the plain `UNIQUE(name)` index
//! with a PARTIAL unique over LIVE rows only, so a tombstoned library can keep
//! its name without blocking a same-named live library.
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

async fn column_missing<C: ConnectionTrait>(db: &C, column: &str) -> Result<bool, DbErr> {
    let sql =
        format!("SELECT COUNT(*) AS cnt FROM pragma_table_info('libraries') WHERE name='{column}'");
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

        // 1. updated_at — nullable at DB level (SQLite can't ADD NOT NULL
        //    without a default), backfilled to `now`, and always set by every
        //    insert/mutation going forward so no NULL row ever exists (the
        //    entity type is NOT NULL). Same S8 split as the presentation
        //    migration: the ADD COLUMN is column-gated (idempotent), but the
        //    backfill UPDATE always runs, keyed on DATA state
        //    (`updated_at IS NULL`), so a crash between ADD COLUMN and the
        //    backfill can never strand NULL rows once the column exists.
        if column_missing(db, "updated_at").await? {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE libraries ADD COLUMN updated_at TEXT",
            ))
            .await?;
        }
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE libraries SET updated_at = ? WHERE updated_at IS NULL",
            [now.into()],
        ))
        .await?;

        // 2. deleted_at — the soft-delete tombstone marker, genuinely
        //    nullable, no backfill.
        if column_missing(db, "deleted_at").await? {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE libraries ADD COLUMN deleted_at TEXT",
            ))
            .await?;
        }

        // 3. sync_id — cross-instance identity. Backfilled with a fresh random
        //    UUID per library (per the settled design); two independently
        //    migrated peers get DIFFERENT ids for the same-named library and
        //    converge on first sync via `apply_sync_library`'s name-match
        //    adopt. Same S8 data-gated backfill (`sync_id IS NULL`) so a
        //    rerun after a crash finishes any stranded rows.
        if column_missing(db, "sync_id").await? {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE libraries ADD COLUMN sync_id TEXT",
            ))
            .await?;
        }
        let rows = db
            .query_all(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT id FROM libraries WHERE sync_id IS NULL",
            ))
            .await?;
        for row in rows {
            let id: String = row.try_get("", "id")?;
            db.execute(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "UPDATE libraries SET sync_id = ? WHERE id = ? AND sync_id IS NULL",
                [uuid::Uuid::new_v4().to_string().into(), id.into()],
            ))
            .await?;
        }

        // Unique index on sync_id — one row per identity (mirrors
        // idx_presentations_sync_id). Idempotent.
        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_libraries_sync_id ON libraries(sync_id)",
        ))
        .await?;

        // Replace the plain UNIQUE(name) index with a PARTIAL unique over LIVE
        // (deleted_at IS NULL) rows only. A soft-deleted library keeps its
        // name occupied; a plain UNIQUE would then block re-creating or
        // adopting a same-named live library. The partial index preserves the
        // "one LIVE library per name" invariant that `upsert_library` /
        // `create_library` rely on while allowing any number of tombstones.
        // Both statements are idempotent (IF EXISTS / IF NOT EXISTS).
        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "DROP INDEX IF EXISTS idx_libraries_name_unique",
        ))
        .await?;
        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_libraries_name_live_unique \
             ON libraries(name) WHERE deleted_at IS NULL",
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
            "CREATE UNIQUE INDEX idx_libraries_name_unique ON libraries(name)",
            "INSERT INTO libraries VALUES \
             ('lib1', 'Songs', 'songs', '2026-01-01T00:00:00+00:00'), \
             ('lib2', 'Hymns', 'hymns', '2026-01-02T00:00:00+00:00')",
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

    async fn rows(db: &DatabaseConnection) -> Vec<(String, Option<String>, Option<String>)> {
        let rows = db
            .query_all(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT id, updated_at, sync_id FROM libraries ORDER BY id",
            ))
            .await
            .expect("select");
        rows.into_iter()
            .map(|r| {
                (
                    r.try_get::<String>("", "id").expect("id"),
                    r.try_get::<Option<String>>("", "updated_at")
                        .expect("updated_at"),
                    r.try_get::<Option<String>>("", "sync_id").expect("sync_id"),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn backfills_columns_and_is_idempotent() {
        let db = setup_pre_migration_db().await;
        let manager = SchemaManager::new(&db);

        Migration.up(&manager).await.expect("first up");

        let after = rows(&db).await;
        assert_eq!(after.len(), 2);
        let mut sync_ids = std::collections::HashSet::new();
        for (id, updated, sync_id) in &after {
            assert!(updated.is_some(), "{id}: updated_at backfilled");
            let sid = sync_id.as_ref().expect("sync_id backfilled");
            assert!(!sid.is_empty(), "{id}: sync_id non-empty");
            assert!(sync_ids.insert(sid.clone()), "{id}: sync_id unique");
        }

        // Re-running must change nothing (idempotent ADD COLUMN + data-gated backfill).
        let before = rows(&db).await;
        Migration.up(&manager).await.expect("second up");
        assert_eq!(before, rows(&db).await, "second run changed nothing");
    }

    #[tokio::test]
    async fn interrupted_backfill_is_completed_by_a_rerun() {
        // S8 regression: the backfill must be keyed on DATA state, not on
        // whether the column already exists — a crash between ADD COLUMN and
        // the backfill must be finished by a rerun.
        let db = setup_pre_migration_db().await;
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("first up");

        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE libraries SET updated_at = NULL, sync_id = NULL WHERE id = 'lib2'",
        ))
        .await
        .expect("simulate a crash that stranded lib2 mid-backfill");

        Migration.up(&manager).await.expect("second up (resumed)");

        let after = rows(&db).await;
        let lib2 = after.iter().find(|r| r.0 == "lib2").expect("lib2");
        assert!(lib2.1.is_some(), "a rerun finishes the updated_at backfill");
        assert!(
            lib2.2.as_ref().is_some_and(|s| !s.is_empty()),
            "a rerun finishes the sync_id backfill"
        );
    }

    #[tokio::test]
    async fn partial_name_unique_allows_a_tombstone_beside_a_live_row() {
        let db = setup_pre_migration_db().await;
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("up");

        // Tombstone lib1 ("Songs"), then a fresh LIVE "Songs" must be allowed
        // (the OLD plain UNIQUE(name) would reject this).
        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE libraries SET deleted_at = '2026-02-01T00:00:00+00:00' WHERE id = 'lib1'",
        ))
        .await
        .expect("tombstone lib1");
        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "INSERT INTO libraries (id, name, search_name, created_at, updated_at, sync_id) VALUES \
             ('lib3', 'Songs', 'songs', '2026-02-02T00:00:00+00:00', \
              '2026-02-02T00:00:00+00:00', 'sid-3')",
        ))
        .await
        .expect("a fresh live 'Songs' beside the tombstone must be allowed");

        // But two LIVE rows with the same name must still be rejected.
        let dup = db
            .execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "INSERT INTO libraries (id, name, search_name, created_at, updated_at, sync_id) \
                 VALUES ('lib4', 'Songs', 'songs', '2026-02-03T00:00:00+00:00', \
                 '2026-02-03T00:00:00+00:00', 'sid-4')",
            ))
            .await;
        assert!(
            dup.is_err(),
            "a second LIVE 'Songs' must still violate the partial unique index"
        );
    }
}
