use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite doesn't support IF NOT EXISTS for ALTER TABLE ADD COLUMN,
        // so we check if the column already exists first.
        let db = manager.get_connection();
        let result = db
            .query_one(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) as cnt FROM pragma_table_info('timers') WHERE name='preach_limit_seconds'",
            ))
            .await?;

        let has_column = result
            .map(|row| row.try_get::<i32>("", "cnt").unwrap_or(0) > 0)
            .unwrap_or(false);

        if !has_column {
            db.execute(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE timers ADD COLUMN preach_limit_seconds BIGINT NULL",
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite doesn't support DROP COLUMN before 3.35.0,
        // and this is a non-destructive addition, so down is a no-op.
        let _ = manager;
        Ok(())
    }
}
