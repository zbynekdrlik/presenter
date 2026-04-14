use anyhow::anyhow;
use chrono::Utc;
use sea_orm::{
    sea_query::Expr as SeaExpr, ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend,
    EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Statement,
    TransactionTrait,
};
use tracing::instrument;

use crate::entities::{bible_passage, bible_presentation, bible_slide, bible_translation};
use presenter_core::{
    bible::{canonical_book_by_name, BibleIngestionBatch},
    search::fold_query,
    slide::BibleSlideMetadata,
    BiblePassage, BiblePresentation, BiblePresentationId, BiblePresentationSlide,
    BiblePresentationSummary, BibleReference, BibleSlideId, BibleTranslation, SlideText,
};
use sea_orm::Set;
use uuid::Uuid;

use super::util::{
    sanitize_like_input, to_domain_passage, to_domain_translation, RepositoryError,
    BIBLE_INSERT_CHUNK,
};
use super::Repository;

/// Build an FTS5 MATCH query from free-text input.
///
/// Splits into words, drops tokens shorter than 2 chars, appends `*` for
/// prefix matching, and joins with spaces (implicit AND in FTS5).
/// Returns `None` if no usable tokens remain.
fn build_fts_query(input: &str) -> Option<String> {
    let tokens: Vec<String> = input
        .split_whitespace()
        .filter(|w| w.len() >= 2)
        .map(|w| {
            // Strip FTS5 special characters to prevent query syntax errors
            let cleaned: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
            format!("{cleaned}*")
        })
        .filter(|t| t.len() > 1) // skip bare "*"
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

impl Repository {
    #[instrument(skip_all)]
    pub async fn replace_bible_translation_passages(
        &self,
        batch: &BibleIngestionBatch,
    ) -> anyhow::Result<()> {
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
        for trig in [
            "bible_passage_fts_insert",
            "bible_passage_fts_delete",
            "bible_passage_fts_update",
        ] {
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

        // 7. Recreate the FTS triggers with the same bodies as the original migration
        txn.execute(Statement::from_string(
            SQLITE,
            "CREATE TRIGGER bible_passage_fts_insert \
             AFTER INSERT ON bible_passages BEGIN \
                INSERT INTO bible_passage_fts(passage_id, translation_code, book, content) \
                VALUES (new.id, new.translation_code, new.book, new.content); \
             END"
            .to_string(),
        ))
        .await?;
        txn.execute(Statement::from_string(
            SQLITE,
            "CREATE TRIGGER bible_passage_fts_delete \
             AFTER DELETE ON bible_passages BEGIN \
                DELETE FROM bible_passage_fts WHERE passage_id = old.id; \
             END"
            .to_string(),
        ))
        .await?;
        txn.execute(Statement::from_string(
            SQLITE,
            "CREATE TRIGGER bible_passage_fts_update \
             AFTER UPDATE ON bible_passages BEGIN \
                DELETE FROM bible_passage_fts WHERE passage_id = old.id; \
                INSERT INTO bible_passage_fts(passage_id, translation_code, book, content) \
                VALUES (new.id, new.translation_code, new.book, new.content); \
             END"
            .to_string(),
        ))
        .await?;

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn get_bible_source_digest(&self, code: &str) -> anyhow::Result<Option<String>> {
        let model = bible_translation::Entity::find_by_id(code.to_string())
            .one(&self.db)
            .await?;
        Ok(model.and_then(|m| m.source_digest))
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
    pub async fn list_bible_translations(&self) -> anyhow::Result<Vec<BibleTranslation>> {
        let models = bible_translation::Entity::find()
            .order_by_asc(bible_translation::Column::Name)
            .all(&self.db)
            .await?;
        Ok(models.into_iter().map(to_domain_translation).collect())
    }

    #[instrument(skip_all)]
    pub async fn search_bible_passages(
        &self,
        translation_code: &str,
        query: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<BiblePassage>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let translation = bible_translation::Entity::find_by_id(translation_code.to_string())
            .one(&self.db)
            .await?;
        let Some(translation) = translation else {
            return Ok(Vec::new());
        };

        let rows = self
            .fts_search(query, Some(translation_code), limit)
            .await?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            results.push(to_domain_passage(row, translation.clone())?);
        }
        Ok(results)
    }

    #[instrument(skip_all)]
    pub async fn search_bible_passages_cross(
        &self,
        translation_code: Option<&str>,
        query: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<BiblePassage>> {
        if let Some(code) = translation_code {
            return self.search_bible_passages(code, query, limit).await;
        }

        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let all_translations: std::collections::HashMap<String, bible_translation::Model> =
            bible_translation::Entity::find()
                .all(&self.db)
                .await?
                .into_iter()
                .map(|m| (m.code.clone(), m))
                .collect();

        if all_translations.is_empty() {
            return Ok(Vec::new());
        }

        let rows = self.fts_search(query, None, limit).await?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let tc = row.translation_code.clone();
            if let Some(translation) = all_translations.get(&tc) {
                results.push(to_domain_passage(row, translation.clone())?);
            }
        }
        Ok(results)
    }

    /// Execute a full-text search against `bible_passage_fts`.
    /// Falls back to LIKE if the FTS table does not exist.
    async fn fts_search(
        &self,
        query: &str,
        translation_code: Option<&str>,
        limit: u32,
    ) -> anyhow::Result<Vec<bible_passage::Model>> {
        if let Some(fts_query) = build_fts_query(query) {
            let result = self
                .fts_search_raw(&fts_query, translation_code, limit)
                .await;
            match result {
                Ok(rows) => return Ok(rows),
                Err(e) => {
                    // FTS table might not exist (e.g. old database). Fall back to LIKE.
                    tracing::warn!(error = %e, "FTS search failed, falling back to LIKE");
                }
            }
        }

        // Fallback: LIKE-based search
        let pattern = format!("%{}%", sanitize_like_input(query));
        let mut q =
            bible_passage::Entity::find().filter(bible_passage::Column::Content.like(pattern));
        if let Some(code) = translation_code {
            q = q.filter(bible_passage::Column::TranslationCode.eq(code.to_string()));
        }
        Ok(q.order_by_asc(bible_passage::Column::BookNumber)
            .order_by_asc(bible_passage::Column::Chapter)
            .order_by_asc(bible_passage::Column::VerseStart)
            .limit(limit as u64)
            .all(&self.db)
            .await?)
    }

    /// Raw FTS5 query returning passage models joined via passage_id.
    async fn fts_search_raw(
        &self,
        fts_query: &str,
        translation_code: Option<&str>,
        limit: u32,
    ) -> anyhow::Result<Vec<bible_passage::Model>> {
        let (sql, values) = if let Some(code) = translation_code {
            (
                "SELECT bp.id, bp.translation_code, bp.book, bp.book_code, \
                        bp.book_number, bp.chapter, bp.verse_start, bp.verse_end, \
                        bp.content, bp.created_at \
                 FROM bible_passage_fts fts \
                 JOIN bible_passages bp ON bp.id = fts.passage_id \
                 WHERE bible_passage_fts MATCH ?1 \
                   AND fts.translation_code = ?2 \
                 ORDER BY fts.rank \
                 LIMIT ?3"
                    .to_string(),
                vec![
                    sea_orm::Value::from(fts_query.to_string()),
                    sea_orm::Value::from(code.to_string()),
                    sea_orm::Value::from(limit as i32),
                ],
            )
        } else {
            (
                "SELECT bp.id, bp.translation_code, bp.book, bp.book_code, \
                        bp.book_number, bp.chapter, bp.verse_start, bp.verse_end, \
                        bp.content, bp.created_at \
                 FROM bible_passage_fts fts \
                 JOIN bible_passages bp ON bp.id = fts.passage_id \
                 WHERE bible_passage_fts MATCH ?1 \
                 ORDER BY fts.rank \
                 LIMIT ?2"
                    .to_string(),
                vec![
                    sea_orm::Value::from(fts_query.to_string()),
                    sea_orm::Value::from(limit as i32),
                ],
            )
        };

        let stmt = Statement::from_sql_and_values(DatabaseBackend::Sqlite, &sql, values);
        let rows = bible_passage::Model::find_by_statement(stmt)
            .all(&self.db)
            .await?;
        Ok(rows)
    }

    #[instrument(skip_all)]
    pub async fn find_bible_passage(
        &self,
        translation_code: &str,
        reference: &BibleReference,
    ) -> anyhow::Result<Option<BiblePassage>> {
        let translation = bible_translation::Entity::find_by_id(translation_code.to_string())
            .one(&self.db)
            .await?;
        let Some(translation) = translation else {
            return Ok(None);
        };

        let mut query = bible_passage::Entity::find()
            .filter(bible_passage::Column::TranslationCode.eq(translation_code.to_string()))
            .filter(bible_passage::Column::Chapter.eq(reference.chapter as i32))
            .filter(bible_passage::Column::VerseStart.eq(reference.verse_start as i32))
            .filter(bible_passage::Column::VerseEnd.eq(reference.verse_end as i32));
        if let Some(code) = reference.book_code.as_deref() {
            query = query.filter(bible_passage::Column::BookCode.eq(code.to_string()));
        } else {
            query = query.filter(bible_passage::Column::Book.eq(reference.book.clone()));
        }
        let passage = query.one(&self.db).await?;

        Ok(passage
            .map(|model| to_domain_passage(model, translation.clone()))
            .transpose()?)
    }
}

use presenter_core::bible::BibleBookChapterSummary;
use sea_orm::sea_query::Expr;

impl Repository {
    #[instrument(skip_all)]
    pub async fn bible_passage_range(
        &self,
        translation_code: &str,
        book: &str,
        book_code: Option<&str>,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
    ) -> anyhow::Result<Vec<BiblePassage>> {
        let translation = bible_translation::Entity::find_by_id(translation_code.to_string())
            .one(&self.db)
            .await?;
        let Some(translation) = translation else {
            return Ok(Vec::new());
        };

        let mut query = bible_passage::Entity::find()
            .filter(bible_passage::Column::TranslationCode.eq(translation_code.to_string()))
            .filter(bible_passage::Column::Chapter.eq(chapter as i32))
            .filter(bible_passage::Column::VerseStart.gte(verse_start as i32))
            .filter(bible_passage::Column::VerseEnd.lte(verse_end as i32))
            .order_by_asc(bible_passage::Column::VerseStart);

        if let Some(code) = book_code {
            query = query.filter(bible_passage::Column::BookCode.eq(code.to_string()));
        } else {
            query = query.filter(bible_passage::Column::Book.eq(book.to_string()));
        }

        let rows = query.all(&self.db).await?;
        let mut passages = Vec::with_capacity(rows.len());
        for row in rows {
            passages.push(super::util::to_domain_passage(row, translation.clone())?);
        }
        Ok(passages)
    }

    #[instrument(skip_all)]
    pub async fn bible_book_chapter_summaries(
        &self,
        translation_code: &str,
    ) -> anyhow::Result<Vec<BibleBookChapterSummary>> {
        #[derive(Debug, FromQueryResult)]
        struct ChapterRow {
            book: String,
            book_code: String,
            book_number: i32,
            chapter: i32,
            verse_count: i32,
        }

        let rows = bible_passage::Entity::find()
            .select_only()
            .column(bible_passage::Column::Book)
            .column(bible_passage::Column::BookCode)
            .column(bible_passage::Column::BookNumber)
            .column(bible_passage::Column::Chapter)
            .column_as(
                Expr::col(bible_passage::Column::VerseEnd).max(),
                "verse_count",
            )
            .filter(bible_passage::Column::TranslationCode.eq(translation_code.to_string()))
            .group_by(bible_passage::Column::Book)
            .group_by(bible_passage::Column::BookCode)
            .group_by(bible_passage::Column::BookNumber)
            .group_by(bible_passage::Column::Chapter)
            .order_by_asc(bible_passage::Column::Book)
            .order_by_asc(bible_passage::Column::Chapter)
            .into_model::<ChapterRow>()
            .all(&self.db)
            .await?;
        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            summaries.push(BibleBookChapterSummary {
                book: row.book,
                book_code: Some(row.book_code),
                book_number: Some((row.book_number.max(0)) as u16),
                chapter: row.chapter.max(0) as u16,
                verse_count: row.verse_count.max(0) as u16,
            });
        }
        Ok(summaries)
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

    // ── Bible presentations ────────────────────────────────────────

    #[instrument(skip_all)]
    pub async fn list_bible_presentation_summaries(
        &self,
    ) -> anyhow::Result<Vec<BiblePresentationSummary>> {
        let presentations = bible_presentation::Entity::find()
            .order_by_asc(bible_presentation::Column::Name)
            .all(&self.db)
            .await?;

        #[derive(FromQueryResult)]
        struct CountRow {
            presentation_id: String,
            slide_count: i64,
        }

        let count_rows: Vec<CountRow> = bible_slide::Entity::find()
            .select_only()
            .column(bible_slide::Column::PresentationId)
            .column_as(Expr::col(bible_slide::Column::Id).count(), "slide_count")
            .group_by(bible_slide::Column::PresentationId)
            .into_model::<CountRow>()
            .all(&self.db)
            .await?;

        let counts: std::collections::HashMap<String, usize> = count_rows
            .into_iter()
            .map(|r| (r.presentation_id, r.slide_count as usize))
            .collect();

        let mut summaries = Vec::with_capacity(presentations.len());
        for model in presentations {
            let slide_count = counts.get(&model.id).copied().unwrap_or(0);
            let uuid = Uuid::parse_str(&model.id)
                .map_err(|_| RepositoryError::InvalidUuid(model.id.clone()))?;
            summaries.push(BiblePresentationSummary {
                id: BiblePresentationId::from_uuid(uuid),
                name: model.name,
                slide_count,
            });
        }
        Ok(summaries)
    }

    #[instrument(skip_all)]
    pub async fn fetch_bible_presentation(
        &self,
        id: BiblePresentationId,
    ) -> anyhow::Result<Option<BiblePresentation>> {
        let id_str = id.to_string();
        let Some(model) = bible_presentation::Entity::find_by_id(id_str.clone())
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };

        let slide_models = bible_slide::Entity::find()
            .filter(bible_slide::Column::PresentationId.eq(id_str))
            .order_by_asc(bible_slide::Column::SlideOrder)
            .all(&self.db)
            .await?;

        let mut slides = Vec::with_capacity(slide_models.len());
        for slide_model in slide_models {
            slides.push(model_to_bible_slide(slide_model)?);
        }

        Ok(Some(BiblePresentation {
            id,
            name: model.name,
            slides,
            created_at: model.created_at.into(),
        }))
    }

    #[instrument(skip_all)]
    pub async fn create_bible_presentation(&self, name: &str) -> anyhow::Result<BiblePresentation> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("bible presentation name cannot be empty"));
        }
        let id = BiblePresentationId::new();
        let now = Utc::now();
        let active = bible_presentation::ActiveModel {
            id: Set(id.to_string()),
            name: Set(trimmed.to_string()),
            created_at: Set(now.into()),
        };
        bible_presentation::Entity::insert(active)
            .exec(&self.db)
            .await?;

        Ok(BiblePresentation {
            id,
            name: trimmed.to_string(),
            slides: Vec::new(),
            created_at: now,
        })
    }

    #[instrument(skip_all)]
    pub async fn rename_bible_presentation(
        &self,
        id: BiblePresentationId,
        name: &str,
    ) -> anyhow::Result<()> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("bible presentation name cannot be empty"));
        }
        let result = bible_presentation::Entity::update_many()
            .col_expr(bible_presentation::Column::Name, SeaExpr::value(trimmed))
            .filter(bible_presentation::Column::Id.eq(id.to_string()))
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("bible presentation not found"));
        }
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn delete_bible_presentation(&self, id: BiblePresentationId) -> anyhow::Result<()> {
        let result = bible_presentation::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("bible presentation not found"));
        }
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn replace_bible_presentation_slides(
        &self,
        id: BiblePresentationId,
        slides: &[BiblePresentationSlide],
    ) -> anyhow::Result<()> {
        let id_str = id.to_string();
        let txn = self.db.begin().await?;

        if bible_presentation::Entity::find_by_id(id_str.clone())
            .one(&txn)
            .await?
            .is_none()
        {
            return Err(anyhow!("bible presentation not found"));
        }

        bible_slide::Entity::delete_many()
            .filter(bible_slide::Column::PresentationId.eq(id_str.clone()))
            .exec(&txn)
            .await?;

        for (index, slide) in slides.iter().enumerate() {
            let mut normalized = slide.clone();
            normalized.order = index as u32;
            let active = bible_slide_to_active_model(&normalized, &id_str)?;
            bible_slide::Entity::insert(active).exec(&txn).await?;
        }

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn append_bible_presentation_slides(
        &self,
        id: BiblePresentationId,
        slides: &[BiblePresentationSlide],
    ) -> anyhow::Result<BiblePresentation> {
        let id_str = id.to_string();
        let txn = self.db.begin().await?;

        if bible_presentation::Entity::find_by_id(id_str.clone())
            .one(&txn)
            .await?
            .is_none()
        {
            return Err(anyhow!("bible presentation not found"));
        }

        let existing_count = bible_slide::Entity::find()
            .filter(bible_slide::Column::PresentationId.eq(id_str.clone()))
            .count(&txn)
            .await? as u32;

        for (index, slide) in slides.iter().enumerate() {
            let mut normalized = slide.clone();
            normalized.order = existing_count + index as u32;
            let active = bible_slide_to_active_model(&normalized, &id_str)?;
            bible_slide::Entity::insert(active).exec(&txn).await?;
        }

        txn.commit().await?;

        self.fetch_bible_presentation(id)
            .await?
            .ok_or_else(|| anyhow!("bible presentation disappeared after append"))
    }
}

