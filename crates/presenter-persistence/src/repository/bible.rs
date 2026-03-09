use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    TransactionTrait,
};
use tracing::instrument;

use crate::entities::{bible_passage, bible_translation};
use presenter_core::{
    bible::{canonical_book_by_name, BibleIngestionBatch},
    BiblePassage, BibleReference, BibleTranslation,
};
use sea_orm::Set;

use super::util::{
    sanitize_like_input, to_domain_passage, to_domain_translation, BIBLE_INSERT_CHUNK,
};
use super::Repository;

impl Repository {
    #[instrument(skip_all)]
    pub async fn replace_bible_translation_passages(
        &self,
        batch: &BibleIngestionBatch,
    ) -> anyhow::Result<()> {
        let (translation, passages) = batch.clone().into_parts();
        let existing = bible_translation::Entity::find_by_id(translation.code.clone())
            .one(&self.db)
            .await?;
        let preserve_dashboard = existing
            .as_ref()
            .map(|model| model.show_in_dashboard)
            .unwrap_or(translation.show_in_dashboard);
        let mut txn = self.db.begin().await?;

        bible_translation::Entity::delete_by_id(translation.code.clone())
            .exec(&mut txn)
            .await?;

        let translation_model = bible_translation::ActiveModel {
            code: Set(translation.code.clone()),
            name: Set(translation.name.clone()),
            language: Set(translation.language.clone()),
            show_in_dashboard: Set(preserve_dashboard),
            source: Set(translation.source.clone()),
            created_at: Set(Utc::now().into()),
        };

        bible_translation::Entity::insert(translation_model)
            .exec(&mut txn)
            .await?;

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
                id: Set(uuid::Uuid::new_v4().to_string()),
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
                    .exec(&mut txn)
                    .await?;
            }
        }

        if !chunk.is_empty() {
            bible_passage::Entity::insert_many(chunk)
                .exec(&mut txn)
                .await?;
        }

        txn.commit().await?;
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

        let pattern = format!("%{}%", sanitize_like_input(query));
        let rows = bible_passage::Entity::find()
            .filter(bible_passage::Column::TranslationCode.eq(translation_code.to_string()))
            .filter(bible_passage::Column::Content.like(pattern))
            .order_by_asc(bible_passage::Column::Book)
            .order_by_asc(bible_passage::Column::Chapter)
            .order_by_asc(bible_passage::Column::VerseStart)
            .limit(limit as u64)
            .all(&self.db)
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

        let pattern = format!("%{}%", sanitize_like_input(query));
        let rows = bible_passage::Entity::find()
            .filter(bible_passage::Column::Content.like(pattern))
            .order_by_asc(bible_passage::Column::BookNumber)
            .order_by_asc(bible_passage::Column::Chapter)
            .order_by_asc(bible_passage::Column::VerseStart)
            .order_by_asc(bible_passage::Column::TranslationCode)
            .limit(limit as u64)
            .all(&self.db)
            .await?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let translation_code = row.translation_code.clone();
            if let Some(translation) = all_translations.get(&translation_code) {
                results.push(to_domain_passage(row, translation.clone())?);
            }
        }
        Ok(results)
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
use sea_orm::FromQueryResult;

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
}
