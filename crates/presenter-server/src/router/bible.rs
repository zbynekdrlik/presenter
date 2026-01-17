use super::AppError;
use crate::state::AppState;
use anyhow::Error as AnyhowError;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
    Json,
};
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::instrument;

#[derive(Debug, Deserialize)]
pub(super) struct BibleUiQuery {
    #[serde(default)]
    pub embed: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct BibleImportSummaryDto {
    pub(super) translation_code: String,
    pub(super) passage_count: usize,
}

impl From<presenter_bible::BibleImportSummary> for BibleImportSummaryDto {
    fn from(summary: presenter_bible::BibleImportSummary) -> Self {
        Self {
            translation_code: summary.translation_code,
            passage_count: summary.passage_count,
        }
    }
}

#[instrument(skip_all)]
pub(super) async fn list_bible_translations(
    State(state): State<AppState>,
) -> Result<Json<Vec<BibleTranslation>>, AppError> {
    let translations = state.list_bible_translations().await?;
    Ok(Json(translations))
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct BibleBooksQuery {
    pub(super) translation: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BibleBookDto {
    pub(super) book: String,
    pub(super) code: String,
    pub(super) number: u16,
    pub(super) chapters: Vec<BibleChapterDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BibleChapterDto {
    pub(super) number: u16,
    pub(super) verse_count: u16,
}

#[instrument(skip_all)]
pub(super) async fn list_bible_books(
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

#[derive(Debug, serde::Deserialize)]
pub(super) struct BibleSearchQuery {
    pub(super) translation: String,
    pub(super) query: String,
    #[serde(default)]
    pub(super) limit: Option<u32>,
}

#[instrument(skip_all)]
pub(super) async fn search_bible_passages(
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
    let passages = state
        .search_bible_passages(&params.translation, trimmed, limit)
        .await?;
    Ok(Json(passages))
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct BiblePassageQuery {
    pub(super) translation: String,
    pub(super) book: String,
    pub(super) chapter: u16,
    pub(super) verse_start: u16,
    #[serde(default)]
    pub(super) verse_end: Option<u16>,
}

#[instrument(skip_all)]
pub(super) async fn get_bible_passage(
    State(state): State<AppState>,
    Query(query): Query<BiblePassageQuery>,
) -> Result<Json<Option<BiblePassage>>, AppError> {
    let verse_end = query.verse_end.unwrap_or(query.verse_start);
    let reference = BibleReference::new(query.book, query.chapter, query.verse_start, verse_end)
        .map_err(AnyhowError::new)?;
    let passage = state
        .find_bible_passage(&query.translation, &reference)
        .await?;
    Ok(Json(passage))
}

#[instrument(skip_all)]
pub(super) async fn refresh_bible_translations(
    State(state): State<AppState>,
) -> Result<Json<Vec<BibleImportSummaryDto>>, AppError> {
    let summaries = state.refresh_default_bible_translations().await?;
    Ok(Json(
        summaries
            .into_iter()
            .map(BibleImportSummaryDto::from)
            .collect(),
    ))
}

#[instrument(skip_all)]
pub(super) async fn get_active_bible_broadcast(
    State(state): State<AppState>,
) -> Result<Json<Option<presenter_core::BibleBroadcast>>, AppError> {
    let active = state.active_bible_broadcast().await;
    Ok(Json(active))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BibleTriggerRequest {
    pub(super) translation: String,
    pub(super) book: String,
    pub(super) chapter: u16,
    pub(super) verse_start: u16,
    #[serde(default)]
    pub(super) verse_end: Option<u16>,
}

#[instrument(skip_all)]
pub(super) async fn trigger_bible_broadcast(
    State(state): State<AppState>,
    Json(payload): Json<BibleTriggerRequest>,
) -> Result<Json<presenter_core::BibleBroadcast>, AppError> {
    let verse_end = payload.verse_end.unwrap_or(payload.verse_start);
    let reference = BibleReference::new(
        payload.book,
        payload.chapter,
        payload.verse_start,
        verse_end,
    )
    .map_err(AnyhowError::new)?;
    match state
        .trigger_bible_passage(&payload.translation, &reference)
        .await
    {
        Ok(broadcast) => Ok(Json(broadcast)),
        Err(err) => {
            if err.to_string().contains("passage not found") {
                return Err(AppError::not_found("passage not found"));
            }
            Err(err.into())
        }
    }
}

#[instrument(skip_all)]
pub(super) async fn clear_bible_broadcast(
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    state.clear_bible_broadcast().await;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn bible_ui(
    State(state): State<AppState>,
    Query(query): Query<BibleUiQuery>,
) -> Result<Html<String>, AppError> {
    let embed = match query.embed.as_deref() {
        Some(value) => matches!(value, "1" | "true" | "yes" | "on"),
        None => false,
    };
    let html = crate::ui::render_bible_ui(&state, embed).await?;
    Ok(html)
}
