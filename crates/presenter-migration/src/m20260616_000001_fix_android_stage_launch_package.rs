use sea_orm_migration::prelude::*;

/// The dead Fully Kiosk component seeded by `m20260414_000002`. Its activity
/// class does not exist on the prod TVs, so the old `am start -n <component>`
/// launch fails on every attempt (issue #404).
const DEAD_LAUNCH_COMPONENT: &str = "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity";

/// New launch package: the TCL built-in browser. The launcher now fires a
/// VIEW intent at this PACKAGE with the configured `PRESENTER_ANDROID_STAGE_URL`.
const NEW_LAUNCH_PACKAGE: &str = "com.tcl.browser";

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Rewrite only rows that still carry the dead Fully Kiosk component to the
/// `com.tcl.browser` package. Operator-customised rows (any other value) are
/// left untouched. Running twice is a no-op (the WHERE clause no longer
/// matches after the first run), so this is safe across redeploys.
///
/// Returns the number of rows updated (for test assertions).
pub(crate) async fn fix_dead_launch_components(
    conn: &impl sea_orm::ConnectionTrait,
) -> Result<u64, DbErr> {
    let backend = conn.get_database_backend();

    // The table is part of the initial schema, but guard defensively: a fresh
    // DB that has not yet created it (or a future site that dropped it) must
    // not fail the migration.
    let table_check = conn
        .query_one(sea_orm::Statement::from_string(
            backend,
            "SELECT name FROM sqlite_master WHERE type='table' AND name='android_stage_displays'"
                .to_string(),
        ))
        .await?;
    if table_check.is_none() {
        return Ok(0);
    }

    let result = conn
        .execute(sea_orm::Statement::from_sql_and_values(
            backend,
            "UPDATE android_stage_displays \
             SET launch_component = ?1, updated_at = ?2 \
             WHERE launch_component = ?3",
            [
                NEW_LAUNCH_PACKAGE.into(),
                chrono::Utc::now().to_rfc3339().into(),
                DEAD_LAUNCH_COMPONENT.into(),
            ],
        ))
        .await?;

    Ok(result.rows_affected())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        fix_dead_launch_components(manager.get_connection()).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // No-op. The dead component must never be restored.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DbBackend, Statement};

    async fn setup_db() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE android_stage_displays ( \
                 id TEXT PRIMARY KEY, label TEXT NOT NULL, host TEXT NOT NULL, \
                 port INTEGER NOT NULL, launch_component TEXT NOT NULL, \
                 is_enabled INTEGER NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)"
                .to_string(),
        ))
        .await
        .expect("ddl");
        db
    }

    async fn insert_row(db: &sea_orm::DatabaseConnection, id: &str, launch_component: &str) {
        db.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO android_stage_displays \
             (id, label, host, port, launch_component, is_enabled, created_at, updated_at) \
             VALUES (?1, 'Stage', 'sd1l.lan', 5555, ?2, 1, '2026-06-16T00:00:00Z', '2026-06-16T00:00:00Z')",
            [id.into(), launch_component.into()],
        ))
        .await
        .expect("insert");
    }

    async fn fetch_component(db: &sea_orm::DatabaseConnection, id: &str) -> String {
        let row = db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT launch_component FROM android_stage_displays WHERE id = ?1",
                [id.into()],
            ))
            .await
            .expect("query")
            .expect("row");
        row.try_get_by_index::<String>(0).expect("string col")
    }

    #[tokio::test]
    async fn migration_replaces_dead_component_and_is_idempotent() {
        let db = setup_db().await;

        // dead-value row (must be fixed), an operator-customised row (left alone),
        // and a row already on the new package (left alone).
        insert_row(
            &db,
            "dead",
            "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity",
        )
        .await;
        insert_row(&db, "custom", "org.mozilla.firefox").await;
        insert_row(&db, "already", "com.tcl.browser").await;

        let updated = fix_dead_launch_components(&db).await.expect("up");
        assert_eq!(updated, 1, "only the dead-value row should be rewritten");

        assert_eq!(fetch_component(&db, "dead").await, "com.tcl.browser");
        assert_eq!(fetch_component(&db, "custom").await, "org.mozilla.firefox");
        assert_eq!(fetch_component(&db, "already").await, "com.tcl.browser");

        // Idempotent: a second run touches nothing.
        let updated_again = fix_dead_launch_components(&db).await.expect("rerun");
        assert_eq!(updated_again, 0, "rerun must be a no-op");
        assert_eq!(fetch_component(&db, "dead").await, "com.tcl.browser");
    }

    #[tokio::test]
    async fn migration_no_op_when_table_missing() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let updated = fix_dead_launch_components(&db)
            .await
            .expect("missing table must not error");
        assert_eq!(updated, 0);
    }
}
