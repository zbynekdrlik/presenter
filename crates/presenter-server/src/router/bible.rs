use axum::{extract::{Query, State}, http::StatusCode, response::Html, Json};
use crate::state::AppState;
use super::{AppError, BibleImportSummaryDto, BiblePassageQuery, BibleSearchQuery, BibleTriggerRequest};

// Lightweight pass-through wrappers to enable feature-focused routing without
// moving implementations yet. This keeps behaviour identical while we land the
// folder structure in small, low-risk steps.

pub(super) async fn list_bible_translations(
    State(state): State<AppState>,
) -> Result<Json<Vec<presenter_core::BibleTranslation>>, AppError> {
    super::list_bible_translations(State(state)).await
}

pub(super) async fn search_bible_passages(
    State(state): State<AppState>,
    Query(params): Query<BibleSearchQuery>,
) -> Result<Json<Vec<presenter_core::BiblePassage>>, super::AppError> {
    super::search_bible_passages(State(state), Query(params)).await
}

pub(super) async fn get_bible_passage(
    State(state): State<AppState>,
    Query(query): Query<BiblePassageQuery>,
) -> Result<Json<Option<presenter_core::BiblePassage>>, super::AppError> {
    super::get_bible_passage(State(state), Query(query)).await
}

pub(super) async fn refresh_bible_translations(
    State(state): State<AppState>,
) -> Result<Json<Vec<BibleImportSummaryDto>>, AppError> {
    super::refresh_bible_translations(State(state)).await
}

pub(super) async fn get_active_bible_broadcast(
    State(state): State<AppState>,
) -> Result<Json<Option<presenter_core::BibleBroadcast>>, AppError> {
    super::get_active_bible_broadcast(State(state)).await
}

pub(super) async fn trigger_bible_broadcast(
    State(state): State<AppState>,
    Json(payload): Json<BibleTriggerRequest>,
) -> Result<Json<presenter_core::BibleBroadcast>, super::AppError> {
    super::trigger_bible_broadcast(State(state), Json(payload)).await
}

pub(super) async fn clear_bible_broadcast(
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    super::clear_bible_broadcast(State(state)).await
}

pub(super) async fn bible_ui(
    State(state): State<AppState>,
) -> Result<Html<String>, AppError> {
    super::bible_ui(State(state)).await
}
