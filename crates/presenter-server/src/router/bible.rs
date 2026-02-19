use super::AppError;
use crate::state::AppState;
use anyhow::Error as AnyhowError;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
    Json,
};
use presenter_core::slide::SlideMetadata;
use presenter_core::{
    BiblePassage, BiblePreferences, BiblePreferencesDraft, BibleReference, BibleTranslation, Slide,
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
    #[serde(default)]
    pub(super) book_code: Option<String>,
    #[serde(default)]
    pub(super) book_number: Option<u16>,
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
    let reference = match (payload.book_code, payload.book_number) {
        (Some(code), Some(number)) => BibleReference::new_with_code(
            payload.book,
            code,
            number,
            payload.chapter,
            payload.verse_start,
            verse_end,
        )
        .map_err(AnyhowError::new)?,
        _ => BibleReference::new(
            payload.book,
            payload.chapter,
            payload.verse_start,
            verse_end,
        )
        .map_err(AnyhowError::new)?,
    };
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

#[instrument(skip_all)]
pub(super) async fn bible_ui(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let html = crate::ui::render_bible_ui(&state).await?;
    Ok(html)
}

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
    main: String,
    translation: String,
    stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<SlideMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    main_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    translation_reference: Option<String>,
}

fn bible_slide_to_dto(slide: &Slide) -> Result<BibleSlideDto, AppError> {
    let metadata = slide.metadata.clone();
    let (main_reference, translation_reference) = metadata
        .as_ref()
        .and_then(|meta| meta.bible.as_ref())
        .map(|meta| {
            (
                meta.main_reference_label.clone(),
                meta.translation_reference_label.clone(),
            )
        })
        .unwrap_or((None, None));

    Ok(BibleSlideDto {
        id: slide.id.to_string(),
        order: slide.order,
        main: slide.content.main.value().to_string(),
        translation: slide.content.translation.value().to_string(),
        stage: slide.content.stage.value().to_string(),
        group: slide
            .content
            .group
            .as_ref()
            .map(|group| group.name().to_string()),
        metadata,
        main_reference,
        translation_reference,
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
    let character_limit = payload.character_limit.unwrap_or(320);
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
    main: String,
    translation: String,
    stage: String,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    metadata: Option<SlideMetadata>,
}

fn request_slide_to_domain(input: BibleSlideInput) -> Result<Slide, AppError> {
    let content = presenter_core::SlideContent::new(
        presenter_core::SlideText::new(&input.main)
            .map_err(|e| AppError::bad_request_message(e.to_string()))?,
        presenter_core::SlideText::new(&input.translation)
            .map_err(|e| AppError::bad_request_message(e.to_string()))?,
        presenter_core::SlideText::new(&input.stage)
            .map_err(|e| AppError::bad_request_message(e.to_string()))?,
        input.group.map(presenter_core::SlideGroup::new),
    );
    let mut slide = Slide::new(0, content);
    if let Some(meta) = input.metadata {
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
