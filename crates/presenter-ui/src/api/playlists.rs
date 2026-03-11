use super::{delete, get_json, patch_json, post_json, put_json, ApiError};
use presenter_core::Playlist;
use serde::Serialize;

pub async fn list_playlists() -> Result<Vec<Playlist>, ApiError> {
    get_json("/playlists").await
}

pub async fn get_playlist(id: &str) -> Result<Playlist, ApiError> {
    get_json(&format!("/playlists/{id}")).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatePlaylistRequest {
    name: String,
    show_in_dashboard: bool,
}

pub async fn create_playlist(name: &str, show_in_dashboard: bool) -> Result<Playlist, ApiError> {
    post_json(
        "/playlists",
        &CreatePlaylistRequest {
            name: name.to_string(),
            show_in_dashboard,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePlaylistRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    show_in_dashboard: Option<bool>,
}

pub async fn update_playlist(
    id: &str,
    name: Option<String>,
    show_in_dashboard: Option<bool>,
) -> Result<Playlist, ApiError> {
    patch_json(
        &format!("/playlists/{id}"),
        &UpdatePlaylistRequest {
            name,
            show_in_dashboard,
        },
    )
    .await
}

pub async fn delete_playlist(id: &str) -> Result<(), ApiError> {
    delete(&format!("/playlists/{id}")).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateEntriesRequest {
    entries: Vec<PlaylistEntryPayload>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum PlaylistEntryPayload {
    Presentation {
        #[serde(skip_serializing_if = "Option::is_none")]
        entry_id: Option<String>,
        presentation_id: String,
    },
    Separator {
        #[serde(skip_serializing_if = "Option::is_none")]
        entry_id: Option<String>,
        name: String,
    },
}

pub async fn replace_entries(
    playlist_id: &str,
    entries: Vec<PlaylistEntryPayload>,
) -> Result<Playlist, ApiError> {
    put_json(
        &format!("/playlists/{playlist_id}/entries"),
        &UpdateEntriesRequest { entries },
    )
    .await
}

pub async fn add_presentation_to_playlist(
    playlist_id: &str,
    presentation_id: &str,
    existing_entries: &[PlaylistEntryPayload],
) -> Result<Playlist, ApiError> {
    let mut entries = existing_entries.to_vec();
    entries.push(PlaylistEntryPayload::Presentation {
        entry_id: None,
        presentation_id: presentation_id.to_string(),
    });
    replace_entries(playlist_id, entries).await
}
