//! #851 P2: sermons (spec §7.1). A sermon IS a Bible presentation plus one
//! `bible_sermons` row (service date, translations, author label, revision)
//! and its source items. Additive and idempotent (`IF NOT EXISTS`): the
//! `bible_*` tables hold production data, so this is a NEW migration, never an
//! edit of an applied one. Raw SQL like `m20260410_000001_separate_bible`, so
//! the schema reads exactly as the spec writes it.
//!
//! Deliberately NO foreign key from `bible_sermon_items.translation_code` to
//! `bible_translations` (spec §5.7): every Bible re-import deletes the
//! translation row and re-inserts it, and a cascading key would delete the
//! items with it. Items keep their own text snapshot instead.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const CREATE_SERMONS: &str = r#"CREATE TABLE IF NOT EXISTS "bible_sermons" (
    "presentation_id" varchar(36) NOT NULL PRIMARY KEY,
    "service_date" text NOT NULL,
    "default_translation_code" text NOT NULL,
    "secondary_translation_code" text NULL,
    "author_email" text NULL,
    "revision" integer NOT NULL DEFAULT 0,
    "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY ("presentation_id") REFERENCES "bible_presentations"("id")
        ON DELETE CASCADE ON UPDATE CASCADE
)"#;

const CREATE_SERMON_DATE_INDEX: &str = r#"CREATE INDEX IF NOT EXISTS
    "idx_bible_sermons_service_date" ON "bible_sermons" ("service_date")"#;

const CREATE_ITEMS: &str = r#"CREATE TABLE IF NOT EXISTS "bible_sermon_items" (
    "id" varchar(36) NOT NULL PRIMARY KEY,
    "presentation_id" varchar(36) NOT NULL,
    "item_order" integer NOT NULL,
    "translation_code" text NOT NULL,
    "book_code" text NOT NULL,
    "book_number" integer NOT NULL,
    "book_name" text NOT NULL,
    "chapter" integer NOT NULL,
    "verse_start" integer NOT NULL,
    "verse_end" integer NOT NULL,
    "source_json" text NOT NULL,
    "edits_json" text NOT NULL,
    "slide_count" integer NOT NULL DEFAULT 0,
    "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY ("presentation_id") REFERENCES "bible_sermons"("presentation_id")
        ON DELETE CASCADE
)"#;

const CREATE_ITEM_ORDER_INDEX: &str = r#"CREATE INDEX IF NOT EXISTS
    "idx_bible_sermon_items_order" ON "bible_sermon_items" ("presentation_id", "item_order")"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            CREATE_SERMONS,
            CREATE_SERMON_DATE_INDEX,
            CREATE_ITEMS,
            CREATE_ITEM_ORDER_INDEX,
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Brand-new tables in this migration; a rollback removes them (items
        // first, they reference the sermons).
        let db = manager.get_connection();
        db.execute_unprepared(r#"DROP TABLE IF EXISTS "bible_sermon_items""#)
            .await?;
        db.execute_unprepared(r#"DROP TABLE IF EXISTS "bible_sermons""#)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DbBackend, Statement};

    const SELECT_ALL_COLUMNS: [&str; 2] = [
        "SELECT presentation_id, service_date, default_translation_code, \
         secondary_translation_code, author_email, revision, created_at, updated_at \
         FROM bible_sermons",
        "SELECT id, presentation_id, item_order, translation_code, book_code, book_number, \
         book_name, chapter, verse_start, verse_end, source_json, edits_json, slide_count, \
         created_at, updated_at FROM bible_sermon_items",
    ];

    async fn one_connection_db() -> sea_orm::DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1).min_connections(1);
        Database::connect(options).await.expect("connect")
    }

    async fn query(db: &sea_orm::DatabaseConnection, sql: &str) -> Result<(), DbErr> {
        db.execute(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
            .map(|_| ())
    }

    #[tokio::test]
    async fn up_creates_both_tables_with_every_column_and_is_idempotent() {
        let db = one_connection_db().await;
        for sql in SELECT_ALL_COLUMNS {
            assert!(query(&db, sql).await.is_err(), "precondition: {sql}");
        }
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("first up");
        Migration.up(&manager).await.expect("second up is a no-op");
        for sql in SELECT_ALL_COLUMNS {
            assert!(query(&db, sql).await.is_ok(), "after up: {sql}");
        }
    }

    #[tokio::test]
    async fn down_removes_both_tables() {
        let db = one_connection_db().await;
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("up");
        Migration.down(&manager).await.expect("down");
        for sql in SELECT_ALL_COLUMNS {
            assert!(query(&db, sql).await.is_err(), "after down: {sql}");
        }
    }
}
