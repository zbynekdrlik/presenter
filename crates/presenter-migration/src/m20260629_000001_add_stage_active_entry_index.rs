use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // #496: store which playlist ENTRY (by index) is active so a set that
        // repeats a song highlights/scrolls the correct occurrence on the
        // worship-pp stage. SQLite has no IF NOT EXISTS for ADD COLUMN, so
        // guard on pragma_table_info first (idempotent on existing DBs).
        let db = manager.get_connection();
        let result = db
            .query_one(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) as cnt FROM pragma_table_info('stage_state') WHERE name='active_entry_index'",
            ))
            .await?;

        let has_column = result
            .map(|row| row.try_get::<i32>("", "cnt").unwrap_or(0) > 0)
            .unwrap_or(false);

        if !has_column {
            db.execute(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE stage_state ADD COLUMN active_entry_index INTEGER NULL",
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite doesn't support DROP COLUMN before 3.35.0, and this is a
        // non-destructive addition, so down is a no-op.
        let _ = manager;
        Ok(())
    }
}
