//! Bible broadcast/trigger endpoints: the active-broadcast readback, the
//! reference-based and single-source-of-truth triggers, clear, and
//! preferences get/set. Split out of `router/bible.rs` (#590) — same
//! pattern as `router/integrations/`.

use super::super::AppError;
use crate::state::AppState;
use anyhow::Context;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use presenter_core::{
    BiblePreferences, BiblePreferencesDraft, BiblePresentationId, BibleReference, BibleSlideId,
    BibleSlideOutput,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// Maps a repository refusal to its HTTP status via the TYPED
/// `RepositoryError` variant returned by the persistence layer — never a
/// string match on the `Display` text (#586, mirrors `router/libraries.rs`'s
/// `map_repository_not_found`, #584). Any other error falls through to the
/// default 500 mapping.
///
/// #608: extracted from an inline `map_err` closure to match the named-helper
/// pattern the other six modules touched by #607 already use.
fn map_repository_not_found(err: anyhow::Error) -> AppError {
    match err.downcast_ref::<presenter_persistence::RepositoryError>() {
        Some(presenter_persistence::RepositoryError::NotFound(msg)) => AppError::not_found(*msg),
        _ => err.into(),
    }
}

#[instrument(skip_all)]
pub(crate) async fn get_active_bible_broadcast(
    State(state): State<AppState>,
) -> Result<Json<Option<presenter_core::BibleBroadcast>>, AppError> {
    let active = state.active_bible_broadcast().await;
    Ok(Json(active))
}

/// Get the active Bible slide output (single-source-of-truth format).
/// Used by the stage page to load the current Bible display on connect.
#[instrument(skip_all)]
pub(crate) async fn get_active_bible_slide_output(
    State(state): State<AppState>,
) -> Result<Json<Option<presenter_core::BibleSlideOutput>>, AppError> {
    let output = state.active_bible_slide_output().await;
    Ok(Json(output))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleTriggerRequest {
    pub(crate) translation: String,
    pub(crate) book: String,
    #[serde(default)]
    pub(crate) book_code: Option<String>,
    #[serde(default)]
    pub(crate) book_number: Option<u16>,
    pub(crate) chapter: u16,
    pub(crate) verse_start: u16,
    #[serde(default)]
    pub(crate) verse_end: Option<u16>,
    #[serde(default)]
    pub(crate) main_text: Option<String>,
    #[serde(default)]
    pub(crate) translation_text: Option<String>,
    #[serde(default)]
    pub(crate) main_reference_label: Option<String>,
    #[serde(default)]
    pub(crate) translation_reference_label: Option<String>,
}

#[instrument(skip_all)]
pub(crate) async fn trigger_bible_broadcast(
    State(state): State<AppState>,
    Json(payload): Json<BibleTriggerRequest>,
) -> Result<Json<presenter_core::BibleBroadcast>, AppError> {
    let verse_end = payload.verse_end.unwrap_or(payload.verse_start);
    let reference = match (payload.book_code, payload.book_number) {
        (Some(code), Some(number)) => BibleReference::new_with_code(
            payload.book,
            code,
            number,
            payload.chapter,
            payload.verse_start,
            verse_end,
        )
        .context("failed to parse Bible reference")?,
        _ => BibleReference::new(
            payload.book,
            payload.chapter,
            payload.verse_start,
            verse_end,
        )
        .context("failed to parse Bible reference")?,
    };
    let text_overrides = crate::state::bible::BibleTriggerOverrides {
        main_text: payload.main_text,
        translation_text: payload.translation_text,
        main_reference_label: payload.main_reference_label,
        translation_reference_label: payload.translation_reference_label,
    };
    state
        .trigger_bible_passage(&payload.translation, &reference, text_overrides)
        .await
        .map(Json)
        // #587: typed refusal (#584 pattern) — downcast to `RepositoryError`
        // instead of matching the `Display` string, which silently stops
        // matching the moment a `.context(...)` is added upstream.
        .map_err(map_repository_not_found)
}

/// Request body for the new single-source-of-truth trigger endpoint.
/// What you send is EXACTLY what goes to all outputs.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleTriggerSlideRequest {
    /// Main verse text (displayed on main output)
    pub main_text: String,
    /// Main reference label (e.g., "John 3:16 (NIV)")
    pub main_reference: String,
    /// Secondary verse text (may be empty)
    #[serde(default)]
    pub secondary_text: String,
    /// Secondary reference label (e.g., "John 3:16 (ESV)")
    #[serde(default)]
    pub secondary_reference: String,
    // Optional structured reference data for backwards compatibility with /bible/active
    #[serde(default)]
    pub translation_code: Option<String>,
    #[serde(default)]
    pub book: Option<String>,
    #[serde(default)]
    pub book_code: Option<String>,
    #[serde(default)]
    pub book_number: Option<u16>,
    #[serde(default)]
    pub chapter: Option<u16>,
    #[serde(default)]
    pub verse_start: Option<u16>,
    #[serde(default)]
    pub verse_end: Option<u16>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleTriggerSlideResponse {
    success: bool,
    output: BibleSlideOutput,
}

/// Trigger a Bible slide using the single-source-of-truth approach.
/// What you send is EXACTLY what goes to all outputs - no database lookup.
#[instrument(skip_all)]
pub(crate) async fn trigger_bible_slide(
    State(state): State<AppState>,
    Json(payload): Json<BibleTriggerSlideRequest>,
) -> Result<Json<BibleTriggerSlideResponse>, AppError> {
    use crate::state::bible::BibleSlideReferenceMetadata;

    let output = BibleSlideOutput::new(
        payload.main_text,
        payload.main_reference,
        payload.secondary_text,
        payload.secondary_reference,
        Utc::now(),
    );
    let reference_metadata = BibleSlideReferenceMetadata {
        translation_code: payload.translation_code,
        book: payload.book,
        book_code: payload.book_code,
        book_number: payload.book_number,
        chapter: payload.chapter,
        verse_start: payload.verse_start,
        verse_end: payload.verse_end,
    };
    state
        .trigger_bible_slide_output(output.clone(), reference_metadata)
        .await;
    Ok(Json(BibleTriggerSlideResponse {
        success: true,
        output,
    }))
}

#[instrument(skip_all)]
pub(crate) async fn trigger_presentation_slide(
    State(state): State<AppState>,
    axum::extract::Path((presentation_id, slide_id)): axum::extract::Path<(String, String)>,
) -> Result<Json<BibleTriggerSlideResponse>, AppError> {
    use crate::state::bible::BibleSlideReferenceMetadata;

    let pres_uuid = presentation_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AppError::bad_request_message("Invalid presentation ID"))?;
    let presentation = state
        .bible_presentation_detail(BiblePresentationId::from_uuid(pres_uuid))
        .await?
        .ok_or_else(|| AppError::not_found("Presentation not found"))?;

    let slide_uuid = slide_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AppError::bad_request_message("Invalid slide ID"))?;
    let slide = presentation
        .slides
        .iter()
        .find(|s| s.id == BibleSlideId::from_uuid(slide_uuid))
        .ok_or_else(|| AppError::not_found("Slide not found in presentation"))?;

    let output = BibleSlideOutput::new(
        slide.main.value().to_string(),
        slide.main_reference.clone(),
        slide.secondary.value().to_string(),
        slide.secondary_reference.clone(),
        Utc::now(),
    );

    let meta = slide.metadata.as_ref();
    let (verse_start, verse_end) = meta
        .and_then(|m| m.verse_span())
        .map(|(s, e)| (Some(s), Some(e)))
        .unwrap_or((None, None));
    let reference_metadata = BibleSlideReferenceMetadata {
        translation_code: meta.map(|m| m.translation_code.clone()),
        book: meta.map(|m| m.book.clone()),
        book_code: meta.and_then(|m| m.book_code.clone()),
        book_number: meta.and_then(|m| m.book_number),
        chapter: meta.map(|m| m.chapter),
        verse_start,
        verse_end,
    };

    state
        .trigger_bible_slide_output(output.clone(), reference_metadata)
        .await;
    Ok(Json(BibleTriggerSlideResponse {
        success: true,
        output,
    }))
}

#[instrument(skip_all)]
pub(crate) async fn clear_bible_broadcast(
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    state.clear_bible_broadcast().await;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(crate) async fn get_bible_preferences(
    State(state): State<AppState>,
) -> Result<Json<BiblePreferences>, AppError> {
    let prefs = state.get_bible_preferences().await?;
    Ok(Json(prefs))
}

#[instrument(skip_all)]
pub(crate) async fn update_bible_preferences(
    State(state): State<AppState>,
    Json(draft): Json<BiblePreferencesDraft>,
) -> Result<StatusCode, AppError> {
    let current = state.get_bible_preferences().await?;
    let updated = draft.apply(current);
    state.set_bible_preferences(updated).await?;
    Ok(StatusCode::NO_CONTENT)
}
