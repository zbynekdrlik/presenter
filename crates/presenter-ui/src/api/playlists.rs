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
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PlaylistEntryPayload {
    Presentation {
        #[serde(skip_serializing_if = "Option::is_none", rename = "entryId")]
        entry_id: Option<String>,
        #[serde(rename = "presentationId")]
        presentation_id: String,
    },
    Separator {
        #[serde(skip_serializing_if = "Option::is_none", rename = "entryId")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_entry_serializes_camelcase_fields() {
        // Regression guard: serde's `rename_all = "camelCase"` on internally
        // tagged enums (`tag = "type"`) does NOT rename inner-struct fields,
        // only variant names. Without explicit `rename = "presentationId"`
        // attributes, the WASM client sent `presentation_id` (snake_case)
        // and the server rejected it with 422. Drag-drop into a playlist
        // silently no-oped as a result.
        let payload = PlaylistEntryPayload::Presentation {
            entry_id: None,
            presentation_id: "00000000-0000-0000-0000-000000000001".to_string(),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(
            json.contains("\"presentationId\""),
            "expected camelCase field; got: {json}"
        );
        assert!(
            !json.contains("\"presentation_id\""),
            "snake_case field leaked through: {json}"
        );
        assert!(
            json.contains("\"type\":\"presentation\""),
            "tag should be lowercase variant: {json}"
        );
    }

    #[test]
    fn presentation_entry_with_existing_id_renames_entry_id() {
        let payload = PlaylistEntryPayload::Presentation {
            entry_id: Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string()),
            presentation_id: "00000000-0000-0000-0000-000000000002".to_string(),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(
            json.contains("\"entryId\""),
            "expected camelCase entryId; got: {json}"
        );
        assert!(
            !json.contains("\"entry_id\""),
            "snake_case entry_id leaked through: {json}"
        );
    }

    #[test]
    fn separator_entry_serializes_with_renamed_entry_id() {
        let payload = PlaylistEntryPayload::Separator {
            entry_id: Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string()),
            name: "section".to_string(),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(json.contains("\"entryId\""), "got: {json}");
        assert!(!json.contains("\"entry_id\""), "got: {json}");
        assert!(
            json.contains("\"type\":\"separator\""),
            "tag should be lowercase variant: {json}"
        );
    }
}
