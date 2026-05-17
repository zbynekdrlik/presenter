use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let backend = conn.get_database_backend();

        // The `ableset_settings` table is created lazily by the repository on first
        // access (see `Repository::ensure_ableset_settings_table`). On a fresh DB the
        // table will not exist yet — there is no legacy data to fix in that case.
        let table_check = conn
            .query_one(sea_orm::Statement::from_string(
                backend,
                "SELECT name FROM sqlite_master WHERE type='table' AND name='ableset_settings'"
                    .to_string(),
            ))
            .await?;
        if table_check.is_none() {
            return Ok(());
        }

        // Replace any legacy port=5950 and library="NEWLEVEL" with the new defaults.
        // Idempotent: only touches rows that still match the legacy values.
        let new_http_port: i32 = 80;
        let new_osc_port: i32 = 39051;
        let new_library = "NEW LEVEL";

        conn.execute(sea_orm::Statement::from_sql_and_values(
            backend,
            "UPDATE ableset_settings SET http_port = ?1 WHERE http_port = 5950",
            [new_http_port.into()],
        ))
        .await?;
        conn.execute(sea_orm::Statement::from_sql_and_values(
            backend,
            "UPDATE ableset_settings SET osc_port = ?1 WHERE osc_port = 5950",
            [new_osc_port.into()],
        ))
        .await?;
        conn.execute(sea_orm::Statement::from_sql_and_values(
            backend,
            "UPDATE ableset_settings SET library_name = ?1 WHERE UPPER(library_name) = 'NEWLEVEL'",
            [new_library.into()],
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // No down — legacy values cannot be safely restored.
        Ok(())
    }
}
