//! Per-slide stage-layout marker endpoints (#515).
//!
//! `PUT  /presentations/{presentation_id}/slides/{slide_id}/stage-layout`
//!     body `{"layoutCode": "fulltext"}` assigns, `{"layoutCode": null}` clears.
//! `GET  /presentations/{presentation_id}/slide-stage-layouts`
//!     returns `{ "<slide_id>": "<layout_code>", … }` for the operator UI.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use presenter_core::{PresentationId, SlideId};
use serde::Deserialize;
use std::collections::HashMap;
use tracing::instrument;

use super::AppError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SlideStageLayoutRequest {
    /// `Some(code)` assigns the marker; `None` (JSON `null` / omitted) clears it.
    #[serde(default)]
    pub(super) layout_code: Option<String>,
}

#[instrument(skip_all)]
pub(super) async fn set_slide_stage_layout(
    State(state): State<AppState>,
    Path((presentation_id, slide_id)): Path<(String, String)>,
    Json(payload): Json<SlideStageLayoutRequest>,
) -> Result<StatusCode, AppError> {
    let presentation_uuid = super::parse_uuid("presentationId", &presentation_id)?;
    let slide_uuid = super::parse_uuid("slideId", &slide_id)?;
    let code = payload
        .layout_code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty());
    state
        .assign_slide_stage_layout(
            PresentationId::from_uuid(presentation_uuid),
            SlideId::from_uuid(slide_uuid),
            code,
        )
        .await
        .map_err(AppError::bad_request)?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn list_slide_stage_layouts(
    State(state): State<AppState>,
    Path(presentation_id): Path<String>,
) -> Result<Json<HashMap<String, String>>, AppError> {
    let presentation_uuid = super::parse_uuid("presentationId", &presentation_id)?;
    let map = state
        .slide_stage_layouts(PresentationId::from_uuid(presentation_uuid))
        .await?;
    Ok(Json(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, State};

    async fn seeded_state() -> (crate::state::AppState, presenter_core::Presentation) {
        let state = crate::state::AppState::in_memory().await.unwrap();
        crate::state::seed_sample_library(&state).await.unwrap();
        let libraries = state.libraries().await.unwrap();
        let presentation = libraries[0].presentations[0].clone();
        (state, presentation)
    }

    #[tokio::test]
    async fn put_assigns_and_null_clears() {
        let (state, presentation) = seeded_state().await;
        let slide_id = presentation.slides[0].id;

        let status = set_slide_stage_layout(
            State(state.clone()),
            Path((presentation.id.to_string(), slide_id.to_string())),
            Json(SlideStageLayoutRequest {
                layout_code: Some("fulltext".to_string()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);

        let Json(map) =
            list_slide_stage_layouts(State(state.clone()), Path(presentation.id.to_string()))
                .await
                .unwrap();
        assert_eq!(
            map.get(&slide_id.to_string()).map(String::as_str),
            Some("fulltext")
        );

        // null layoutCode clears the marker.
        let status = set_slide_stage_layout(
            State(state.clone()),
            Path((presentation.id.to_string(), slide_id.to_string())),
            Json(SlideStageLayoutRequest { layout_code: None }),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);

        let Json(map) = list_slide_stage_layouts(State(state), Path(presentation.id.to_string()))
            .await
            .unwrap();
        assert!(map.is_empty());
    }

    #[tokio::test]
    async fn put_rejects_unknown_layout() {
        let (state, presentation) = seeded_state().await;
        let slide_id = presentation.slides[0].id;

        let result = set_slide_stage_layout(
            State(state),
            Path((presentation.id.to_string(), slide_id.to_string())),
            Json(SlideStageLayoutRequest {
                layout_code: Some("no-such-layout".to_string()),
            }),
        )
        .await;
        assert!(result.is_err(), "unknown layout must be rejected");
    }
}
