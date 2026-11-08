//! #915: `study_bookmarks`, the named reading places of `/ui/study` (the owner
//! chose a list of several named bookmarks, 2026-11-08). Stored on the server so
//! the phone and the PC show the same list. `owner_email` = the Cloudflare
//! Access e-mail of the writer; NULL = the shared list of the LAN, where there
//! is no e-mail. `book_name` = the translation's own name of the book, read when
//! the place was checked, so the list needs no book list to show its label.
//! Site-local (no sync columns), additive and idempotent (`IF NOT EXISTS`).
//!
//! No foreign key to `bible_translations`: a Bible re-import deletes and
//! re-inserts the translation row, and a cascading key would delete the
//! bookmarks with it (the sermon items' rule, spec §5.7).
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const CREATE_BOOKMARKS: &str = r#"CREATE TABLE IF NOT EXISTS "study_bookmarks" (
    "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
    "owner_email" text NULL,
    "name" text NOT NULL,
    "translation_code" text NOT NULL,
    "book_code" text NOT NULL,
    "book_name" text NOT NULL,
    "chapter" integer NOT NULL,
    "verse" integer NOT NULL,
    "position" integer NOT NULL,
    "created_at" text NOT NULL,
    "updated_at" text NOT NULL
)"#;

const CREATE_OWNER_INDEX: &str = r#"CREATE INDEX IF NOT EXISTS
    "idx_study_bookmarks_owner" ON "study_bookmarks" ("owner_email", "position")"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [CREATE_BOOKMARKS, CREATE_OWNER_INDEX] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A brand-new table in this migration; a rollback removes it.
        manager
            .get_connection()
            .execute_unprepared(r#"DROP TABLE IF EXISTS "study_bookmarks""#)
            .await?;
        Ok(())
    }
}
