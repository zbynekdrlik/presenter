use sea_orm_migration::prelude::*;

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

        // Keep FTS in sync via triggers
        db.execute_unprepared(&format!(
            "CREATE TRIGGER IF NOT EXISTS bible_passage_fts_insert \
             AFTER INSERT ON bible_passages BEGIN \
                INSERT INTO {FTS_TABLE}(passage_id, translation_code, book, content) \
                VALUES (new.id, new.translation_code, new.book, new.content); \
             END"
        ))
        .await?;

        db.execute_unprepared(&format!(
            "CREATE TRIGGER IF NOT EXISTS bible_passage_fts_delete \
             AFTER DELETE ON bible_passages BEGIN \
                DELETE FROM {FTS_TABLE} WHERE passage_id = old.id; \
             END"
        ))
        .await?;

        db.execute_unprepared(&format!(
            "CREATE TRIGGER IF NOT EXISTS bible_passage_fts_update \
             AFTER UPDATE ON bible_passages BEGIN \
                DELETE FROM {FTS_TABLE} WHERE passage_id = old.id; \
                INSERT INTO {FTS_TABLE}(passage_id, translation_code, book, content) \
                VALUES (new.id, new.translation_code, new.book, new.content); \
             END"
        ))
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TRIGGER IF EXISTS bible_passage_fts_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS bible_passage_fts_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS bible_passage_fts_update")
            .await?;
        db.execute_unprepared(&format!("DROP TABLE IF EXISTS {FTS_TABLE}"))
            .await?;
        Ok(())
    }
}
