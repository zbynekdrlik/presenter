//! #851 D2b: verse markup and Strong's occurrences (spec §5.3). Side tables, so
//! `bible_passages` and every search over it stay as they are. Additive and
//! idempotent; both rows go with their translation through the cascades, and
//! the import rebuilds them in its own transaction.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const CREATE_MARKUP: &str = r#"CREATE TABLE IF NOT EXISTS "bible_passage_markup" (
    "passage_id" text NOT NULL PRIMARY KEY
        REFERENCES "bible_passages"("id") ON DELETE CASCADE,
    "markup_json" text NOT NULL
) WITHOUT ROWID"#;

const CREATE_OCCURRENCES: &str = r#"CREATE TABLE IF NOT EXISTS "bible_strongs_occurrences" (
    "translation_code" text NOT NULL
        REFERENCES "bible_translations"("code") ON DELETE CASCADE ON UPDATE CASCADE,
    "strongs" text NOT NULL,
    "book_code" text NOT NULL,
    "book_number" integer NOT NULL,
    "chapter" integer NOT NULL,
    "verse" integer NOT NULL,
    "word_count" integer NOT NULL,
    PRIMARY KEY ("translation_code", "strongs", "book_number", "chapter", "verse")
) WITHOUT ROWID"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(CREATE_MARKUP).await?;
        db.execute_unprepared(CREATE_OCCURRENCES).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(r#"DROP TABLE IF EXISTS "bible_strongs_occurrences""#)
            .await?;
        db.execute_unprepared(r#"DROP TABLE IF EXISTS "bible_passage_markup""#)
            .await?;
        Ok(())
    }
}
