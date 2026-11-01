//! The sermon API (#851 spec §11) and `GET /bible/study/identity` (§9). JSON is
//! camelCase, query parameters snake_case, like the other Bible routes. The
//! reads come first (P4a); the writes carry `expectedRevision` and answer the
//! full sermon. `sermon_error` is the ONE place a sermon handler turns an error
//! into a response.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, patch, post, put};
use axum::{Extension, Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use presenter_core::bible::sermon::{ItemEdits, SourceVerse};
use presenter_core::BiblePresentationId;
use serde::{Deserialize, Deserializer, Serialize};
use tracing::instrument;
use uuid::Uuid;

use super::super::access_identity::access_email;
use super::super::integrations::extract_client_ip;
use super::super::AppError;
use crate::state::sermons::{
    local_today, ItemRange, SermonDetail, SermonDraft, SermonItemView, SermonList, SermonOrigin,
    SermonPatch, SermonRefusal, SermonSlideView,
};
use crate::state::AppState;

type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;
type SermonResult = Result<Json<SermonDetailDto>, AppError>;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/bible/sermons", get(list_sermons).post(create_sermon))
        .route("/bible/sermons/today", get(todays_sermons))
        .route(
            "/bible/sermons/{id}",
            get(get_sermon).patch(update_sermon).delete(delete_sermon),
        )
        .route("/bible/sermons/{id}/items", post(add_item))
        .route("/bible/sermons/{id}/items/reorder", post(reorder_items))
        .route(
            "/bible/sermons/{id}/items/{item_id}",
            patch(patch_item).delete(delete_item),
        )
        .route("/bible/sermons/{id}/items/{item_id}/edits", put(put_edits))
        .route("/bible/sermons/{id}/regenerate", post(regenerate))
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
    secondary_missing: bool,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSermonRequest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    service_date: Option<NaiveDate>,
    #[serde(default)]
    default_translation: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    secondary_translation: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddItemRequest {
    expected_revision: i64,
    #[serde(default)]
    position: Option<usize>,
    translation: String,
    book_code: String,
    chapter: u16,
    verse_start: u16,
    verse_end: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditsRequest {
    expected_revision: i64,
    verses: ItemEdits,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSermonRequest {
    expected_revision: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    service_date: Option<NaiveDate>,
    #[serde(default, deserialize_with = "double_option")]
    secondary_translation: Option<Option<String>>,
}

/// Either `{expectedRevision, translation}` or `{expectedRevision,
/// refreshSource: true}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchItemRequest {
    expected_revision: i64,
    #[serde(default)]
    translation: Option<String>,
    #[serde(default)]
    refresh_source: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderRequest {
    expected_revision: i64,
    item_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RevisionRequest {
    expected_revision: i64,
}

/// `?expectedRevision=` as spec §11 writes it; `expected_revision` (the
/// snake_case query convention) is accepted too.
#[derive(Debug, Deserialize)]
struct RevisionQuery {
    #[serde(rename = "expectedRevision", alias = "expected_revision")]
    expected_revision: i64,
}

/// serde double-`Option`: an ABSENT field is `None`, an explicit `null` is
/// `Some(None)` ("no secondary line"), a value is `Some(Some(v))`.
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(deserializer)?))
}

// ---- Mapping ----------------------------------------------------------------

/// The author label and client IP of a write (spec §9, §13).
fn origin(headers: &HeaderMap, peer: Peer) -> SermonOrigin {
    let peer = peer.map(|Extension(ConnectInfo(addr))| addr);
    SermonOrigin {
        author: access_email(headers, peer),
        client_ip: extract_client_ip(headers, peer),
    }
}

/// A refused sermon request (spec §12, logged at WARN): `SermonRefusal` → 422
/// with its Slovak message, everything else through the central mapping.
fn sermon_error(err: anyhow::Error) -> AppError {
    tracing::warn!(
        target: "presenter::bible::sermon",
        error = %err,
        "sermon request refused"
    );
    match err.downcast_ref::<SermonRefusal>() {
        Some(refusal) => AppError::unprocessable(refusal.0.clone()),
        None => AppError::from(err),
    }
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
        secondary_missing,
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
        secondary_missing,
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
async fn create_sermon(
    State(state): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<CreateSermonRequest>,
) -> SermonResult {
    let draft = SermonDraft {
        title: body.title,
        service_date: body.service_date,
        default_translation: body.default_translation,
        secondary_translation: body.secondary_translation,
    };
    let detail = state
        .create_sermon(draft, local_today(), &origin(&headers, peer))
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
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
async fn delete_sermon(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<StatusCode, AppError> {
    state
        .delete_sermon(BiblePresentationId::from_uuid(id), &origin(&headers, peer))
        .await
        .map_err(sermon_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
async fn add_item(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<AddItemRequest>,
) -> SermonResult {
    let range = ItemRange {
        translation: body.translation,
        book_code: body.book_code,
        chapter: body.chapter,
        verse_start: body.verse_start,
        verse_end: body.verse_end,
    };
    let detail = state
        .add_sermon_item(
            BiblePresentationId::from_uuid(id),
            body.expected_revision,
            body.position,
            &range,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn put_edits(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<EditsRequest>,
) -> SermonResult {
    let detail = state
        .set_sermon_item_edits(
            BiblePresentationId::from_uuid(id),
            item_id,
            body.expected_revision,
            &body.verses,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn update_sermon(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<UpdateSermonRequest>,
) -> SermonResult {
    let patch = SermonPatch {
        title: body.title,
        service_date: body.service_date,
        secondary_translation: body.secondary_translation,
    };
    let detail = state
        .update_sermon(
            BiblePresentationId::from_uuid(id),
            body.expected_revision,
            patch,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn patch_item(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<PatchItemRequest>,
) -> SermonResult {
    let id = BiblePresentationId::from_uuid(id);
    let origin = origin(&headers, peer);
    let revision = body.expected_revision;
    let result = match (body.refresh_source, body.translation.as_deref()) {
        (true, _) => {
            state
                .refresh_sermon_item_source(id, item_id, revision, &origin)
                .await
        }
        (false, Some(translation)) => {
            state
                .change_sermon_item_translation(id, item_id, revision, translation, &origin)
                .await
        }
        (false, None) => {
            return Err(AppError::bad_request_message(
                "translation or refreshSource is required",
            ))
        }
    };
    Ok(Json(detail_dto(result.map_err(sermon_error)?)))
}

#[instrument(skip_all)]
async fn delete_item(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<RevisionQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> SermonResult {
    let detail = state
        .delete_sermon_item(
            BiblePresentationId::from_uuid(id),
            item_id,
            query.expected_revision,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn reorder_items(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<ReorderRequest>,
) -> SermonResult {
    let detail = state
        .reorder_sermon_items(
            BiblePresentationId::from_uuid(id),
            body.expected_revision,
            &body.item_ids,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
    Ok(Json(detail_dto(detail)))
}

#[instrument(skip_all)]
async fn regenerate(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    peer: Peer,
    Json(body): Json<RevisionRequest>,
) -> SermonResult {
    let detail = state
        .regenerate_sermon(
            BiblePresentationId::from_uuid(id),
            body.expected_revision,
            &origin(&headers, peer),
        )
        .await
        .map_err(sermon_error)?;
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
