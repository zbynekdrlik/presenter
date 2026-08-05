use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::{parse_uuid, AppError};
use crate::state::stage_display::StageLayoutRefusal;
use crate::state::AppState;
use axum::http::StatusCode;
use presenter_core::{
    PlaylistId, PresentationId, SlideId, StageDisplayLayout, StageDisplaySnapshot,
};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StageSnapshotQuery {
    pub(super) layout: Option<String>,
}

#[instrument(skip_all)]
pub(super) async fn stage_display_selected_snapshot_json(
    State(state): State<AppState>,
    Query(query): Query<StageSnapshotQuery>,
) -> Result<Json<StageDisplaySnapshot>, AppError> {
    let result = if let Some(code) = query.layout.as_deref() {
        state.stage_display_snapshot(code).await?
    } else {
        state.selected_stage_display_snapshot().await?
    };
    match result {
        Some(snapshot) => Ok(Json(snapshot)),
        None => Err(AppError::not_found("Stage display unavailable")),
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StageLayoutResponse {
    pub(super) code: String,
    pub(super) layout: StageDisplayLayout,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StageLayoutUpdateRequest {
    pub(super) code: String,
}

#[instrument(skip_all)]
pub(super) async fn get_stage_layout(
    State(state): State<AppState>,
) -> Result<Json<StageLayoutResponse>, AppError> {
    let code = state.stage_layout_code().await;
    let layouts = state.stage_displays().await?;
    let layout = layouts
        .into_iter()
        .find(|layout| layout.code == code)
        .or_else(|| StageDisplayLayout::built_in().into_iter().next())
        .ok_or_else(|| AppError::internal("no stage layouts available"))?;
    Ok(Json(StageLayoutResponse {
        code: layout.code.clone(),
        layout,
    }))
}

/// Maps a `set_stage_layout_code` refusal to its HTTP status via the TYPED
/// `StageLayoutRefusal` error — never a blanket `.to_string()`-based 404
/// (#588: that hid a genuine internal failure, e.g. a corrupted `timers` row
/// surfaced by `broadcast_stage_snapshots`, as a benign "layout not found").
/// Any other error falls through to the default 500 mapping.
fn map_stage_layout_refusal(err: anyhow::Error) -> AppError {
    match err.downcast_ref::<StageLayoutRefusal>() {
        Some(refusal) => AppError::not_found(refusal.to_string()),
        None => err.into(),
    }
}

#[instrument(skip_all)]
pub(super) async fn set_stage_layout(
    State(state): State<AppState>,
    Json(payload): Json<StageLayoutUpdateRequest>,
) -> Result<Json<StageLayoutResponse>, AppError> {
    let code = payload.code.trim();
    if code.is_empty() {
        return Err(AppError::bad_request_message("code cannot be empty"));
    }
    let layout = state
        .set_stage_layout_code(code)
        .await
        .map_err(map_stage_layout_refusal)?;
    Ok(Json(StageLayoutResponse {
        code: layout.code.clone(),
        layout,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StageStateRequest {
    pub(super) presentation_id: String,
    pub(super) current_slide_id: String,
    #[serde(default)]
    pub(super) next_slide_id: Option<String>,
    #[serde(default)]
    pub(super) playlist_id: Option<String>,
    /// #496: zero-based index of the triggered playlist entry. Additive and
    /// optional — old clients omit it and the server falls back to marking by
    /// presentation_id. Lets the server disambiguate a repeated song.
    #[serde(default)]
    pub(super) entry_index: Option<u32>,
}

#[instrument(skip_all)]
pub(super) async fn update_stage_state(
    State(state): State<AppState>,
    Json(payload): Json<StageStateRequest>,
) -> Result<StatusCode, AppError> {
    let presentation_id =
        PresentationId::from_uuid(parse_uuid("presentationId", &payload.presentation_id)?);
    let current_slide_id =
        SlideId::from_uuid(parse_uuid("currentSlideId", &payload.current_slide_id)?);
    let next_slide_id = match payload.next_slide_id {
        Some(value) => Some(SlideId::from_uuid(parse_uuid("nextSlideId", &value)?)),
        None => None,
    };
    let playlist_id = match payload.playlist_id {
        Some(value) => Some(PlaylistId::from_uuid(parse_uuid("playlistId", &value)?)),
        None => None,
    };
    state
        .update_stage_state(
            presentation_id,
            current_slide_id,
            next_slide_id,
            playlist_id,
            payload.entry_index,
        )
        .await
        .map_err(AppError::bad_request)?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn list_stage_connections(
    State(state): State<AppState>,
) -> Result<Json<Vec<presenter_core::StageClientSnapshot>>, AppError> {
    let snapshot = state.stage_connections_snapshot().await;
    Ok(Json(snapshot))
}

#[instrument(skip_all)]
pub(super) async fn list_stage_displays(
    State(state): State<AppState>,
) -> Result<Json<Vec<StageDisplayLayout>>, AppError> {
    let displays = state.stage_displays().await?;
    Ok(Json(displays))
}

#[instrument(skip_all)]
pub(super) async fn clear_stage_state(
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    state.clear_stage().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BroadcastLiveResponse {
    pub(super) enabled: bool,
}

#[instrument(skip_all)]
pub(super) async fn get_broadcast_live(
    State(state): State<AppState>,
) -> Json<BroadcastLiveResponse> {
    Json(BroadcastLiveResponse {
        enabled: state.broadcast_live(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BroadcastLiveRequest {
    pub(super) enabled: bool,
}

#[instrument(skip_all)]
pub(super) async fn set_broadcast_live(
    State(state): State<AppState>,
    Json(payload): Json<BroadcastLiveRequest>,
) -> StatusCode {
    state.set_broadcast_live(payload.enabled);
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod camera_snapshot_query_tests {
    use super::*;
    use axum::extract::{Query, State};

    #[tokio::test]
    async fn snapshot_query_returns_requested_layout_regardless_of_global_selection() {
        let state = crate::state::AppState::in_memory().await.unwrap();
        // Seed a presentation so a snapshot can be produced.
        crate::state::seed_sample_library(&state).await.unwrap();
        state.set_stage_layout_code("worship-snv").await.unwrap();

        let query = StageSnapshotQuery {
            layout: Some("camera-crew".to_string()),
        };
        let result = stage_display_selected_snapshot_json(State(state), Query(query)).await;
        let Ok(axum::Json(snapshot)) = result else {
            panic!("expected Ok with snapshot, got {result:?}");
        };
        assert_eq!(snapshot.layout.code, "camera-crew");
    }
}

#[cfg(test)]
mod broadcast_live_handler_tests {
    use super::*;
    use axum::extract::State;

    #[tokio::test]
    async fn set_broadcast_live_handler_toggles_state() {
        let state = crate::state::AppState::in_memory().await.unwrap();
        assert!(!state.broadcast_live());

        let status = set_broadcast_live(
            State(state.clone()),
            Json(BroadcastLiveRequest { enabled: true }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(state.broadcast_live());

        let status = set_broadcast_live(
            State(state.clone()),
            Json(BroadcastLiveRequest { enabled: false }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(!state.broadcast_live());
    }
}

/// #588: `set_stage_layout` must discriminate a genuine internal failure from
/// an unknown/invalid layout code — not blanket-map every error to 404.
#[cfg(test)]
mod set_stage_layout_error_mapping_tests {
    use super::*;
    use axum::extract::State;
    use axum::response::IntoResponse;

    /// An internal failure unrelated to the layout code itself (here: a
    /// corrupted `timers` row surfaced while `broadcast_stage_snapshots`
    /// rebuilds the stage context after a VALID layout switch) must answer
    /// 500 — never 404. Before the #588 fix, `set_stage_layout` mapped every
    /// error from `set_stage_layout_code` to `AppError::not_found`, so this
    /// genuine server fault was reported to the client as "layout not
    /// found".
    #[tokio::test]
    async fn set_stage_layout_returns_500_on_internal_failure_not_404() {
        let state = crate::state::AppState::in_memory().await.unwrap();
        // Seed the timers row (created lazily by the first stage-context
        // rebuild) via an initial, unrelated valid layout switch.
        let _ = set_stage_layout(
            State(state.clone()),
            Json(StageLayoutUpdateRequest {
                code: "timer".to_string(),
            }),
        )
        .await
        .unwrap();

        // Corrupt the persisted timer state directly via raw SQL so the next
        // stage-context rebuild hits `RepositoryError::UnknownTimerState` —
        // a real internal fault, unrelated to the requested layout code.
        let conn = state.repository().connection();
        sea_orm::ConnectionTrait::execute(
            conn,
            sea_orm::Statement::from_string(
                sea_orm::ConnectionTrait::get_database_backend(conn),
                "UPDATE timers SET countdown_state = 'corrupted-state'".to_string(),
            ),
        )
        .await
        .unwrap();

        // Switch to a DIFFERENT valid, operator-selectable code so the
        // early-return-on-unchanged-code guard doesn't short-circuit before
        // `broadcast_stage_snapshots` runs.
        let result = set_stage_layout(
            State(state),
            Json(StageLayoutUpdateRequest {
                code: "preach".to_string(),
            }),
        )
        .await;

        let Err(err) = result else {
            panic!("expected an error from the corrupted timers row, got Ok");
        };
        assert_eq!(
            err.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "an internal failure on the layout-switch path must be 500, not 404 (#588)"
        );
    }

    /// #631: a failure while assembling the fresh stage snapshot must NOT
    /// have already applied, persisted, and broadcast the layout switch — a
    /// client that receives this exact 500 must find NOTHING changed. Same
    /// corrupted-timers-row setup as the sibling test above, but the
    /// corruption happens BEFORE the switch attempt (not after), so the
    /// failure surfaces from the pre-switch snapshot-context build rather
    /// than from a step that runs after the switch already committed.
    #[tokio::test]
    async fn set_stage_layout_failure_does_not_apply_the_switch() {
        let state = crate::state::AppState::in_memory().await.unwrap();
        // Seed the timers row via an initial valid switch.
        let _ = set_stage_layout(
            State(state.clone()),
            Json(StageLayoutUpdateRequest {
                code: "timer".to_string(),
            }),
        )
        .await
        .unwrap();

        // Corrupt it so the NEXT switch's snapshot-context build fails.
        let conn = state.repository().connection();
        sea_orm::ConnectionTrait::execute(
            conn,
            sea_orm::Statement::from_string(
                sea_orm::ConnectionTrait::get_database_backend(conn),
                "UPDATE timers SET countdown_state = 'corrupted-state'".to_string(),
            ),
        )
        .await
        .unwrap();

        let result = set_stage_layout(
            State(state.clone()),
            Json(StageLayoutUpdateRequest {
                code: "preach".to_string(),
            }),
        )
        .await;

        assert!(
            result.is_err(),
            "expected the corrupted-state failure to surface as an error"
        );
        assert_eq!(
            state.stage_layout_code().await,
            "timer",
            "#631: a failed switch must not have applied the layout change — \
             the client's 500 must mean nothing changed"
        );
    }

    /// An unknown/invalid layout code must still answer 404 — the acceptance
    /// criterion #588 must NOT regress.
    #[tokio::test]
    async fn set_stage_layout_returns_404_for_unknown_code() {
        let state = crate::state::AppState::in_memory().await.unwrap();
        let result = set_stage_layout(
            State(state),
            Json(StageLayoutUpdateRequest {
                code: "no-such-layout".to_string(),
            }),
        )
        .await;
        let Err(err) = result else {
            panic!("expected an error for an unknown layout code, got Ok");
        };
        assert_eq!(
            err.into_response().status(),
            StatusCode::NOT_FOUND,
            "an unknown layout code must be 404"
        );
    }
}
