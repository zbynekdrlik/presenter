use axum::{extract::{Query, State}, Json};
use serde::Deserialize;
use tracing::instrument;

use crate::state::AppState;
use super::{AppError};
use presenter_core::SearchResult;

#[derive(Debug, Deserialize)]
pub(super) struct SearchQueryParams {
    #[serde(default, alias = "q", alias = "query")]
    query: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[instrument(skip_all)]
pub(super) async fn search_presenter_endpoint(
    State(state): State<AppState>,
    Query(params): Query<SearchQueryParams>,
) -> Result<Json<Vec<SearchResult>>, AppError> {
    let query = params.query.unwrap_or_default();
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Json(Vec::new()))
    }
    let limit = params.limit.unwrap_or(25).clamp(1, 100) as u64;
    let results = state.search_presenter(trimmed, limit).await?;
    Ok(Json(results))
}
