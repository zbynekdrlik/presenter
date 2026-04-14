use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let result = db
            .query_one(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) as cnt FROM pragma_table_info('bible_translations') WHERE name='source_digest'",
            ))
            .await?;

        let has_column = result
            .map(|row| row.try_get::<i32>("", "cnt").unwrap_or(0) > 0)
            .unwrap_or(false);

        if !has_column {
            db.execute(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE bible_translations ADD COLUMN source_digest TEXT NULL",
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite DROP COLUMN support is limited; non-destructive addition — no-op.
        let _ = manager;
        Ok(())
    }
}
