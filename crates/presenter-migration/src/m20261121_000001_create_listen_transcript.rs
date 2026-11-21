//! #852 C3-persist (spec §12, §22.14): `listen_transcript`, the guest's closed
//! sentences kept 7 days, so a restart mid-sermon (a deploy, a crash) keeps
//! the interpreter's EN column and C4 / C6 can read a service back later.
//! One row per closed sentence: the `/listen/ws` sentence fields plus the
//! server's local `service_date` (TEXT `YYYY-MM-DD`, filtered by equality /
//! lexical order like `interpreter_tips`). Site-local (no sync columns). The
//! server deletes rows older than 7 days. Additive + idempotent per the repo
//! DB policy (prod data): `if_not_exists` table + index, never an edit of an
//! applied migration.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum ListenTranscript {
    Table,
    Id,
    MicId,
    SentenceId,
    Text,
    StartMs,
    EndMs,
    ClosedAt,
    Language,
    ServiceDate,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ListenTranscript::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ListenTranscript::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ListenTranscript::MicId).text().not_null())
                    .col(
                        ColumnDef::new(ListenTranscript::SentenceId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ListenTranscript::Text).text().not_null())
                    .col(
                        ColumnDef::new(ListenTranscript::StartMs)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ListenTranscript::EndMs)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ListenTranscript::ClosedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ListenTranscript::Language).text().null())
                    .col(
                        ColumnDef::new(ListenTranscript::ServiceDate)
                            .text()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_listen_transcript_service_date_id")
                    .table(ListenTranscript::Table)
                    .col(ListenTranscript::ServiceDate)
                    .col(ListenTranscript::Id)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Brand-new table in this migration; a rollback simply removes it.
        manager
            .drop_table(
                Table::drop()
                    .table(ListenTranscript::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};

    async fn exec(db: &DatabaseConnection, sql: &str) -> Result<sea_orm::ExecResult, DbErr> {
        db.execute(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
    }

    async fn count(db: &DatabaseConnection, sql: &str) -> i64 {
        db.query_one(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
            .expect("query")
            .expect("row")
            .try_get_by("n")
            .expect("count")
    }

    #[tokio::test]
    async fn up_creates_the_table_on_a_database_with_data_and_keeps_that_data() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        exec(
            &db,
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT)",
        )
        .await
        .expect("existing table");
        exec(&db, "INSERT INTO app_settings VALUES ('listen.mics', '[]')")
            .await
            .expect("existing row");
        let select = "SELECT id, mic_id, sentence_id, text, start_ms, end_ms, closed_at, \
                      language, service_date FROM listen_transcript";
        assert!(exec(&db, select).await.is_err(), "no table before up");

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("migration up");

        assert!(exec(&db, select).await.is_ok(), "every column after up");
        assert_eq!(
            count(&db, "SELECT COUNT(*) AS n FROM app_settings").await,
            1,
            "existing data kept"
        );
        let index = count(
            &db,
            "SELECT COUNT(*) AS n FROM sqlite_master WHERE type = 'index' \
             AND name = 'idx_listen_transcript_service_date_id'",
        )
        .await;
        assert_eq!(index, 1, "the service_date + id index exists");
    }

    #[tokio::test]
    async fn up_is_idempotent_and_preserves_rows() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("first up");
        exec(
            &db,
            "INSERT INTO listen_transcript (mic_id, sentence_id, text, start_ms, end_ms, \
             closed_at, language, service_date) VALUES ('guest', 1, 'Grace.', 0, 900, \
             '2026-11-22T09:30:05+00:00', 'en', '2026-11-22')",
        )
        .await
        .expect("insert");
        Migration.up(&manager).await.expect("second up is a no-op");
        assert_eq!(
            count(&db, "SELECT COUNT(*) AS n FROM listen_transcript").await,
            1,
            "a re-run keeps the rows"
        );
    }
}
