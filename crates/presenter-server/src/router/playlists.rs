use super::AppError;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use presenter_core::{
    playlist::{MidiBinding, PlaylistEntryKind},
    Playlist, PlaylistEntry, PlaylistEntryId, PlaylistId, PresentationId,
};
use tracing::instrument;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreatePlaylistRequest {
    pub(super) name: String,
    #[serde(default)]
    pub(super) show_in_dashboard: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdatePlaylistRequest {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) show_in_dashboard: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdatePlaylistEntriesRequest {
    pub(super) entries: Vec<PlaylistEntryPayload>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub(super) enum PlaylistEntryPayload {
    Presentation {
        #[serde(default, rename = "entryId")]
        entry_id: Option<Uuid>,
        #[serde(rename = "presentationId")]
        presentation_id: Uuid,
        #[serde(default, rename = "midiNote")]
        midi_note: Option<u8>,
    },
    Separator {
        #[serde(default, rename = "entryId")]
        entry_id: Option<Uuid>,
        name: String,
    },
}

#[instrument(skip_all)]
pub(super) async fn list_playlists(
    State(state): State<AppState>,
) -> Result<Json<Vec<Playlist>>, AppError> {
    let playlists = state.playlists().await?;
    let enriched = state.enrich_playlists_with_names(playlists).await?;
    Ok(Json(enriched))
}

#[instrument(skip_all)]
pub(super) async fn create_playlist(
    State(state): State<AppState>,
    Json(payload): Json<CreatePlaylistRequest>,
) -> Result<Json<Playlist>, AppError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request_message("name cannot be empty"));
    }
    let playlist = state
        .create_playlist(name, payload.show_in_dashboard)
        .await?;
    // Just-created has no entries, but enrich for response shape consistency.
    let enriched = state.enrich_playlist_with_names(playlist).await?;
    Ok(Json(enriched))
}

#[instrument(skip_all)]
pub(super) async fn get_playlist(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Playlist>, AppError> {
    let playlist = state
        .get_playlist(PlaylistId::from_uuid(id))
        .await?
        .ok_or_else(|| AppError::not_found("playlist not found"))?;
    let enriched = state.enrich_playlist_with_names(playlist).await?;
    Ok(Json(enriched))
}

#[instrument(skip_all)]
pub(super) async fn update_playlist(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdatePlaylistRequest>,
) -> Result<Json<Playlist>, AppError> {
    let playlist_id = PlaylistId::from_uuid(id);
    if let Some(name) = payload.name.as_ref() {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request_message("name cannot be empty"));
        }
        state.rename_playlist(playlist_id, trimmed).await?;
    }

    if let Some(favorite) = payload.show_in_dashboard {
        // #633: `RepositoryError::NotFound` maps to 404 by default via the
        // centralized `From<anyhow::Error> for AppError`.
        state.set_playlist_favorite(playlist_id, favorite).await?;
    }

    let updated = state
        .playlists()
        .await?
        .into_iter()
        .find(|playlist| playlist.id == playlist_id)
        .ok_or_else(|| AppError::not_found("playlist not found"))?;
    let enriched = state.enrich_playlist_with_names(updated).await?;
    Ok(Json(enriched))
}

#[instrument(skip_all)]
pub(super) async fn delete_playlist(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    state.delete_playlist(PlaylistId::from_uuid(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn replace_playlist_entries(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdatePlaylistEntriesRequest>,
) -> Result<Json<Playlist>, AppError> {
    let entries = payload
        .entries
        .into_iter()
        .map(|entry| match entry {
            PlaylistEntryPayload::Presentation {
                entry_id,
                presentation_id,
                midi_note,
            } => {
                let id = entry_id
                    .map(PlaylistEntryId::from_uuid)
                    .unwrap_or_else(PlaylistEntryId::new);
                let binding = midi_note
                    .map(MidiBinding::new)
                    .transpose()
                    .map_err(AppError::bad_request)?;
                Ok(PlaylistEntry {
                    id,
                    kind: PlaylistEntryKind::Presentation {
                        presentation_id: PresentationId::from_uuid(presentation_id),
                        midi_binding: binding,
                        presentation_name: None,
                    },
                })
            }
            PlaylistEntryPayload::Separator { entry_id, name } => {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    return Err(AppError::bad_request_message(
                        "separator name cannot be empty",
                    ));
                }
                let id = entry_id
                    .map(PlaylistEntryId::from_uuid)
                    .unwrap_or_else(PlaylistEntryId::new);
                Ok(PlaylistEntry {
                    id,
                    kind: PlaylistEntryKind::Separator {
                        name: trimmed.to_string(),
                    },
                })
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    // #652 F9: reject a payload with a duplicate `entryId` HERE — pure body
    // validation, before the request ever reaches state/repository. Left
    // unchecked, a repeated id reaches the repository's raw INSERT unchanged
    // and trips a PK violation on the second row -> 500 (hardening; no UI
    // path emits this today).
    let mut seen_ids = std::collections::HashSet::with_capacity(entries.len());
    for entry in &entries {
        if !seen_ids.insert(entry.id) {
            return Err(AppError::unprocessable("duplicate playlist entry id"));
        }
    }

    // #633: `RepositoryError::NotFound`/`TargetNotFound` map to 404/422 by
    // default via the centralized `From<anyhow::Error> for AppError`.
    let playlist = state
        .replace_playlist_entries(PlaylistId::from_uuid(id), entries)
        .await?;
    let enriched = state.enrich_playlist_with_names(playlist).await?;
    Ok(Json(enriched))
}

#[cfg(test)]
mod duplicate_entry_id_tests {
    use super::*;
    use axum::extract::{Path, State};
    use axum::response::IntoResponse;

    /// #652 F9: a replace payload with two entries sharing the SAME explicit
    /// `entryId` reaches the repository's raw INSERT unchanged and trips a
    /// PK violation -> 500 (hardening; no UI path emits this today).
    /// Settled design: reject it in the ROUTER handler — pure body
    /// validation, no repository call at all — before the request ever
    /// reaches state.
    #[tokio::test]
    async fn replace_entries_rejects_duplicate_entry_id() {
        let state = AppState::in_memory().await.unwrap();
        let playlist = state.create_playlist("Dup Test", false).await.unwrap();
        let shared_id = Uuid::new_v4();

        let result = replace_playlist_entries(
            State(state),
            Path(playlist.id.into_uuid()),
            Json(UpdatePlaylistEntriesRequest {
                entries: vec![
                    PlaylistEntryPayload::Separator {
                        entry_id: Some(shared_id),
                        name: "A".to_string(),
                    },
                    PlaylistEntryPayload::Separator {
                        entry_id: Some(shared_id),
                        name: "B".to_string(),
                    },
                ],
            }),
        )
        .await;

        let Err(err) = result else {
            panic!("expected a duplicate-entry-id refusal, got Ok");
        };
        assert_eq!(
            err.into_response().status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "a duplicate playlist entry id must be 422, not a raw PK-violation 500 (#652 F9)"
        );
    }
}
