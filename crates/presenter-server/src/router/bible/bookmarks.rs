//! The study page's named bookmarks (#915): `GET|POST /bible/study/bookmarks`,
//! `PATCH|DELETE /bible/study/bookmarks/{id}`. JSON camelCase. The list is the
//! caller's: the Cloudflare Access e-mail (`access_email`), none on the LAN =
//! the shared list. An id of another list answers 404, never its row.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, patch};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::access_identity::access_email;
use super::super::AppError;
use super::sermons::Peer;
use crate::state::study_bookmarks::{BookmarkRefusal, PlaceRequest, StudyBookmark};
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/bible/study/bookmarks",
            get(list_bookmarks).post(create_bookmark),
        )
        .route(
            "/bible/study/bookmarks/{id}",
            patch(update_bookmark).delete(delete_bookmark),
        )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BookmarkDto {
    id: i64,
    name: String,
    translation: String,
    book_code: String,
    /// The translation's own name of the book.
    book: String,
    chapter: u16,
    verse: u16,
    updated_at: String,
}

impl From<StudyBookmark> for BookmarkDto {
    fn from(bookmark: StudyBookmark) -> Self {
        Self {
            id: bookmark.id,
            name: bookmark.name,
            translation: bookmark.translation,
            book_code: bookmark.book_code,
            book: bookmark.book,
            chapter: bookmark.chapter,
            verse: bookmark.verse,
            updated_at: bookmark.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeletedDto {
    bookmark: BookmarkDto,
    /// Where it stood in the list: Späť puts it back there.
    index: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaceBody {
    translation: String,
    book_code: String,
    chapter: u16,
    verse: u16,
}

impl From<PlaceBody> for PlaceRequest {
    fn from(body: PlaceBody) -> Self {
        Self {
            translation: body.translation,
            book_code: body.book_code,
            chapter: body.chapter,
            verse: body.verse,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateBody {
    name: String,
    #[serde(flatten)]
    place: PlaceBody,
    #[serde(default)]
    index: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    place: Option<PlaceBody>,
}

fn owner(headers: &HeaderMap, peer: Peer) -> Option<String> {
    access_email(headers, peer.map(|Extension(info)| info.0))
}

/// `BookmarkRefusal` → 422 with its Slovak message; everything else through the
/// central mapping (an unknown id is the repository's `NotFound` → 404).
fn bookmark_error(err: anyhow::Error) -> AppError {
    tracing::warn!(
        target: "presenter::bible::bookmark",
        error = %err,
        "bookmark request refused"
    );
    match err.downcast_ref::<BookmarkRefusal>() {
        Some(refusal) => AppError::unprocessable(refusal.0.clone()),
        None => AppError::from(err),
    }
}

#[instrument(skip_all)]
async fn list_bookmarks(
    State(state): State<AppState>,
    peer: Peer,
    headers: HeaderMap,
) -> Result<Json<Vec<BookmarkDto>>, AppError> {
    let owner = owner(&headers, peer);
    let list = state
        .list_study_bookmarks(owner.as_deref())
        .await
        .map_err(bookmark_error)?;
    Ok(Json(list.into_iter().map(BookmarkDto::from).collect()))
}

#[instrument(skip_all)]
async fn create_bookmark(
    State(state): State<AppState>,
    peer: Peer,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> Result<Json<BookmarkDto>, AppError> {
    let owner = owner(&headers, peer);
    let place = PlaceRequest::from(body.place);
    let made = state
        .create_study_bookmark(owner.as_deref(), &body.name, &place, body.index)
        .await
        .map_err(bookmark_error)?;
    Ok(Json(made.into()))
}

#[instrument(skip_all)]
async fn update_bookmark(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    peer: Peer,
    headers: HeaderMap,
    Json(body): Json<UpdateBody>,
) -> Result<Json<BookmarkDto>, AppError> {
    let owner = owner(&headers, peer);
    let place = body.place.map(PlaceRequest::from);
    let changed = state
        .update_study_bookmark(owner.as_deref(), id, body.name.as_deref(), place.as_ref())
        .await
        .map_err(bookmark_error)?;
    Ok(Json(changed.into()))
}

#[instrument(skip_all)]
async fn delete_bookmark(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    peer: Peer,
    headers: HeaderMap,
) -> Result<Json<DeletedDto>, AppError> {
    let owner = owner(&headers, peer);
    let (bookmark, index) = state
        .delete_study_bookmark(owner.as_deref(), id)
        .await
        .map_err(bookmark_error)?;
    Ok(Json(DeletedDto {
        bookmark: bookmark.into(),
        index,
    }))
}

#[cfg(test)]
mod tests;
