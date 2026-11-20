//! The study page's bookmark folders (#952): `GET|POST /bible/study/bookmark-folders`
//! (the reader's folders in their order, each with its bookmark `count`; a new
//! one at the end), `PUT /bible/study/bookmark-folders/{id}` (rename),
//! `POST /bible/study/bookmark-folders/delete` (`{id}`: the folder AND its
//! bookmarks, one transaction) and `PUT /bible/study/bookmark-folders/order`
//! (the whole list). JSON camelCase. The default folder "Kde som skončil" is no
//! row here: it is the bookmarks with no folder. The scope is the bookmarks'
//! (`AccessEmail`); another reader's folder is 404, a name in use 409, a bad
//! name or a full list 422.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::access_identity::AccessEmail;
use super::super::AppError;
use super::bookmarks::bookmark_error;
use crate::state::study_bookmark_folders::BookmarkFolder;
use crate::state::AppState;

const PATH: &str = "/bible/study/bookmark-folders";

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(PATH, get(list_folders).post(create_folder))
        // Static `/order` and `/delete` win over `/{id}`.
        .route(&format!("{PATH}/order"), put(reorder_folders))
        .route(&format!("{PATH}/delete"), post(delete_folder))
        .route(&format!("{PATH}/{{id}}"), put(rename_folder))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderDto {
    id: i64,
    name: String,
    /// How many bookmarks it holds (for a delete: how many went with it).
    count: usize,
}

impl From<BookmarkFolder> for FolderDto {
    fn from(folder: BookmarkFolder) -> Self {
        Self {
            id: folder.id,
            name: folder.name,
            count: folder.count,
        }
    }
}

#[derive(Debug, Deserialize)]
struct NameBody {
    name: String,
}

#[derive(Debug, Deserialize)]
struct IdBody {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct IdsBody {
    ids: Vec<i64>,
}

#[instrument(skip_all)]
async fn list_folders(
    State(state): State<AppState>,
    AccessEmail(owner): AccessEmail,
) -> Result<Json<Vec<FolderDto>>, AppError> {
    let folders = state
        .list_study_bookmark_folders(owner.as_deref())
        .await
        .map_err(bookmark_error)?;
    Ok(Json(folders.into_iter().map(FolderDto::from).collect()))
}

#[instrument(skip_all)]
async fn create_folder(
    State(state): State<AppState>,
    AccessEmail(owner): AccessEmail,
    Json(body): Json<NameBody>,
) -> Result<Json<FolderDto>, AppError> {
    let made = state
        .create_study_bookmark_folder(owner.as_deref(), &body.name)
        .await
        .map_err(bookmark_error)?;
    Ok(Json(made.into()))
}

#[instrument(skip_all)]
async fn rename_folder(
    State(state): State<AppState>,
    AccessEmail(owner): AccessEmail,
    Path(id): Path<i64>,
    Json(body): Json<NameBody>,
) -> Result<Json<FolderDto>, AppError> {
    let renamed = state
        .rename_study_bookmark_folder(owner.as_deref(), id, &body.name)
        .await
        .map_err(bookmark_error)?;
    Ok(Json(renamed.into()))
}

#[instrument(skip_all)]
async fn delete_folder(
    State(state): State<AppState>,
    AccessEmail(owner): AccessEmail,
    Json(body): Json<IdBody>,
) -> Result<Json<FolderDto>, AppError> {
    let gone = state
        .delete_study_bookmark_folder(owner.as_deref(), body.id)
        .await
        .map_err(bookmark_error)?;
    Ok(Json(gone.into()))
}

#[instrument(skip_all)]
async fn reorder_folders(
    State(state): State<AppState>,
    AccessEmail(owner): AccessEmail,
    Json(body): Json<IdsBody>,
) -> Result<StatusCode, AppError> {
    state
        .reorder_study_bookmark_folders(owner.as_deref(), &body.ids)
        .await
        .map_err(bookmark_error)?;
    Ok(StatusCode::NO_CONTENT)
}

// The HTTP tests sit with the bookmarks' (`bookmarks/tests.rs`): one app, one
// set of request helpers for both.
