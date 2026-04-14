use sea_orm_migration::prelude::*;

use crate::bible_fts_triggers::{CREATE_TRIGGER_STATEMENTS, TRIGGER_NAMES};

#[derive(DeriveMigrationName)]
pub struct Migration;

const FTS_TABLE: &str = "bible_passage_fts";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Standalone FTS5 table (not external-content) because bible_passages
        // uses string UUIDs as PKs and FTS5 content_rowid requires integers.
        // We store passage_id as an unindexed column for joining back.
        db.execute_unprepared(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS {FTS_TABLE} USING fts5(\
                passage_id UNINDEXED, \
                translation_code UNINDEXED, \
                book, \
                content, \
                tokenize='unicode61'\
            )"
        ))
        .await?;

        // Populate from existing passages
        db.execute_unprepared(&format!(
            "INSERT INTO {FTS_TABLE}(passage_id, translation_code, book, content) \
             SELECT id, translation_code, book, content FROM bible_passages"
        ))
        .await?;

        // Keep FTS in sync via triggers. The exact same trigger bodies are
        // recreated inside the fast-import transaction in presenter-persistence,
        // so both sites share the constants in bible_fts_triggers.
        for stmt in CREATE_TRIGGER_STATEMENTS {
            db.execute_unprepared(stmt).await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for name in TRIGGER_NAMES {
            db.execute_unprepared(&format!("DROP TRIGGER IF EXISTS {name}"))
                .await?;
        }
        db.execute_unprepared(&format!("DROP TABLE IF EXISTS {FTS_TABLE}"))
            .await?;
        Ok(())
    }
}
