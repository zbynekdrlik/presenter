//! #852 šepkár tips (spec §9.2): one additive, idempotent table
//! `interpreter_tips` — short tips a šepkár sends the interpreter from
//! `/ui/sepkar`. Site-local runtime data: NO sync columns, so the SNV ↔ PP
//! song/library sync (which only knows presentations and libraries) never
//! sees it. The server deletes rows older than 30 days.
//!
//! `service_date` is TEXT `YYYY-MM-DD` (the server's local date at creation):
//! the active list, the duplicate check and the retention filter on it by
//! equality / lexical order, never by comparing stored timestamp strings
//! (`repository-error-pattern.md`). Additive + idempotent per the repo DB
//! policy (prod data): `if_not_exists` table + index, never an edit of an
//! applied migration.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum InterpreterTips {
    Table,
    Id,
    ServiceDate,
    SourceTerm,
    TargetTerm,
    Note,
    SenderName,
    SenderIp,
    UserAgent,
    CreatedAt,
    DismissedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(InterpreterTips::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(InterpreterTips::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(InterpreterTips::ServiceDate)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(InterpreterTips::SourceTerm).text().null())
                    .col(ColumnDef::new(InterpreterTips::TargetTerm).text().null())
                    .col(ColumnDef::new(InterpreterTips::Note).text().null())
                    .col(
                        ColumnDef::new(InterpreterTips::SenderName)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(InterpreterTips::SenderIp).text().not_null())
                    .col(ColumnDef::new(InterpreterTips::UserAgent).text().null())
                    .col(
                        ColumnDef::new(InterpreterTips::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .col(
                        ColumnDef::new(InterpreterTips::DismissedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_interpreter_tips_service_date")
                    .table(InterpreterTips::Table)
                    .col(InterpreterTips::ServiceDate)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Brand-new table in this migration; a rollback simply removes it.
        manager
            .drop_table(
                Table::drop()
                    .table(InterpreterTips::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

    const SELECT_ALL_COLUMNS: &str = "SELECT id, service_date, source_term, target_term, note, \
         sender_name, sender_ip, user_agent, created_at, dismissed_at FROM interpreter_tips";

    #[tokio::test]
    async fn up_creates_interpreter_tips_table() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let before = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                SELECT_ALL_COLUMNS.to_string(),
            ))
            .await;
        assert!(
            before.is_err(),
            "precondition: no table before the migration"
        );

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("migration up");

        let after = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                SELECT_ALL_COLUMNS.to_string(),
            ))
            .await;
        assert!(after.is_ok(), "full column set after up: {after:?}");
    }

    #[tokio::test]
    async fn up_is_idempotent_and_preserves_rows() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("first up");
        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO interpreter_tips (service_date, note, sender_name, sender_ip) \
             VALUES ('2026-10-18', 'tip', 'Peter', '192.0.2.1')"
                .to_string(),
        ))
        .await
        .expect("insert");
        Migration
            .up(&manager)
            .await
            .expect("second up must be a no-op");
        let n: i64 = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM interpreter_tips".to_string(),
            ))
            .await
            .expect("query")
            .expect("row")
            .try_get_by("n")
            .expect("count");
        assert_eq!(n, 1, "a re-run keeps the rows");
    }
}
