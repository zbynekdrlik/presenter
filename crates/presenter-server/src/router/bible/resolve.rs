//! Bible slide resolution: the `/bible/resolve` preview endpoint and the
//! shared `BibleSlideDto` conversions used by both the resolve preview and
//! the presentation CRUD handlers (`bible/presentations.rs`). Split out of
//! `router/bible.rs` (#590) — same pattern as `router/integrations/`.

use super::super::AppError;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use presenter_core::slide::SlideMetadata;
use presenter_core::{BiblePresentationSlide, BibleTranslation, Slide};
use serde::{Deserialize, Serialize};
use tracing::instrument;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleResolveRequest {
    pub(crate) main_translation: String,
    #[serde(default)]
    pub(crate) secondary_translation: Option<String>,
    pub(crate) book: String,
    #[serde(default)]
    pub(crate) book_code: Option<String>,
    pub(crate) chapter: u16,
    pub(crate) verse_start: u16,
    #[serde(default)]
    pub(crate) verse_end: Option<u16>,
    #[serde(default)]
    pub(crate) character_limit: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleResolveResponse {
    main_translation: BibleTranslation,
    #[serde(skip_serializing_if = "Option::is_none")]
    secondary_translation: Option<BibleTranslation>,
    slides: Vec<BibleSlideDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleSlideDto {
    id: String,
    order: u32,
    bible_main: String,
    bible_translation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<SlideMetadata>,
    bible_main_reference: String,
    bible_translation_reference: String,
}

/// Convert a stored `BiblePresentationSlide` (from the bible repository) to
/// the wire DTO. The DTO field names remain unchanged to preserve wire
/// compatibility with the existing frontend.
pub(crate) fn bible_slide_to_dto(slide: &BiblePresentationSlide) -> BibleSlideDto {
    // Wrap the structured `BibleSlideMetadata` into the legacy `SlideMetadata`
    // envelope the frontend expects: `{ metadata: { bible: { ... } } }`.
    let metadata = slide.metadata.as_ref().map(|bible_meta| {
        let mut sm = SlideMetadata::new();
        sm.bible = Some(bible_meta.clone());
        sm
    });

    BibleSlideDto {
        id: slide.id.to_string(),
        order: slide.order,
        bible_main: slide.main.value().to_string(),
        bible_translation: slide.secondary.value().to_string(),
        metadata,
        bible_main_reference: slide.main_reference.clone(),
        bible_translation_reference: slide.secondary_reference.clone(),
    }
}

/// Convert a generated worship `Slide` (from `compose_bible_slides`) to the
/// wire DTO. The composer currently stores the reference label in
/// `content.stage` and the structured metadata inside `metadata.bible`. This
/// conversion is used only by the `/bible/resolve` preview endpoint and the
/// `resolve_bible_slides` AI tool — neither persists these slides.
fn generated_slide_to_dto(slide: &Slide) -> BibleSlideDto {
    let metadata = slide.metadata.clone();
    let (main_reference, translation_reference) = metadata
        .as_ref()
        .and_then(|meta| meta.bible.as_ref())
        .map(|meta| {
            (
                meta.main_reference_label.clone().unwrap_or_default(),
                meta.translation_reference_label.clone().unwrap_or_default(),
            )
        })
        .unwrap_or_default();

    BibleSlideDto {
        id: slide.id.to_string(),
        order: slide.order,
        bible_main: slide.content.main.value().to_string(),
        bible_translation: slide.content.translation.value().to_string(),
        metadata,
        bible_main_reference: main_reference,
        bible_translation_reference: translation_reference,
    }
}

#[instrument(skip_all)]
pub(crate) async fn resolve_bible_slides(
    State(state): State<AppState>,
    Json(payload): Json<BibleResolveRequest>,
) -> Result<Json<BibleResolveResponse>, AppError> {
    if payload.main_translation.trim().is_empty() {
        return Err(AppError::bad_request_message("mainTranslation is required"));
    }
    let main_translation_code = payload.main_translation.trim();
    let book = payload.book.trim();
    let book_code = payload
        .book_code
        .as_deref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let verse_end = if let Some(end) = payload.verse_end {
        end
    } else {
        let summaries = state
            .bible_book_chapter_summaries(main_translation_code)
            .await?;
        summaries
            .into_iter()
            .find(|summary| {
                if summary.chapter != payload.chapter {
                    return false;
                }
                if let Some(code) = book_code {
                    summary
                        .book_code
                        .as_deref()
                        .map(|candidate| candidate.eq_ignore_ascii_case(code))
                        .unwrap_or(false)
                } else {
                    summary.book.eq_ignore_ascii_case(book)
                }
            })
            .map(|summary| summary.verse_count)
            .unwrap_or(payload.verse_start)
    }
    .max(payload.verse_start);
    let character_limit = if let Some(limit) = payload.character_limit {
        limit
    } else {
        let prefs = state.get_bible_preferences().await?;
        prefs.character_limit
    };
    let (main_translation, secondary_translation, slides) = state
        .generate_bible_slides(
            main_translation_code,
            payload.secondary_translation.as_deref(),
            book,
            book_code,
            payload.chapter,
            payload.verse_start,
            verse_end,
            character_limit,
        )
        .await?;
    let slide_dtos: Vec<BibleSlideDto> = slides.iter().map(generated_slide_to_dto).collect();
    Ok(Json(BibleResolveResponse {
        main_translation,
        secondary_translation,
        slides: slide_dtos,
    }))
}
