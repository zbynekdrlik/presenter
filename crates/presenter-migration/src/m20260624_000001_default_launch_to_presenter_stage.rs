use sea_orm_migration::prelude::*;

/// Previous default launch package: the TCL built-in browser (set by
/// `m20260616_000001`). It only exists on TCL TVs, so it fails on other brands
/// (e.g. Sharp/MediaTek, where it is absent) — see #472.
const OLD_LAUNCH_PACKAGE: &str = "com.tcl.browser";

/// New default launch package: our own Presenter Stage app. The watchdog
/// auto-installs this APK via ADB on any TV that lacks it, so the stage runs on
/// EVERY Android TV without a kiosk browser or a per-brand browser dependency.
const NEW_LAUNCH_PACKAGE: &str = "sk.newlevel.presenterstage";

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Rewrite only rows that still carry the previous default (`com.tcl.browser`)
/// to our own app package. Operator-customised rows (any other value, e.g. a
/// deliberately chosen browser) are left untouched. Running twice is a no-op
/// (the WHERE clause no longer matches after the first run), so this is safe
/// across redeploys.
///
/// Returns the number of rows updated (for test assertions).
pub(crate) async fn migrate_default_launch_package(
    conn: &impl sea_orm::ConnectionTrait,
) -> Result<u64, DbErr> {
    let backend = conn.get_database_backend();

    // The table is part of the initial schema, but guard defensively: a fresh DB
    // that has not yet created it (or a future site that dropped it) must not
    // fail the migration.
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
                OLD_LAUNCH_PACKAGE.into(),
            ],
        ))
        .await?;

    Ok(result.rows_affected())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        migrate_default_launch_package(manager.get_connection()).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // No-op. Reverting to the TCL-only package would re-break non-TCL TVs.
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
             VALUES (?1, 'Stage', 'sd1l.lan', 5555, ?2, 1, '2026-06-24T00:00:00Z', '2026-06-24T00:00:00Z')",
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
    async fn migration_replaces_tcl_default_and_is_idempotent() {
        let db = setup_db().await;

        // old-default row (must be migrated), an operator-customised row (left
        // alone), and a row already on our app (left alone).
        insert_row(&db, "tcl", "com.tcl.browser").await;
        insert_row(&db, "custom", "org.mozilla.firefox").await;
        insert_row(&db, "ours", "sk.newlevel.presenterstage").await;

        let updated = migrate_default_launch_package(&db).await.expect("up");
        assert_eq!(
            updated, 1,
            "only the com.tcl.browser row should be rewritten"
        );

        assert_eq!(
            fetch_component(&db, "tcl").await,
            "sk.newlevel.presenterstage"
        );
        assert_eq!(fetch_component(&db, "custom").await, "org.mozilla.firefox");
        assert_eq!(
            fetch_component(&db, "ours").await,
            "sk.newlevel.presenterstage"
        );

        // Idempotent: a second run touches nothing.
        let updated_again = migrate_default_launch_package(&db).await.expect("rerun");
        assert_eq!(updated_again, 0, "rerun must be a no-op");
    }

    #[tokio::test]
    async fn migration_no_op_when_table_missing() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let updated = migrate_default_launch_package(&db)
            .await
            .expect("missing table must not error");
        assert_eq!(updated, 0);
    }
}