fn model_to_bible_slide(model: bible_slide::Model) -> anyhow::Result<BiblePresentationSlide> {
    let uuid =
        Uuid::parse_str(&model.id).map_err(|_| RepositoryError::InvalidUuid(model.id.clone()))?;
    let metadata = match model.metadata_json.as_deref() {
        Some(raw) if !raw.trim().is_empty() => {
            match serde_json::from_str::<BibleSlideMetadata>(raw) {
                Ok(meta) => Some(meta),
                Err(err) => {
                    tracing::warn!(
                        slide_id = %model.id,
                        error = %err,
                        raw = %raw,
                        "failed to deserialize bible slide metadata, returning None"
                    );
                    None
                }
            }
        }
        _ => None,
    };
    let main = SlideText::new(model.main_text).map_err(RepositoryError::from)?;
    let secondary = SlideText::new(model.secondary_text).map_err(RepositoryError::from)?;
    Ok(BiblePresentationSlide {
        id: BibleSlideId::from_uuid(uuid),
        order: model.slide_order.max(0) as u32,
        main,
        main_reference: model.main_reference,
        secondary,
        secondary_reference: model.secondary_reference,
        metadata,
    })
}

fn bible_slide_to_active_model(
    slide: &BiblePresentationSlide,
    presentation_id: &str,
) -> anyhow::Result<bible_slide::ActiveModel> {
    let metadata_json = match slide.metadata.as_ref() {
        Some(meta) => Some(serde_json::to_string(meta)?),
        None => None,
    };
    Ok(bible_slide::ActiveModel {
        id: Set(slide.id.to_string()),
        presentation_id: Set(presentation_id.to_string()),
        slide_order: Set(slide.order as i32),
        main_text: Set(slide.main.value().to_owned()),
        main_search: Set(fold_query(slide.main.value())),
        main_reference: Set(slide.main_reference.clone()),
        secondary_text: Set(slide.secondary.value().to_owned()),
        secondary_search: Set(fold_query(slide.secondary.value())),
        secondary_reference: Set(slide.secondary_reference.clone()),
        metadata_json: Set(metadata_json),
    })
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    use crate::repository::Repository;
    use presenter_core::SlideText;

    async fn fresh_repo() -> Repository {
        Repository::connect_in_memory()
            .await
            .expect("in-memory repo")
    }

    fn sample_slide(main: &str, reference: &str) -> BiblePresentationSlide {
        BiblePresentationSlide {
            id: BibleSlideId::new(),
            order: 0,
            main: SlideText::new(main).unwrap(),
            main_reference: reference.to_string(),
            secondary: SlideText::new("").unwrap(),
            secondary_reference: String::new(),
            metadata: None,
        }
    }

    #[tokio::test]
    async fn create_and_fetch_bible_presentation() {
        let repo = fresh_repo().await;
        let created = repo.create_bible_presentation("My Sermon").await.unwrap();
        assert_eq!(created.name, "My Sermon");
        assert!(created.slides.is_empty());

        let fetched = repo
            .fetch_bible_presentation(created.id)
            .await
            .unwrap()
            .expect("should exist");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "My Sermon");
    }

    #[tokio::test]
    async fn list_bible_presentation_summaries_returns_all_with_correct_counts() {
        let repo = fresh_repo().await;
        repo.create_bible_presentation("Bravo").await.unwrap();
        let alpha = repo.create_bible_presentation("Alpha").await.unwrap();

        let slide_a = sample_slide("First", "Gen 1:1");
        let slide_b = BiblePresentationSlide {
            id: BibleSlideId::new(),
            order: 1,
            main: SlideText::new("Second").unwrap(),
            main_reference: "Gen 1:2".to_string(),
            secondary: SlideText::new("").unwrap(),
            secondary_reference: String::new(),
            metadata: None,
        };
        repo.replace_bible_presentation_slides(alpha.id, &[slide_a, slide_b])
            .await
            .unwrap();

        let list = repo.list_bible_presentation_summaries().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Alpha");
        assert_eq!(list[0].slide_count, 2);
        assert_eq!(list[1].name, "Bravo");
        assert_eq!(list[1].slide_count, 0);
    }

    #[tokio::test]
    async fn rename_bible_presentation_updates_name() {
        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Old").await.unwrap();
        repo.rename_bible_presentation(p.id, "New").await.unwrap();
        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "New");
    }

    #[tokio::test]
    async fn delete_bible_presentation_removes_it_and_cascades_slides() {
        use crate::entities::bible_slide;
        use sea_orm::{EntityTrait, PaginatorTrait, QueryFilter};

        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Doomed").await.unwrap();
        let slide = sample_slide("text", "Ref");
        repo.replace_bible_presentation_slides(p.id, &[slide])
            .await
            .unwrap();

        let count_before = bible_slide::Entity::find()
            .filter(bible_slide::Column::PresentationId.eq(p.id.to_string()))
            .count(&repo.db)
            .await
            .unwrap();
        assert_eq!(count_before, 1);

        repo.delete_bible_presentation(p.id).await.unwrap();

        assert!(repo.fetch_bible_presentation(p.id).await.unwrap().is_none());

        let count_after = bible_slide::Entity::find()
            .filter(bible_slide::Column::PresentationId.eq(p.id.to_string()))
            .count(&repo.db)
            .await
            .unwrap();
        assert_eq!(count_after, 0, "FK cascade should have removed the slide");
    }

    #[tokio::test]
    async fn replace_bible_slides_overwrites_existing() {
        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Test").await.unwrap();
        let slide = sample_slide("For God so loved the world", "John 3:16");
        repo.replace_bible_presentation_slides(p.id, &[slide])
            .await
            .unwrap();
        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert_eq!(fetched.slides.len(), 1);
        assert_eq!(fetched.slides[0].main_reference, "John 3:16");

        repo.replace_bible_presentation_slides(p.id, &[])
            .await
            .unwrap();
        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert!(fetched.slides.is_empty());
    }

    #[tokio::test]
    async fn bible_slide_metadata_round_trips_through_replace_and_fetch() {
        use presenter_core::slide::{BibleSlideMetadata, BibleSlideVerseRef};

        let repo = fresh_repo().await;
        let p = repo
            .create_bible_presentation("Metadata Test")
            .await
            .unwrap();

        let metadata = BibleSlideMetadata {
            translation_code: "en-kjv".to_string(),
            secondary_translation_code: Some("sk-sevp".to_string()),
            book: "John".to_string(),
            book_code: Some("JHN".to_string()),
            book_number: Some(43),
            chapter: 3,
            verses: vec![
                BibleSlideVerseRef::new(16, 16),
                BibleSlideVerseRef::new(17, 17),
            ],
            main_reference_label: Some("John 3:16-17".to_string()),
            translation_reference_label: Some("Ján 3:16-17".to_string()),
        };

        let slide = BiblePresentationSlide {
            id: BibleSlideId::new(),
            order: 0,
            main: SlideText::new("For God so loved the world").unwrap(),
            main_reference: "John 3:16-17".to_string(),
            secondary: SlideText::new("Lebo tak Boh miloval svet").unwrap(),
            secondary_reference: "Ján 3:16-17".to_string(),
            metadata: Some(metadata.clone()),
        };

        repo.replace_bible_presentation_slides(p.id, std::slice::from_ref(&slide))
            .await
            .unwrap();

        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert_eq!(fetched.slides.len(), 1);
        let fetched_slide = &fetched.slides[0];
        assert_eq!(fetched_slide.metadata, Some(metadata));
        assert_eq!(fetched_slide.main.value(), "For God so loved the world");
        assert_eq!(fetched_slide.main_reference, "John 3:16-17");
        assert_eq!(fetched_slide.secondary_reference, "Ján 3:16-17");
    }

    #[tokio::test]
    async fn append_bible_slides_preserves_order() {
        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Test").await.unwrap();
        let slide_a = sample_slide("First", "Gen 1:1");
        let slide_b = sample_slide("Second", "Gen 1:2");
        repo.append_bible_presentation_slides(p.id, &[slide_a])
            .await
            .unwrap();
        let result = repo
            .append_bible_presentation_slides(p.id, &[slide_b])
            .await
            .unwrap();
        assert_eq!(result.slides.len(), 2);
        assert_eq!(result.slides[0].order, 0);
        assert_eq!(result.slides[1].order, 1);
        assert_eq!(result.slides[0].main_reference, "Gen 1:1");
        assert_eq!(result.slides[1].main_reference, "Gen 1:2");
    }
}

