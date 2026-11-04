use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseBackend, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Statement,
};
use tracing::instrument;

use crate::entities::{bible_passage, bible_translation};
use presenter_core::bible::{BibleBookChapterSummary, BookScope};
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};

use crate::repository::util::{sanitize_like_input, to_domain_passage, to_domain_translation};
use crate::repository::Repository;

/// Build an FTS5 MATCH query from free-text input.
///
/// Splits into words, drops tokens shorter than 2 chars, appends `*` for
/// prefix matching, and joins with spaces (implicit AND in FTS5).
/// Returns `None` if no usable tokens remain. A fully quoted input is an exact
/// phrase (#851 spec §6.4).
fn build_fts_query(input: &str) -> Option<String> {
    if let Some(phrase) = quoted_phrase(input) {
        let words: Vec<&str> = phrase.split_whitespace().collect();
        return (!words.is_empty()).then(|| format!("content : \"{}\"", words.join(" ")));
    }
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

/// A phone's keyboard types `„viera“` as readily as `"viera"`.
fn is_quote(c: char) -> bool {
    matches!(c, '"' | '„' | '“' | '”')
}

/// The words of one fully quoted input, punctuation turned into spaces and
/// combining marks dropped (the tokenizer folds them): inside an FTS5 phrase a
/// `"` would end it. Two quoted parts are not one phrase.
fn quoted_phrase(input: &str) -> Option<String> {
    let inner = input
        .trim()
        .strip_prefix(is_quote)?
        .strip_suffix(is_quote)?;
    if inner.contains(is_quote) {
        return None;
    }
    Some(
        inner
            .chars()
            .filter(|c| !matches!(c, '\u{0300}'..='\u{036F}'))
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect(),
    )
}

impl Repository {
    #[instrument(skip_all)]
    pub async fn get_bible_source_digest(&self, code: &str) -> anyhow::Result<Option<String>> {
        let model = bible_translation::Entity::find_by_id(code.to_string())
            .one(&self.db)
            .await?;
        Ok(model.and_then(|m| m.source_digest))
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
        scope: BookScope,
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
            .fts_search(query, Some(translation_code), scope, limit)
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
        scope: BookScope,
        limit: u32,
    ) -> anyhow::Result<Vec<BiblePassage>> {
        if let Some(code) = translation_code {
            return self.search_bible_passages(code, query, scope, limit).await;
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

        let rows = self.fts_search(query, None, scope, limit).await?;

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
        scope: BookScope,
        limit: u32,
    ) -> anyhow::Result<Vec<bible_passage::Model>> {
        let phrase = quoted_phrase(query);
        if phrase
            .as_deref()
            .is_some_and(|words| words.trim().is_empty())
        {
            return Ok(Vec::new());
        }
        if let Some(fts_query) = build_fts_query(query) {
            let result = self
                .fts_search_raw(&fts_query, translation_code, scope, limit)
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
        let (first_book, last_book) = scope.book_numbers();
        let text = phrase.unwrap_or_else(|| query.to_string());
        let pattern = format!("%{}%", sanitize_like_input(text.trim()));
        let mut q = bible_passage::Entity::find()
            .filter(bible_passage::Column::Content.like(pattern))
            .filter(bible_passage::Column::BookNumber.between(first_book, last_book));
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
        scope: BookScope,
        limit: u32,
    ) -> anyhow::Result<Vec<bible_passage::Model>> {
        let (first_book, last_book) = scope.book_numbers();
        let mut values = vec![
            sea_orm::Value::from(fts_query.to_string()),
            sea_orm::Value::from(i32::from(first_book)),
            sea_orm::Value::from(i32::from(last_book)),
        ];
        let translation_filter = match translation_code {
            Some(code) => {
                values.push(sea_orm::Value::from(code.to_string()));
                "AND fts.translation_code = ?"
            }
            None => "",
        };
        values.push(sea_orm::Value::from(limit as i32));
        let sql = format!(
            "SELECT bp.id, bp.translation_code, bp.book, bp.book_code, \
                    bp.book_number, bp.chapter, bp.verse_start, bp.verse_end, \
                    bp.content, bp.created_at \
             FROM bible_passage_fts fts \
             JOIN bible_passages bp ON bp.id = fts.passage_id \
             WHERE bible_passage_fts MATCH ? \
               AND bp.book_number BETWEEN ? AND ? \
               {translation_filter} \
             ORDER BY fts.rank \
             LIMIT ?"
        );

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
            passages.push(crate::repository::util::to_domain_passage(
                row,
                translation.clone(),
            )?);
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
}

#[cfg(test)]
mod fts_query_tests {
    use super::build_fts_query;

    #[test]
    fn words_are_prefixes_and_a_fully_quoted_input_is_a_phrase() {
        assert_eq!(
            build_fts_query("viera nádej").as_deref(),
            Some("viera* nádej*")
        );
        assert_eq!(
            build_fts_query("\"viera, nádej-láska\"").as_deref(),
            Some("content : \"viera nádej láska\"")
        );
        assert_eq!(
            build_fts_query("„viera, nádej“").as_deref(),
            Some("content : \"viera nádej\"")
        );
        assert_eq!(
            build_fts_query("\"viera\" nádej").as_deref(),
            Some("viera* nádej*")
        );
        assert_eq!(
            build_fts_query("\"\"viera\" \"nádej\"\"").as_deref(),
            Some("viera* nádej*")
        );
        assert_eq!(
            build_fts_query("\"na\u{0301}dej\"").as_deref(),
            Some("content : \"nadej\"")
        );
        assert_eq!(build_fts_query("\" - \""), None);
        assert_eq!(build_fts_query("\""), None);
    }
}

#[cfg(test)]
mod fts_query_tests {
    use super::build_fts_query;

    #[test]
    fn a_reference_keeps_only_its_word_and_never_glues_its_numbers() {
        assert_eq!(build_fts_query("zid 11:1").as_deref(), Some("zid*"));
        assert_eq!(build_fts_query("11:1"), None);
        assert_eq!(build_fts_query("Ž 23"), None);
    }

    #[test]
    fn words_are_split_on_punctuation_and_counted_in_characters() {
        assert_eq!(
            build_fts_query("viera je zárukou").as_deref(),
            Some("viera* je* zárukou*")
        );
        assert_eq!(
            build_fts_query("\"Pán\"-je(môj)").as_deref(),
            Some("Pán* je* môj*")
        );
        assert_eq!(build_fts_query("ž a"), None);
    }
}
