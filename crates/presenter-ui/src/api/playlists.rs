use super::{delete, get_json, post_json, put_json, ApiError};
use presenter_core::Playlist;
use serde::Serialize;

/// Fetch all playlists.
pub async fn list_playlists() -> Result<Vec<Playlist>, ApiError> {
    get_json("/playlists").await
}

/// Fetch a single playlist.
pub async fn get_playlist(id: &str) -> Result<Playlist, ApiError> {
    get_json(&format!("/playlists/{id}")).await
}

#[derive(Serialize)]
pub struct CreatePlaylistRequest {
    pub name: String,
}

/// Create a new playlist.
pub async fn create_playlist(name: &str) -> Result<Playlist, ApiError> {
    post_json(
        "/playlists",
        &CreatePlaylistRequest {
            name: name.to_string(),
        },
    )
    .await
}

/// Delete a playlist.
pub async fn delete_playlist(id: &str) -> Result<(), ApiError> {
    delete(&format!("/playlists/{id}")).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddEntryRequest {
    pub presentation_id: String,
}

/// Add a presentation to a playlist.
pub async fn add_entry(playlist_id: &str, presentation_id: &str) -> Result<Playlist, ApiError> {
    post_json(
        &format!("/playlists/{playlist_id}/entries"),
        &AddEntryRequest {
            presentation_id: presentation_id.to_string(),
        },
    )
    .await
}

/// Remove an entry from a playlist.
pub async fn remove_entry(playlist_id: &str, entry_id: &str) -> Result<(), ApiError> {
    delete(&format!("/playlists/{playlist_id}/entries/{entry_id}")).await
}

#[derive(Serialize)]
pub struct ReorderRequest {
    pub entry_ids: Vec<String>,
}

/// Reorder playlist entries.
pub async fn reorder_entries(
    playlist_id: &str,
    entry_ids: Vec<String>,
) -> Result<Playlist, ApiError> {
    put_json(
        &format!("/playlists/{playlist_id}/entries/reorder"),
        &ReorderRequest { entry_ids },
    )
    .await
}