#[cfg(test)]
mod digest_tests {
    use crate::repository::Repository;
    use presenter_core::bible::BibleIngestionBatch;
    use presenter_core::BibleTranslation;

    async fn fresh_repo() -> Repository {
        Repository::connect_in_memory()
            .await
            .expect("in-memory repo")
    }

    async fn seed_translation(repo: &Repository, code: &str) {
        let translation = BibleTranslation::new(code, code, "xx").with_source("test");
        let batch =
            BibleIngestionBatch::new(translation, Vec::new()).expect("empty batch is valid");
        repo.replace_bible_translation_passages(&batch)
            .await
            .expect("seed translation");
    }

    #[tokio::test]
    async fn set_and_get_source_digest_round_trip() {
        let repo = fresh_repo().await;
        seed_translation(&repo, "eng-test").await;

        assert_eq!(
            repo.get_bible_source_digest("eng-test").await.unwrap(),
            None,
            "fresh translation has no digest",
        );

        repo.set_bible_source_digest("eng-test", "abc123")
            .await
            .expect("set digest");

        assert_eq!(
            repo.get_bible_source_digest("eng-test").await.unwrap(),
            Some("abc123".to_string()),
        );
    }

    #[tokio::test]
    async fn get_digest_for_unknown_code_is_none() {
        let repo = fresh_repo().await;
        assert_eq!(
            repo.get_bible_source_digest("nope-none").await.unwrap(),
            None,
        );
    }
}

