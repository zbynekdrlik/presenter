//! #564: two-port model for `resolume_hosts`. `port` stays the user's
//! configured intent (never auto-changed); this adds `active_port` — the
//! port a runtime auto-discovery probe found Resolume Arena/Avenue actually
//! listening on, when it drifted from `port` (e.g. after Arena rebound to
//! the next port because its own restart raced ours). `NULL` means "dial
//! `port`". Additive + idempotent (`column_missing` guard), per the repo's
//! DB policy — safe to re-run on a DB that already has the column.
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

async fn column_missing<C: ConnectionTrait>(db: &C, column: &str) -> Result<bool, DbErr> {
    let sql = format!(
        "SELECT COUNT(*) AS cnt FROM pragma_table_info('resolume_hosts') WHERE name='{column}'"
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
        if column_missing(db, "active_port").await? {
            db.execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE resolume_hosts ADD COLUMN active_port INTEGER",
            ))
            .await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Non-destructive additive migration; SQLite pre-3.35 has no DROP
        // COLUMN, and dropping it would discard a legitimately-discovered
        // active port. No-op.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DbBackend};

    async fn setup_pre_migration_db() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE resolume_hosts (\
                id TEXT PRIMARY KEY, label TEXT NOT NULL, host TEXT NOT NULL, \
                port INTEGER NOT NULL DEFAULT 8090, is_enabled BOOLEAN NOT NULL DEFAULT 1, \
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL)"
                .to_string(),
        ))
        .await
        .expect("seed table predating active_port");
        db
    }

    /// RED before the migration (the column does not exist), GREEN after.
    #[tokio::test]
    async fn adds_active_port_to_a_db_that_lacks_it() {
        let db = setup_pre_migration_db().await;

        let before = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT active_port FROM resolume_hosts".to_string(),
            ))
            .await;
        assert!(
            before.is_err(),
            "precondition: active_port must not exist before the migration"
        );

        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("migration up");

        let after = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT active_port FROM resolume_hosts".to_string(),
            ))
            .await;
        assert!(
            after.is_ok(),
            "active_port must be queryable after the migration: {after:?}"
        );
    }

    /// Re-running on a DB that already has the column is a no-op and never
    /// touches existing rows (idempotent — SQLite errors re-adding a column).
    #[tokio::test]
    async fn is_idempotent_and_preserves_existing_rows() {
        let db = setup_pre_migration_db().await;
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("first up");

        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO resolume_hosts \
             (id, label, host, port, is_enabled, active_port, created_at, updated_at) \
             VALUES ('h1', 'Main', '10.0.0.5', 8090, 1, 8091, \
             '2026-07-17T00:00:00Z', '2026-07-17T00:00:00Z')"
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
                "SELECT active_port FROM resolume_hosts WHERE id = 'h1'".to_string(),
            ))
            .await
            .expect("query")
            .expect("row");
        let active_port: i32 = row.try_get_by("active_port").expect("active_port");
        assert_eq!(active_port, 8091, "re-run must preserve the existing value");
    }
}
