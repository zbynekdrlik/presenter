use super::AppError;
use crate::state::AppState;
use anyhow::Context;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use presenter_core::slide::SlideMetadata;
use presenter_core::{
    BiblePassage, BiblePreferences, BiblePreferencesDraft, BibleReference, BibleSlideOutput,
    BibleTranslation, Slide,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::instrument;

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
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateBibleTranslationRequest {
    pub(super) name: Option<String>,
    pub(super) language: Option<String>,
    pub(super) show_in_dashboard: Option<bool>,
}

#[instrument(skip_all)]
pub(super) async fn update_bible_translation(
    State(state): State<AppState>,
    axum::extract::Path(code): axum::extract::Path<String>,
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
    #[serde(default)]
    pub(super) translation: Option<String>,
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
        .context("failed to parse Bible reference")?;
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

/// Get the active Bible slide output (single-source-of-truth format).
/// Used by the stage page to load the current Bible display on connect.
#[instrument(skip_all)]
pub(super) async fn get_active_bible_slide_output(
    State(state): State<AppState>,
) -> Result<Json<Option<presenter_core::BibleSlideOutput>>, AppError> {
    let output = state.active_bible_slide_output().await;
    Ok(Json(output))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BibleTriggerRequest {
    pub(super) translation: String,
    pub(super) book: String,
    #[serde(default)]
    pub(super) book_code: Option<String>,
    #[serde(default)]
    pub(super) book_number: Option<u16>,
    pub(super) chapter: u16,
    pub(super) verse_start: u16,
    #[serde(default)]
    pub(super) verse_end: Option<u16>,
    #[serde(default)]
    pub(super) main_text: Option<String>,
    #[serde(default)]
    pub(super) translation_text: Option<String>,
    #[serde(default)]
    pub(super) main_reference_label: Option<String>,
    #[serde(default)]
    pub(super) translation_reference_label: Option<String>,
}

#[instrument(skip_all)]
pub(super) async fn trigger_bible_broadcast(
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
    match state
        .trigger_bible_passage(&payload.translation, &reference, text_overrides)
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

/// Request body for the new single-source-of-truth trigger endpoint.
/// What you send is EXACTLY what goes to all outputs.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BibleTriggerSlideRequest {
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
pub(super) struct BibleTriggerSlideResponse {
    success: bool,
    output: BibleSlideOutput,
}

/// Trigger a Bible slide using the single-source-of-truth approach.
/// What you send is EXACTLY what goes to all outputs - no database lookup.
#[instrument(skip_all)]
pub(super) async fn trigger_bible_slide(
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
pub(super) async fn trigger_presentation_slide(
    State(state): State<AppState>,
    axum::extract::Path((presentation_id, slide_id)): axum::extract::Path<(String, String)>,
) -> Result<Json<BibleTriggerSlideResponse>, AppError> {
    use crate::state::bible::BibleSlideReferenceMetadata;

    let pres_uuid = presentation_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AppError::bad_request_message("Invalid presentation ID"))?;
    let presentation = state
        .bible_presentation_detail(presenter_core::PresentationId::from_uuid(pres_uuid))
        .await?
        .ok_or_else(|| AppError::not_found("Presentation not found"))?;

    let slide_uuid = slide_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AppError::bad_request_message("Invalid slide ID"))?;
    let slide = presentation
        .slides
        .iter()
        .find(|s| s.id == presenter_core::SlideId::from_uuid(slide_uuid))
        .ok_or_else(|| AppError::not_found("Slide not found in presentation"))?;

    let main_reference = slide
        .metadata
        .as_ref()
        .and_then(|m| m.bible.as_ref())
        .and_then(|b| b.main_reference_label.clone())
        .unwrap_or_default();

    let secondary_reference = slide
        .metadata
        .as_ref()
        .and_then(|m| m.bible.as_ref())
        .and_then(|b| b.translation_reference_label.clone())
        .unwrap_or_default();

    let bible_translation_text = slide.content.translation.value().to_string();

    let output = BibleSlideOutput::new(
        slide.content.main.value().to_string(),
        main_reference,
        bible_translation_text,
        secondary_reference,
        Utc::now(),
    );

    let meta = slide.metadata.as_ref().and_then(|m| m.bible.as_ref());
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
pub(super) async fn clear_bible_broadcast(
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    state.clear_bible_broadcast().await;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn get_bible_preferences(
    State(state): State<AppState>,
) -> Result<Json<BiblePreferences>, AppError> {
    let prefs = state.get_bible_preferences().await?;
    Ok(Json(prefs))
}

#[instrument(skip_all)]
pub(super) async fn update_bible_preferences(
    State(state): State<AppState>,
    Json(draft): Json<BiblePreferencesDraft>,
) -> Result<StatusCode, AppError> {
    let current = state.get_bible_preferences().await?;
    let updated = draft.apply(current);
    state.set_bible_preferences(updated).await?;
    Ok(StatusCode::NO_CONTENT)
}

// bible_ui handler removed — /ui/bible now redirects to /ui/operator/bible

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BibleResolveRequest {
    pub(super) main_translation: String,
    #[serde(default)]
    pub(super) secondary_translation: Option<String>,
    pub(super) book: String,
    #[serde(default)]
    pub(super) book_code: Option<String>,
    pub(super) chapter: u16,
    pub(super) verse_start: u16,
    #[serde(default)]
    pub(super) verse_end: Option<u16>,
    #[serde(default)]
    pub(super) character_limit: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BibleResolveResponse {
    main_translation: BibleTranslation,
    #[serde(skip_serializing_if = "Option::is_none")]
    secondary_translation: Option<BibleTranslation>,
    slides: Vec<BibleSlideDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BibleSlideDto {
    id: String,
    order: u32,
    bible_main: String,
    bible_translation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<SlideMetadata>,
    bible_main_reference: String,
    bible_translation_reference: String,
}

fn bible_slide_to_dto(slide: &Slide) -> Result<BibleSlideDto, AppError> {
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

    Ok(BibleSlideDto {
        id: slide.id.to_string(),
        order: slide.order,
        bible_main: slide.content.main.value().to_string(),
        bible_translation: slide.content.translation.value().to_string(),
        metadata,
        bible_main_reference: main_reference,
        bible_translation_reference: translation_reference,
    })
}

#[instrument(skip_all)]
pub(super) async fn resolve_bible_slides(
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
    let mut slide_dtos = Vec::with_capacity(slides.len());
    for slide in slides {
        slide_dtos.push(bible_slide_to_dto(&slide)?);
    }
    Ok(Json(BibleResolveResponse {
        main_translation,
        secondary_translation,
        slides: slide_dtos,
    }))
}

// Bible Presentations
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BiblePresentationSummaryDto {
    id: String,
    name: String,
    slide_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BiblePresentationDetailDto {
    id: String,
    name: String,
    slides: Vec<BibleSlideDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BiblePresentationCreateRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RenameBiblePresentationRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppendBibleSlidesRequest {
    slides: Vec<BibleSlideInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BibleSlideInput {
    bible_main: String,
    bible_translation: String,
    #[serde(default)]
    bible_main_reference: String,
    #[serde(default)]
    bible_translation_reference: String,
    #[serde(default)]
    metadata: Option<SlideMetadata>,
}

fn request_slide_to_domain(input: BibleSlideInput) -> Result<Slide, AppError> {
    let content = presenter_core::SlideContent::new(
        presenter_core::SlideText::new(&input.bible_main)
            .map_err(|e| AppError::bad_request_message(e.to_string()))?,
        presenter_core::SlideText::new(&input.bible_translation)
            .map_err(|e| AppError::bad_request_message(e.to_string()))?,
        presenter_core::SlideText::new(&input.bible_main_reference)
            .map_err(|e| AppError::bad_request_message(e.to_string()))?,
        if input.bible_main_reference.is_empty() {
            None
        } else {
            Some(presenter_core::SlideGroup::new(&input.bible_main_reference))
        },
    );
    let mut slide = Slide::new(0, content);
    // Build metadata with reference labels
    let metadata = match input.metadata {
        Some(mut meta) => {
            if let Some(ref mut bible) = meta.bible {
                if bible.main_reference_label.is_none() && !input.bible_main_reference.is_empty() {
                    bible.main_reference_label = Some(input.bible_main_reference.clone());
                }
                if bible.translation_reference_label.is_none()
                    && !input.bible_translation_reference.is_empty()
                {
                    bible.translation_reference_label =
                        Some(input.bible_translation_reference.clone());
                }
            }
            Some(meta)
        }
        None if !input.bible_main_reference.is_empty() => Some(SlideMetadata::new().with_bible(
            presenter_core::slide::BibleSlideMetadata {
                translation_code: String::new(),
                secondary_translation_code: None,
                book: String::new(),
                book_code: None,
                book_number: None,
                chapter: 0,
                verses: Vec::new(),
                main_reference_label: Some(input.bible_main_reference.clone()),
                translation_reference_label: if input.bible_translation_reference.is_empty() {
                    None
                } else {
                    Some(input.bible_translation_reference.clone())
                },
            },
        )),
        None => None,
    };
    if let Some(meta) = metadata {
        slide = slide.with_metadata(Some(meta));
    }
    Ok(slide)
}

fn bible_presentation_to_detail(
    presentation: &presenter_core::Presentation,
) -> Result<BiblePresentationDetailDto, AppError> {
    let mut slides = Vec::with_capacity(presentation.slides.len());
    for slide in &presentation.slides {
        slides.push(bible_slide_to_dto(slide)?);
    }
    Ok(BiblePresentationDetailDto {
        id: presentation.id.to_string(),
        name: presentation.name.clone(),
        slides,
    })
}

#[instrument(skip_all)]
pub(super) async fn list_bible_presentations(
    State(state): State<AppState>,
) -> Result<Json<Vec<BiblePresentationSummaryDto>>, AppError> {
    let summaries = state.list_bible_presentations().await?;
    let mut result = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let slide_count = state
            .bible_presentation_detail(summary.id)
            .await?
            .map(|presentation| presentation.slides.len())
            .unwrap_or(0);
        result.push(BiblePresentationSummaryDto {
            id: summary.id.to_string(),
            name: summary.name,
            slide_count,
        });
    }
    Ok(Json(result))
}

#[instrument(skip_all)]
pub(super) async fn create_bible_presentation_handler(
    State(state): State<AppState>,
    Json(payload): Json<BiblePresentationCreateRequest>,
) -> Result<Json<BiblePresentationDetailDto>, AppError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request_message("name cannot be empty"));
    }
    let presentation = state.create_bible_presentation(name).await?;
    let dto = bible_presentation_to_detail(&presentation)?;
    Ok(Json(dto))
}

#[instrument(skip_all)]
pub(super) async fn get_bible_presentation(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<Json<BiblePresentationDetailDto>, AppError> {
    let presentation = state
        .bible_presentation_detail(presenter_core::PresentationId::from_uuid(id))
        .await?
        .ok_or_else(|| AppError::not_found("presentation not found"))?;
    let dto = bible_presentation_to_detail(&presentation)?;
    Ok(Json(dto))
}

#[instrument(skip_all)]
pub(super) async fn rename_bible_presentation_handler(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
    Json(payload): Json<RenameBiblePresentationRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request_message("name cannot be empty"));
    }
    state
        .rename_bible_presentation(presenter_core::PresentationId::from_uuid(id), name)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn delete_bible_presentation_handler(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    state
        .delete_presentation(presenter_core::PresentationId::from_uuid(id))
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Request body for updating a single Bible slide's text and references.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateBibleSlideRequest {
    pub(super) bible_main: String,
    pub(super) bible_translation: String,
    pub(super) bible_main_reference: String,
    pub(super) bible_translation_reference: String,
}

/// PATCH /bible/presentations/{id}/slides/{slide_id}
/// Updates a Bible slide's content (main, translation) and metadata references
/// (main_reference_label, translation_reference_label). Also writes mainReference
/// to content.stage for Resolume compatibility (transparent to UI). Does NOT touch group.
#[instrument(skip_all)]
pub(super) async fn update_bible_slide(
    State(state): State<AppState>,
    axum::extract::Path((presentation_id, slide_id)): axum::extract::Path<(String, String)>,
    Json(payload): Json<UpdateBibleSlideRequest>,
) -> Result<Json<BibleSlideDto>, AppError> {
    let pres_uuid = presentation_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AppError::bad_request_message("Invalid presentation ID"))?;
    let slide_uuid = slide_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AppError::bad_request_message("Invalid slide ID"))?;

    let pres_id = presenter_core::PresentationId::from_uuid(pres_uuid);
    let s_id = presenter_core::SlideId::from_uuid(slide_uuid);

    // Look up the existing slide to preserve its metadata structure and group
    let presentation = state
        .bible_presentation_detail(pres_id)
        .await?
        .ok_or_else(|| AppError::not_found("Presentation not found"))?;

    let existing_slide = presentation
        .slides
        .iter()
        .find(|s| s.id == s_id)
        .ok_or_else(|| AppError::not_found("Slide not found in presentation"))?;

    // Build updated metadata: preserve existing bible metadata but update reference labels
    let updated_metadata = {
        let mut meta = existing_slide
            .metadata
            .clone()
            .unwrap_or_else(SlideMetadata::new);
        if let Some(ref mut bible) = meta.bible {
            bible.main_reference_label = if payload.bible_main_reference.is_empty() {
                None
            } else {
                Some(payload.bible_main_reference.clone())
            };
            bible.translation_reference_label = if payload.bible_translation_reference.is_empty() {
                None
            } else {
                Some(payload.bible_translation_reference.clone())
            };
        }
        // If there's no bible metadata at all but references are provided, we still store them
        if meta.bible.is_none()
            && (!payload.bible_main_reference.is_empty()
                || !payload.bible_translation_reference.is_empty())
        {
            meta.bible = Some(presenter_core::slide::BibleSlideMetadata {
                translation_code: String::new(),
                secondary_translation_code: None,
                book: String::new(),
                book_code: None,
                book_number: None,
                chapter: 0,
                verses: Vec::new(),
                main_reference_label: if payload.bible_main_reference.is_empty() {
                    None
                } else {
                    Some(payload.bible_main_reference.clone())
                },
                translation_reference_label: if payload.bible_translation_reference.is_empty() {
                    None
                } else {
                    Some(payload.bible_translation_reference.clone())
                },
            });
        }
        Some(meta)
    };

    // Stage stores the main reference for Resolume compatibility
    let stage_value = payload.bible_main_reference.clone();

    // Group stores reference label for grouping
    let existing_group = if stage_value.is_empty() {
        None
    } else {
        Some(stage_value.clone())
    };

    let updated = state
        .update_slide_content(
            pres_id,
            s_id,
            payload.bible_main,
            payload.bible_translation,
            stage_value,
            existing_group,
            updated_metadata,
        )
        .await?;

    bible_slide_to_dto(&updated).map(Json)
}

#[instrument(skip_all)]
pub(super) async fn append_bible_presentation_handler(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
    Json(payload): Json<AppendBibleSlidesRequest>,
) -> Result<Json<BiblePresentationDetailDto>, AppError> {
    if payload.slides.is_empty() {
        return Err(AppError::bad_request_message("slides cannot be empty"));
    }
    let mut slides = Vec::with_capacity(payload.slides.len());
    for input in payload.slides {
        slides.push(request_slide_to_domain(input)?);
    }
    let presentation = state
        .append_bible_presentation_slides(presenter_core::PresentationId::from_uuid(id), slides)
        .await?;
    let dto = bible_presentation_to_detail(&presentation)?;
    Ok(Json(dto))
}
