//! The study page's bookmarks (#915, #951): `GET|POST /bible/study/bookmarks`
//! (the list newest first, each row with its verse's `text`),
//! `PUT /bible/study/bookmarks/order` (the whole list, top-first) and
//! `POST /bible/study/bookmarks/delete` (the chosen ones at once). JSON camelCase. The list is the
//! caller's: the reader's e-mail (`AccessEmail`: a verified Access login or a
//! remembered device), nobody = the shared list. An id of another list answers 404, never its row.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::access_identity::AccessEmail;
use super::super::AppError;
use crate::state::study_bookmarks::{BookmarkRefusal, PlaceRequest, StudyBookmark};
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/bible/study/bookmarks",
            get(list_bookmarks).post(create_bookmark),
        )
        .route("/bible/study/bookmarks/order", put(reorder_bookmarks))
        .route("/bible/study/bookmarks/delete", post(delete_bookmarks))
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
    /// The start of the verse's text: the bookmark's translation, else the
    /// main one; `null` outside the list or when neither has the verse.
    text: Option<String>,
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
            text: bookmark.text,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeletedDto {
    bookmark: BookmarkDto,
    /// Where it stood in the list (top-first): Späť puts it back there.
    index: usize,
}

#[derive(Debug, Deserialize)]
struct IdsBody {
    ids: Vec<i64>,
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
    AccessEmail(owner): AccessEmail,
) -> Result<Json<Vec<BookmarkDto>>, AppError> {
    let list = state
        .list_study_bookmarks(owner.as_deref())
        .await
        .map_err(bookmark_error)?;
    Ok(Json(list.into_iter().map(BookmarkDto::from).collect()))
}

#[instrument(skip_all)]
async fn create_bookmark(
    State(state): State<AppState>,
    AccessEmail(owner): AccessEmail,
    Json(body): Json<CreateBody>,
) -> Result<Json<BookmarkDto>, AppError> {
    let place = PlaceRequest::from(body.place);
    let made = state
        .create_study_bookmark(owner.as_deref(), &body.name, &place, body.index)
        .await
        .map_err(bookmark_error)?;
    Ok(Json(made.into()))
}

#[instrument(skip_all)]
async fn reorder_bookmarks(
    State(state): State<AppState>,
    AccessEmail(owner): AccessEmail,
    Json(body): Json<IdsBody>,
) -> Result<StatusCode, AppError> {
    state
        .reorder_study_bookmarks(owner.as_deref(), &body.ids)
        .await
        .map_err(bookmark_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
async fn delete_bookmarks(
    State(state): State<AppState>,
    AccessEmail(owner): AccessEmail,
    Json(body): Json<IdsBody>,
) -> Result<Json<Vec<DeletedDto>>, AppError> {
    let deleted = state
        .delete_study_bookmarks(owner.as_deref(), &body.ids)
        .await
        .map_err(bookmark_error)?;
    Ok(Json(
        deleted
            .into_iter()
            .map(|(bookmark, index)| DeletedDto {
                bookmark: bookmark.into(),
                index,
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests;
