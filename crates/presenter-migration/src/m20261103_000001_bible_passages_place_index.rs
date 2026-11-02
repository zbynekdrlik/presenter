//! #851 D4: a verse by translation, book code, chapter and verse. The reference
//! index starts with the book NAME, which a study read does not have, so every
//! such lookup scanned the whole translation.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const CREATE_INDEX: &str = r#"CREATE INDEX IF NOT EXISTS "idx_bible_passages_place"
    ON "bible_passages" ("translation_code", "book_code", "chapter", "verse_start")"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(CREATE_INDEX)
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(r#"DROP INDEX IF EXISTS "idx_bible_passages_place""#)
            .await?;
        Ok(())
    }
}
