//! The sermon API (#851 spec §11) and `GET /bible/study/identity` (§9). JSON is
//! camelCase, query parameters snake_case, like the other Bible routes. The
//! reads come first (P4a); the writes carry `expectedRevision` and answer the
//! full sermon. `sermon_error` is the ONE place a sermon handler turns an error
//! into a response.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Extension, Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use presenter_core::bible::sermon::{ItemEdits, SourceVerse};
use presenter_core::BiblePresentationId;
use serde::Serialize;
use tracing::instrument;
use uuid::Uuid;

use super::super::access_identity::access_email;
use super::super::AppError;
use crate::state::sermons::{
    local_today, SermonDetail, SermonItemView, SermonList, SermonSlideView,
};
use crate::state::AppState;

type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;
type SermonResult = Result<Json<SermonDetailDto>, AppError>;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/bible/sermons", get(list_sermons))
        .route("/bible/sermons/today", get(todays_sermons))
        .route("/bible/sermons/{id}", get(get_sermon))
        .route("/bible/study/identity", get(identity))
}

// ---- DTOs -----------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SermonListDto {
    today: NaiveDate,
    default_service_date: NaiveDate,
    sermons: Vec<SermonSummaryDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SermonSummaryDto {
    id: String,
    title: String,
    service_date: NaiveDate,
    author_email: Option<String>,
    item_count: usize,
    slide_count: usize,
    updated_at: DateTime<Utc>,
    revision: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SermonDetailDto {
    id: String,
    title: String,
    service_date: NaiveDate,
    author_email: Option<String>,
    default_translation: String,
    secondary_translation: Option<String>,
    revision: i64,
    items: Vec<SermonItemDto>,
    slides: Vec<SermonSlideDto>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SermonItemDto {
    id: String,
    order: usize,
    translation: String,
    book_code: String,
    book_number: u16,
    book: String,
    chapter: u16,
    verse_start: u16,
    verse_end: u16,
    source: Vec<SourceVerse>,
    edits: ItemEdits,
    label: String,
    rendered_text: String,
    source_changed: bool,
    slide_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SermonSlideDto {
    id: String,
    item_id: String,
    main_text: String,
    main_reference: String,
    secondary_text: String,
    secondary_reference: String,
}

#[derive(Debug, Serialize)]
struct IdentityDto {
    email: Option<String>,
}

// ---- Mapping ----------------------------------------------------------------

/// A failed sermon request (spec §12, logged at WARN with its cause), mapped
/// through the central `From<anyhow::Error> for AppError`.
fn sermon_error(err: anyhow::Error) -> AppError {
    tracing::warn!(
        target: "presenter::bible::sermon",
        error = %err,
        "sermon request refused"
    );
    AppError::from(err)
}

fn list_dto(list: SermonList) -> SermonListDto {
    SermonListDto {
        today: list.today,
        default_service_date: list.default_service_date,
        sermons: list
            .sermons
            .into_iter()
            .map(|summary| SermonSummaryDto {
                id: summary.id.to_string(),
                title: summary.title,
                service_date: summary.service_date,
                author_email: summary.author_email,
                item_count: summary.item_count,
                slide_count: summary.slide_count,
                updated_at: summary.updated_at,
                revision: summary.revision,
            })
            .collect(),
    }
}

fn item_dto(order: usize, view: SermonItemView) -> SermonItemDto {
    let SermonItemView {
        record,
        label,
        rendered_text,
        source_changed,
        slide_ids,
    } = view;
    SermonItemDto {
        id: record.id.to_string(),
        order,
        translation: record.translation_code,
        book_code: record.book_code,
        book_number: record.book_number,
        book: record.book_name,
        chapter: record.chapter,
        verse_start: record.verse_start,
        verse_end: record.verse_end,
        source: record.source,
        edits: record.edits,
        label,
        rendered_text,
        source_changed,
        slide_ids: slide_ids.iter().map(ToString::to_string).collect(),
    }
}

fn slide_dto(view: SermonSlideView) -> SermonSlideDto {
    SermonSlideDto {
        id: view.slide.id.to_string(),
        item_id: view.item_id.to_string(),
        main_text: view.slide.main.value().to_string(),
        main_reference: view.slide.main_reference,
        secondary_text: view.slide.secondary.value().to_string(),
        secondary_reference: view.slide.secondary_reference,
    }
}

fn detail_dto(detail: SermonDetail) -> SermonDetailDto {
    let SermonDetail {
        sermon,
        items,
        slides,
        warnings,
    } = detail;
    SermonDetailDto {
        id: sermon.id.to_string(),
        title: sermon.title,
        service_date: sermon.service_date,
        author_email: sermon.author_email,
        default_translation: sermon.default_translation_code,
        secondary_translation: sermon.secondary_translation_code,
        revision: sermon.revision,
        items: items
            .into_iter()
            .enumerate()
            .map(|(order, item)| item_dto(order, item))
            .collect(),
        slides: slides.into_iter().map(slide_dto).collect(),
        warnings,
    }
}

// ---- Handlers -----------------------------------------------------------------

#[instrument(skip_all)]
async fn list_sermons(State(state): State<AppState>) -> Result<Json<SermonListDto>, AppError> {
    let list = state
        .list_sermons(local_today())
        .await
        .map_err(sermon_error)?;
    Ok(Json(list_dto(list)))
}

#[instrument(skip_all)]
async fn todays_sermons(
    State(state): State<AppState>,
) -> Result<Json<Vec<SermonDetailDto>>, AppError> {
    let details = state
        .sermons_on(local_today())
        .await
        .map_err(sermon_error)?;
    Ok(Json(details.into_iter().map(detail_dto).collect()))
}

#[instrument(skip_all)]
async fn get_sermon(State(state): State<AppState>, Path(id): Path<Uuid>) -> SermonResult {
    let detail = state
        .sermon_detail(BiblePresentationId::from_uuid(id))
        .await
        .map_err(sermon_error)?
        .ok_or_else(|| AppError::not_found("sermon not found"))?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn identity(headers: HeaderMap, peer: Peer) -> Json<IdentityDto> {
    let peer = peer.map(|Extension(ConnectInfo(addr))| addr);
    Json(IdentityDto {
        email: access_email(&headers, peer),
    })
}

#[cfg(test)]
mod tests;