#[cfg(test)]
mod fast_import_tests {
    use crate::repository::Repository;
    use presenter_core::bible::BibleIngestionBatch;
    use presenter_core::{BiblePassage, BibleReference, BibleTranslation};

    fn make_passage(
        translation: &BibleTranslation,
        book: &str,
        chapter: u16,
        verse: u16,
        content: &str,
    ) -> BiblePassage {
        let reference =
            BibleReference::new(book.to_string(), chapter, verse, verse).expect("valid reference");
        BiblePassage::new(reference, translation.clone(), content.to_string())
    }

    async fn fresh_repo() -> Repository {
        Repository::connect_in_memory()
            .await
            .expect("in-memory repo")
    }

    #[tokio::test]
    async fn fast_import_preserves_fts_search() {
        let repo = fresh_repo().await;
        let translation = BibleTranslation::new("eng-fast", "Fast Test", "en").with_source("test");
        let passages = vec![
            make_passage(&translation, "John", 3, 16, "For God so loved the world"),
            make_passage(
                &translation,
                "Genesis",
                1,
                1,
                "In the beginning God created",
            ),
        ];
        let batch = BibleIngestionBatch::new(translation, passages).expect("valid batch");

        repo.replace_bible_translation_passages(&batch)
            .await
            .expect("import");

        let hits = repo
            .search_bible_passages("eng-fast", "beginning", 10)
            .await
            .expect("search");
        assert!(
            hits.iter().any(|p| p.text.contains("In the beginning")),
            "FTS search should find 'beginning' after fast import, got {:?}",
            hits,
        );
    }

