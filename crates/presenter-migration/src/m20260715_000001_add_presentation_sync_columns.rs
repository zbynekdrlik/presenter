//! #555 song sync: adds `updated_at` (LWW clock, backfilled from `created_at`),
//! `sync_id` (cross-instance identity, deterministic UUIDv5 backfill), and
//! `deleted_at` (soft-delete trash marker) to `presentations`.
use presenter_core::sync_id_for_name;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;
use std::collections::HashMap;

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
        //    #558 S8: the ADD COLUMN is gated on column_missing (idempotent —
        //    SQLite errors re-adding an existing column), but the backfill UPDATE
        //    is NOT — it always runs, keyed on DATA state (`updated_at IS NULL`),
        //    so a crash between ADD COLUMN and the backfill can never strand NULL
        //    rows forever once column_missing() starts returning false.
        if column_missing(db, "updated_at").await? {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE presentations ADD COLUMN updated_at TEXT",
            ))
            .await?;
        }
        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE presentations SET updated_at = created_at WHERE updated_at IS NULL",
        ))
        .await?;

        // 2. deleted_at — genuinely nullable (the trash marker), no backfill needed.
        if column_missing(db, "deleted_at").await? {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE presentations ADD COLUMN deleted_at TEXT",
            ))
            .await?;
        }

        // 3. sync_id — cross-instance identity. Same S8 split as updated_at: the
        //    ADD COLUMN is column-gated, the backfill below always runs (data-gated
        //    via `sync_id IS NULL` in its UPDATE). Ranked deterministically by
        //    (library name, presentation name, created_at, id) — #558 S5: NOT by
        //    unordered scan order, which tracks physical/insertion order and gives
        //    two independently built databases no reason to agree on which "twin"
        //    ranks first. A same-library+same-name collision derives
        //    UUIDv5(lib/name/k) — a deterministic occurrence counter, never
        //    Uuid::new_v4() (which can never converge between two sites).
        //
        //    #558 R6 (accepted residual risk, documented — no redesign): the
        //    `created_at` tie-break ranks a collision group using EACH SITE'S
        //    OWN import history, which has no cross-site correlation — two
        //    independently built databases holding what SHOULD be "the same"
        //    pair of same-library+same-name twins can rank them in a
        //    DIFFERENT order (whichever twin has the earlier `created_at` on
        //    THAT site keeps the plain name-derived id; the other derives
        //    `.../k`), so the two twins can pair up CROSSWISE between sites
        //    instead of matching content-to-content. This is a legacy-data
        //    edge case only (a fresh import always assigns via #558 S3's
        //    content-pure, occurrence-count rule instead): same-library +
        //    same-name twins are rare, and when they occur their content is
        //    usually identical anyway, so a crosswise pairing is low-impact.
        //    Every row backfilled as a NON-FIRST occurrence of a collision
        //    group (`k > 1`) is logged below so operators can spot-check
        //    those specific songs after the first sync.
        if column_missing(db, "sync_id").await? {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE presentations ADD COLUMN sync_id TEXT",
            ))
            .await?;
        }
        let rows = db
            .query_all(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT p.id AS id, COALESCE(l.name, '') AS lib_name, p.name AS name \
                 FROM presentations p LEFT JOIN libraries l ON l.id = p.library_id \
                 ORDER BY lib_name, name, p.created_at, p.id",
            ))
            .await?;
        let mut occurrence: HashMap<(String, String), u32> = HashMap::new();
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let lib_name: String = row.try_get("", "lib_name")?;
            let name: String = row.try_get("", "name")?;
            let k = occurrence
                .entry((lib_name.clone(), name.clone()))
                .or_insert(0);
            *k += 1;
            let sid = if *k == 1 {
                sync_id_for_name(&lib_name, &name)
            } else {
                // #558 R6: a same-library+same-name collision group — logged
                // so operators can spot-check this song after the first
                // sync (the created_at tie-break has no cross-site
                // correlation; see the comment above).
                tracing::warn!(
                    presentation_id = %id,
                    library_name = %lib_name,
                    presentation_name = %name,
                    occurrence = *k,
                    "sync_id backfill: same-library+same-name collision group — \
                     verify this song pairs correctly with its peer after first sync"
                );
                uuid::Uuid::new_v5(
                    &presenter_core::SYNC_ID_NAMESPACE,
                    format!("{lib_name}/{name}/{k}").as_bytes(),
                )
                .to_string()
            };
            db.execute(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "UPDATE presentations SET sync_id = ? WHERE id = ? AND sync_id IS NULL",
                [sid.into(), id.into()],
            ))
            .await?;
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

        // The two colliding ("Twin") rows must get DISTINCT sync_ids. #558 S5:
        // rank by (library name, presentation name, created_at, id) — p1 was
        // created earlier (2026-01-02) than p2 (2026-01-03), so p1 keeps the
        // deterministic name-based id and p2 derives via UUIDv5 with a
        // deterministic occurrence counter (never Uuid::new_v4()).
        let twin_a = rows.iter().find(|r| r.0 == "p1").expect("p1");
        let twin_b = rows.iter().find(|r| r.0 == "p2").expect("p2");
        assert_ne!(twin_a.3, twin_b.3, "collision fallback keeps ids unique");
        assert_eq!(
            twin_a.3,
            sync_id_for_name("Songs", "Twin"),
            "the earlier-created twin keeps the deterministic name-based id"
        );
        assert_eq!(
            twin_b.3,
            uuid::Uuid::new_v5(
                &presenter_core::SYNC_ID_NAMESPACE,
                "Songs/Twin/2".as_bytes(),
            )
            .to_string(),
            "the later twin derives via UUIDv5 with a deterministic occurrence counter"
        );

        // Re-running the migration must be a no-op (idempotent).
        let before = all_rows(&db).await;
        Migration.up(&manager).await.expect("second up");
        assert_eq!(before, all_rows(&db).await, "second run changed nothing");
    }

    #[tokio::test]
    async fn backfill_collision_assignment_is_independent_of_physical_row_order() {
        // S5 regression: the old backfill SELECT had no ORDER BY, so on
        // SQLite the scan order tracked physical/insertion (rowid) order --
        // which two INDEPENDENTLY built databases (each holding what should
        // be "the same" duplicate-named songs) have no reason to agree on.
        // Combined with a RANDOM Uuid::new_v4() collision fallback, the two
        // sites could assign the deterministic name-based id to DIFFERENT
        // physical rows ("the twins pair up crosswise between sites"), and
        // the random fallback itself never converges at all. Fixed rule:
        // rank by (library name, presentation name, created_at, id) -- a
        // stable key that does NOT depend on physical row/insertion order --
        // and the collision fallback is UUIDv5(lib/name/k), never random.
        async fn build(insert_order: [&str; 2]) -> DatabaseConnection {
            let db = Database::connect("sqlite::memory:").await.expect("connect");
            for sql in [
                "CREATE TABLE libraries (id TEXT PRIMARY KEY, name TEXT NOT NULL, \
                 search_name TEXT NOT NULL, created_at TEXT NOT NULL)",
                "CREATE TABLE presentations (id TEXT PRIMARY KEY, library_id TEXT NOT NULL, \
                 name TEXT NOT NULL, search_name TEXT NOT NULL, created_at TEXT NOT NULL)",
                "INSERT INTO libraries VALUES \
                 ('lib1', 'Songs', 'songs', '2026-01-01T00:00:00+00:00')",
            ] {
                db.execute(Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    sql,
                ))
                .await
                .expect("seed");
            }
            let created_at: std::collections::HashMap<&str, &str> = [
                ("p1", "2026-01-02T00:00:00+00:00"),
                ("p2", "2026-01-03T00:00:00+00:00"),
            ]
            .into();
            for id in insert_order {
                let sql = format!(
                    "INSERT INTO presentations VALUES \
                     ('{id}', 'lib1', 'Twin', 'twin', '{}')",
                    created_at[id]
                );
                db.execute(Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    sql,
                ))
                .await
                .expect("seed presentation");
            }
            db
        }

        let forward = build(["p1", "p2"]).await;
        let reversed = build(["p2", "p1"]).await;

        Migration
            .up(&SchemaManager::new(&forward))
            .await
            .expect("forward up");
        Migration
            .up(&SchemaManager::new(&reversed))
            .await
            .expect("reversed up");

        let forward_rows = all_rows(&forward).await;
        let reversed_rows = all_rows(&reversed).await;
        let sid = |rows: &[(String, String, Option<String>, String)], id: &str| {
            rows.iter().find(|r| r.0 == id).unwrap().3.clone()
        };

        assert_eq!(
            sid(&forward_rows, "p1"),
            sid(&reversed_rows, "p1"),
            "p1 (earlier created_at) must get the same sync_id regardless of physical insert order"
        );
        assert_eq!(
            sid(&forward_rows, "p2"),
            sid(&reversed_rows, "p2"),
            "p2 (later created_at) must get the same sync_id regardless of physical insert order"
        );
        assert_eq!(
            sid(&forward_rows, "p1"),
            sync_id_for_name("Songs", "Twin"),
            "the earlier-created twin keeps the deterministic name-based id"
        );
        assert_eq!(
            sid(&forward_rows, "p2"),
            uuid::Uuid::new_v5(
                &presenter_core::SYNC_ID_NAMESPACE,
                "Songs/Twin/2".as_bytes(),
            )
            .to_string(),
            "the later twin derives via UUIDv5 with a deterministic occurrence counter, never random"
        );
    }

    #[tokio::test]
    async fn interrupted_backfill_is_completed_by_a_rerun_after_columns_already_exist() {
        // S8 regression: the OLD code gated the WHOLE backfill (not just the
        // ADD COLUMN) behind `column_missing()`. A crash between ADD COLUMN
        // and the backfill UPDATE/loop leaves NULL rows -- and since the
        // column now EXISTS, a restart's `column_missing()` check is false
        // and the backfill is skipped FOREVER (every SELECT then fails to
        // decode a NOT NULL `updated_at` / non-empty `sync_id` -> the whole
        // app 500s). Simulate the crash: run the full migration once, then
        // manually null out one row's updated_at/sync_id (as if its backfill
        // never completed before the crash) and re-run the migration -- a
        // rerun MUST finish the interrupted backfill, keyed on DATA state,
        // not on whether the column already exists.
        let db = setup_pre_migration_db().await;
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("first up");

        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE presentations SET updated_at = NULL, sync_id = NULL WHERE id = 'p3'",
        ))
        .await
        .expect("simulate a crash that stranded p3 mid-backfill");

        Migration.up(&manager).await.expect("second up (resumed)");

        let rows = all_rows(&db).await;
        let p3 = rows.iter().find(|r| r.0 == "p3").expect("p3");
        assert!(
            p3.2.is_some(),
            "a rerun must finish the interrupted updated_at backfill, not skip it \
             because the column already exists"
        );
        assert!(
            !p3.3.is_empty(),
            "a rerun must finish the interrupted sync_id backfill too"
        );
        assert_eq!(p3.3, sync_id_for_name("Songs", "Solo"));
    }
}
