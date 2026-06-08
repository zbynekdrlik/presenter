use anyhow::anyhow;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseBackend, EntityTrait, Set, Statement,
    TransactionTrait,
};
use tracing::instrument;
use uuid::Uuid;

use crate::entities::{bible_passage, bible_translation};
use presenter_core::{
    bible::{canonical_book_by_name, BibleIngestionBatch},
    BibleTranslation,
};

use crate::repository::util::{to_domain_translation, BIBLE_INSERT_CHUNK};
use crate::repository::Repository;

impl Repository {
    #[instrument(skip_all)]
    /// Replace all passages for a translation via the fast-import path:
    /// drop the `bible_passage_fts` triggers inside the transaction,
    /// bulk-insert passages, bulk-populate FTS with one `INSERT ... SELECT`,
    /// recreate the triggers. A rollback restores the triggers atomically.
    ///
    /// Callers MUST ensure no other connection is writing to `bible_passages`
    /// during this call — the triggers are briefly absent mid-transaction, so
    /// concurrent inserts/updates/deletes from another writer would leave FTS
    /// out of sync. The deploy workflow stops the server before running
    /// `ingest_bibles`, which guarantees this.
    pub async fn replace_bible_translation_passages(
        &self,
        batch: &BibleIngestionBatch,
    ) -> anyhow::Result<()> {
        use presenter_migration::bible_fts_triggers::{CREATE_TRIGGER_STATEMENTS, TRIGGER_NAMES};
        const SQLITE: DatabaseBackend = DatabaseBackend::Sqlite;

        let (translation, passages) = batch.clone().into_parts();
        let existing = bible_translation::Entity::find_by_id(translation.code.clone())
            .one(&self.db)
            .await?;
        let preserve_dashboard = existing
            .as_ref()
            .map(|model| model.show_in_dashboard)
            .unwrap_or(translation.show_in_dashboard);
        let preserve_digest = existing
            .as_ref()
            .and_then(|model| model.source_digest.clone());

        let txn = self.db.begin().await?;

        // 1. Clear this translation's FTS rows (other translations untouched)
        txn.execute(Statement::from_sql_and_values(
            SQLITE,
            "DELETE FROM bible_passage_fts WHERE translation_code = ?",
            [translation.code.clone().into()],
        ))
        .await?;

        // 2. Drop the FTS triggers inside the transaction. SQLite supports DDL
        //    in transactions; if we roll back, the triggers are restored.
        for trig in TRIGGER_NAMES {
            txn.execute(Statement::from_string(
                SQLITE,
                format!("DROP TRIGGER IF EXISTS {trig}"),
            ))
            .await?;
        }

        // 3. Delete old translation row (cascades to old passages via FK)
        bible_translation::Entity::delete_by_id(translation.code.clone())
            .exec(&txn)
            .await?;

        // 4. Insert fresh translation row (preserving show_in_dashboard and existing digest)
        let translation_model = bible_translation::ActiveModel {
            code: Set(translation.code.clone()),
            name: Set(translation.name.clone()),
            language: Set(translation.language.clone()),
            show_in_dashboard: Set(preserve_dashboard),
            source: Set(translation.source.clone()),
            created_at: Set(Utc::now().into()),
            source_digest: Set(preserve_digest),
        };
        bible_translation::Entity::insert(translation_model)
            .exec(&txn)
            .await?;

        // 5. Batch-insert passages (no trigger overhead because triggers are dropped)
        let mut chunk = Vec::with_capacity(BIBLE_INSERT_CHUNK);
        for passage in passages {
            let reference = &passage.reference;
            let (code, number) = match &reference.book_code {
                Some(c) => (c.clone(), reference.book_number.unwrap_or(0) as i32),
                None => match canonical_book_by_name(&reference.book) {
                    Some(meta) => (meta.code.to_string(), meta.number as i32),
                    None => (reference.book.clone(), 0),
                },
            };
            let model = bible_passage::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                translation_code: Set(translation.code.clone()),
                book: Set(reference.book.clone()),
                book_code: Set(code),
                book_number: Set(number),
                chapter: Set(reference.chapter as i32),
                verse_start: Set(reference.verse_start as i32),
                verse_end: Set(reference.verse_end as i32),
                content: Set(passage.text.clone()),
                created_at: Set(Utc::now().into()),
            };

            chunk.push(model);
            if chunk.len() == BIBLE_INSERT_CHUNK {
                let to_insert = std::mem::take(&mut chunk);
                bible_passage::Entity::insert_many(to_insert)
                    .exec(&txn)
                    .await?;
            }
        }

        if !chunk.is_empty() {
            bible_passage::Entity::insert_many(chunk).exec(&txn).await?;
        }

        // 6. Bulk populate FTS from the freshly-inserted passages
        txn.execute(Statement::from_sql_and_values(
            SQLITE,
            "INSERT INTO bible_passage_fts(passage_id, translation_code, book, content) \
             SELECT id, translation_code, book, content FROM bible_passages \
             WHERE translation_code = ?",
            [translation.code.clone().into()],
        ))
        .await?;

        // 7. Recreate the FTS triggers. Bodies live in
        //    presenter_migration::bible_fts_triggers so the schema and the
        //    fast-import path can never drift.
        for stmt in CREATE_TRIGGER_STATEMENTS {
            txn.execute(Statement::from_string(SQLITE, stmt.to_string()))
                .await?;
        }

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn set_bible_source_digest(&self, code: &str, digest: &str) -> anyhow::Result<()> {
        let existing = bible_translation::Entity::find_by_id(code.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("bible_translation {code} not found"))?;
        let mut active: bible_translation::ActiveModel = existing.into();
        active.source_digest = Set(Some(digest.to_string()));
        active.update(&self.db).await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn update_bible_translation(
        &self,
        code: &str,
        name: Option<&str>,
        language: Option<&str>,
        show_in_dashboard: Option<bool>,
    ) -> anyhow::Result<Option<BibleTranslation>> {
        let model = bible_translation::Entity::find_by_id(code.to_string())
            .one(&self.db)
            .await?;
        let Some(model) = model else {
            return Ok(None);
        };
        let mut active: bible_translation::ActiveModel = model.into();
        if let Some(name) = name {
            active.name = Set(name.to_string());
        }
        if let Some(language) = language {
            active.language = Set(language.to_string());
        }
        if let Some(show_in_dashboard) = show_in_dashboard {
            active.show_in_dashboard = Set(show_in_dashboard);
        }
        let saved = active.update(&self.db).await?;
        Ok(Some(to_domain_translation(saved)))
    }

    #[instrument(skip_all)]
    pub async fn delete_bible_translation(&self, code: &str) -> anyhow::Result<bool> {
        let result = bible_translation::Entity::delete_by_id(code.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    #[instrument(skip_all)]
    pub async fn set_all_bible_dashboard_pins(&self, pinned: bool) -> anyhow::Result<u64> {
        let result = bible_translation::Entity::update_many()
            .col_expr(
                bible_translation::Column::ShowInDashboard,
                sea_orm::sea_query::Expr::value(pinned),
            )
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }
}