    #[tokio::test]
    async fn fast_import_idempotent_row_counts() {
        let repo = fresh_repo().await;
        let translation = BibleTranslation::new("eng-idem", "Idem Test", "en").with_source("test");
        let passages = vec![
            make_passage(&translation, "John", 1, 1, "In the beginning was the Word"),
            make_passage(
                &translation,
                "John",
                1,
                2,
                "He was with God in the beginning",
            ),
        ];
        let batch = BibleIngestionBatch::new(translation, passages).expect("valid batch");

        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();
        let first_hits = repo
            .search_bible_passages("eng-idem", "beginning", 10)
            .await
            .unwrap();

        // Second import with identical data
        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();
        let second_hits = repo
            .search_bible_passages("eng-idem", "beginning", 10)
            .await
            .unwrap();

        assert_eq!(
            first_hits.len(),
            second_hits.len(),
            "re-import should produce identical hit count (not accumulating FTS rows)",
        );
        assert_eq!(first_hits.len(), 2);
    }

    #[tokio::test]
    async fn fast_import_preserves_existing_digest() {
        let repo = fresh_repo().await;
        let translation = BibleTranslation::new("eng-dig", "Dig", "en")
            .with_source("test");
        let batch = BibleIngestionBatch::new(translation.clone(), Vec::new())
            .expect("valid batch");

        // Initial import — no digest yet
        repo.replace_bible_translation_passages(&batch).await.unwrap();
        assert_eq!(
            repo.get_bible_source_digest("eng-dig").await.unwrap(),
            None,
            "fresh translation has no digest",
        );

        // Stamp a digest
        repo.set_bible_source_digest("eng-dig", "keepme")
            .await
            .unwrap();

        // Re-import — digest must survive
        repo.replace_bible_translation_passages(&batch).await.unwrap();
        assert_eq!(
            repo.get_bible_source_digest("eng-dig").await.unwrap(),
            Some("keepme".to_string()),
            "re-import must NOT nuke an existing digest",
        );
    }
}
