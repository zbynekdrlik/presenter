//! Bible translation admin: listing, metadata updates, and the
//! import/refresh trigger. Split out of `router/bible.rs` (#590) — same
//! pattern as `router/integrations/`: a handler-group file with `pub(crate)`
//! items so `router.rs`'s route table can reference them directly.

use axum::extract::{Path, State};
use axum::Json;
use presenter_core::BibleTranslation;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleImportSummaryDto {
    pub(crate) translation_code: String,
    pub(crate) passage_count: usize,
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
pub(crate) async fn list_bible_translations(
    State(state): State<AppState>,
) -> Result<Json<Vec<BibleTranslation>>, AppError> {
    let translations = state.list_bible_translations().await?;
    Ok(Json(translations))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateBibleTranslationRequest {
    pub(crate) name: Option<String>,
    pub(crate) language: Option<String>,
    pub(crate) show_in_dashboard: Option<bool>,
}

#[instrument(skip_all)]
pub(crate) async fn update_bible_translation(
    State(state): State<AppState>,
    axum::extract::Path(code): Path<String>,
    Json(payload): Json<UpdateBibleTranslationRequest>,
) -> Result<Json<BibleTranslation>, AppError> {
    let translation = state
        .update_bible_translation(
            &code,
            payload.name.as_deref(),
            payload.language.as_deref(),
            payload.show_in_dashboard,
        )
        .await?
        .ok_or_else(|| AppError::not_found("translation not found"))?;
    Ok(Json(translation))
}

#[instrument(skip_all)]
pub(crate) async fn refresh_bible_translations(
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
