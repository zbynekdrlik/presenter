use axum::{extract::{Path, State}, http::StatusCode, Json};
use uuid::Uuid;
use crate::state::AppState;
use super::{
  AppError,
  CreatePlaylistRequest,
  UpdatePlaylistEntriesRequest,
  UpdatePlaylistRequest,
};

pub(super) async fn list_playlists(
    State(state): State<AppState>,
) -> Result<Json<Vec<presenter_core::Playlist>>, AppError> {
    super::list_playlists(State(state)).await
}

pub(super) async fn create_playlist(
    State(state): State<AppState>,
    Json(payload): Json<CreatePlaylistRequest>,
) -> Result<Json<presenter_core::Playlist>, AppError> {
    super::create_playlist(State(state), Json(payload)).await
}

pub(super) async fn update_playlist(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdatePlaylistRequest>,
) -> Result<Json<presenter_core::Playlist>, AppError> {
    super::update_playlist(State(state), Path(id), Json(payload)).await
}

pub(super) async fn delete_playlist(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    super::delete_playlist(State(state), Path(id)).await
}

pub(super) async fn replace_playlist_entries(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdatePlaylistEntriesRequest>,
) -> Result<Json<presenter_core::Playlist>, AppError> {
    super::replace_playlist_entries(State(state), Path(id), Json(payload)).await
}
