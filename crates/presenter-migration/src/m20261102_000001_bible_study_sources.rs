//! #851 D3a, D3b: Strong's dictionaries and cross-references (spec §5.4, §5.5).
//! Filled by `ingest_bibles`; a source's rows go with its row through the
//! cascades. `to_book_number_end` is here because 16 source ranges cross a book.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const CREATE_SOURCES: &str = r#"CREATE TABLE IF NOT EXISTS "bible_study_sources" (
    "code" text NOT NULL PRIMARY KEY,
    "name" text NOT NULL,
    "license" text NOT NULL,
    "source_digest" text NOT NULL,
    "imported_at" text NOT NULL
)"#;

const CREATE_LEXICON: &str = r#"CREATE TABLE IF NOT EXISTS "bible_lexicon_entries" (
    "strongs" text NOT NULL PRIMARY KEY,
    "source_code" text NOT NULL
        REFERENCES "bible_study_sources"("code") ON DELETE CASCADE,
    "lemma" text NOT NULL,
    "transliteration" text NOT NULL,
    "pronunciation" text NOT NULL,
    "definition" text NOT NULL,
    "derivation" text NOT NULL,
    "kjv_renderings" text NOT NULL,
    "related" text NOT NULL
) WITHOUT ROWID"#;

const CREATE_LEXICON_SOURCE_INDEX: &str = r#"CREATE INDEX IF NOT EXISTS
    "idx_bible_lexicon_entries_source" ON "bible_lexicon_entries" ("source_code")"#;

const CREATE_CROSS_REFERENCES: &str = r#"CREATE TABLE IF NOT EXISTS "bible_cross_references" (
    "source_code" text NOT NULL
        REFERENCES "bible_study_sources"("code") ON DELETE CASCADE,
    "from_book_number" integer NOT NULL,
    "from_chapter" integer NOT NULL,
    "from_verse" integer NOT NULL,
    "to_book_number" integer NOT NULL,
    "to_chapter" integer NOT NULL,
    "to_verse" integer NOT NULL,
    "to_book_number_end" integer NOT NULL,
    "to_chapter_end" integer NOT NULL,
    "to_verse_end" integer NOT NULL,
    "votes" integer NOT NULL,
    PRIMARY KEY ("from_book_number", "from_chapter", "from_verse",
                 "to_book_number", "to_chapter", "to_verse")
) WITHOUT ROWID"#;

const CREATE_CROSS_REFERENCES_SOURCE_INDEX: &str = r#"CREATE INDEX IF NOT EXISTS
    "idx_bible_cross_references_source" ON "bible_cross_references" ("source_code")"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            CREATE_SOURCES,
            CREATE_LEXICON,
            CREATE_LEXICON_SOURCE_INDEX,
            CREATE_CROSS_REFERENCES,
            CREATE_CROSS_REFERENCES_SOURCE_INDEX,
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for table in [
            "bible_cross_references",
            "bible_lexicon_entries",
            "bible_study_sources",
        ] {
            db.execute_unprepared(&format!(r#"DROP TABLE IF EXISTS "{table}""#))
                .await?;
        }
        Ok(())
    }
}
