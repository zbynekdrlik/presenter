//! Bible browsing: book/chapter listing, cross-translation search, and
//! single-passage lookup. Split out of `router/bible.rs` (#590) — same
//! pattern as `router/integrations/`.

use anyhow::Context;
use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;
use presenter_core::{BiblePassage, BibleReference};

#[derive(Debug, Deserialize)]
pub(crate) struct BibleBooksQuery {
    pub(crate) translation: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleBookDto {
    pub(crate) book: String,
    pub(crate) code: String,
    pub(crate) number: u16,
    pub(crate) chapters: Vec<BibleChapterDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleChapterDto {
    pub(crate) number: u16,
    pub(crate) verse_count: u16,
}

#[instrument(skip_all)]
pub(crate) async fn list_bible_books(
    State(state): State<AppState>,
    Query(params): Query<BibleBooksQuery>,
) -> Result<Json<Vec<BibleBookDto>>, AppError> {
    let summaries = state.list_bible_books(&params.translation).await?;

    // Aggregate flat chapter summaries into books with chapter arrays
    let mut books_map: HashMap<String, BibleBookDto> = HashMap::new();
    let mut book_order: Vec<String> = Vec::new();

    for summary in summaries {
        let key = summary
            .book_code
            .clone()
            .unwrap_or_else(|| summary.book.clone());
        if !books_map.contains_key(&key) {
            book_order.push(key.clone());
            books_map.insert(
                key.clone(),
                BibleBookDto {
                    book: summary.book.clone(),
                    code: summary.book_code.clone().unwrap_or_default(),
                    number: summary.book_number.unwrap_or(0),
                    chapters: Vec::new(),
                },
            );
        }
        if let Some(book) = books_map.get_mut(&key) {
            book.chapters.push(BibleChapterDto {
                number: summary.chapter,
                verse_count: summary.verse_count,
            });
        }
    }

    // Preserve book order from the database query
    let books: Vec<BibleBookDto> = book_order
        .into_iter()
        .filter_map(|key| books_map.remove(&key))
        .collect();

    Ok(Json(books))
}

#[derive(Debug, Deserialize)]
pub(crate) struct BibleSearchQuery {
    #[serde(default)]
    pub(crate) translation: Option<String>,
    pub(crate) query: String,
    #[serde(default)]
    pub(crate) limit: Option<u32>,
}

#[instrument(skip_all)]
pub(crate) async fn search_bible_passages(
    State(state): State<AppState>,
    Query(params): Query<BibleSearchQuery>,
) -> Result<Json<Vec<BiblePassage>>, AppError> {
    let trimmed = params.query.trim();
    if trimmed.len() < 2 {
        return Err(AppError::bad_request_message(
            "query must be at least 2 characters",
        ));
    }
    let limit = params.limit.unwrap_or(25).min(100);
    let translation_code = params
        .translation
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let passages = state
        .search_bible_passages_cross(translation_code, trimmed, limit)
        .await?;
    Ok(Json(passages))
}

#[derive(Debug, Deserialize)]
pub(crate) struct BiblePassageQuery {
    pub(crate) translation: String,
    pub(crate) book: String,
    pub(crate) chapter: u16,
    pub(crate) verse_start: u16,
    #[serde(default)]
    pub(crate) verse_end: Option<u16>,
}

#[instrument(skip_all)]
pub(crate) async fn get_bible_passage(
    State(state): State<AppState>,
    Query(query): Query<BiblePassageQuery>,
) -> Result<Json<Option<BiblePassage>>, AppError> {
    let verse_end = query.verse_end.unwrap_or(query.verse_start);
    let reference = BibleReference::new(query.book, query.chapter, query.verse_start, verse_end)
        .context("failed to parse Bible reference")?;
    let passage = state
        .find_bible_passage(&query.translation, &reference)
        .await?;
    Ok(Json(passage))
}
