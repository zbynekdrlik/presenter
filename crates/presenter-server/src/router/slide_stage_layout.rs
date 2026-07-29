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
use crate::state::stage_display::StageLayoutRefusal;
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
        .map_err(map_slide_stage_layout_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Maps a `assign_slide_stage_layout` error to its HTTP status via the TYPED
/// `StageLayoutRefusal` — never a blanket `AppError::bad_request` (#615: that
/// hid a genuine internal failure, e.g. a missing presentation or a DB error,
/// as a benign client-side 400). Same shape as #588's
/// `map_stage_layout_refusal` in `router/stage.rs`, but maps to
/// `bad_request_message` (400) — the SAME client-error status this route has
/// ALWAYS returned for bad layout codes — so only the fallthrough to 500
/// changes, not the typed-refusal status. Any other error falls through to
/// the router's default 500 mapping.
fn map_slide_stage_layout_error(err: anyhow::Error) -> AppError {
    match err.downcast_ref::<StageLayoutRefusal>() {
        Some(refusal) => AppError::bad_request_message(refusal.to_string()),
        None => err.into(),
    }
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
    use axum::response::IntoResponse;

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

    /// #615: an unknown/invalid layout code must still answer 400 — the SAME
    /// client-error status the route returns today. `StageLayoutRefusal` is
    /// a client-side refusal (bad layout code), not an internal failure.
    #[tokio::test]
    async fn put_returns_400_for_unknown_layout_code() {
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
        let Err(err) = result else {
            panic!("expected an error for an unknown layout code, got Ok");
        };
        assert_eq!(
            err.into_response().status(),
            StatusCode::BAD_REQUEST,
            "an unknown layout code must be 400 (same client-error status as today, #615)"
        );
    }

    /// #615: a genuine internal failure (here: a non-existent presentation
    /// causes `assign_slide_stage_layout` to `bail!("presentation not
    /// found")`, an untyped `anyhow::Error` that is NOT a
    /// `StageLayoutRefusal`) must answer 500 — never 400. Before the fix,
    /// the blanket `.map_err(AppError::bad_request)` masked this as a
    /// client error.
    #[tokio::test]
    async fn put_returns_500_on_internal_failure_not_400() {
        let (state, _presentation) = seeded_state().await;
        // A valid-format UUID that doesn't correspond to any seeded
        // presentation: `presentation_detail` returns `None`, the state
        // method `bail!`s — a non-typed error that must fall through to 500.
        let random_uuid = uuid::Uuid::new_v4();
        let random_slide = presenter_core::SlideId::new();

        let result = set_slide_stage_layout(
            State(state),
            Path((random_uuid.to_string(), random_slide.to_string())),
            Json(SlideStageLayoutRequest {
                layout_code: Some("fulltext".to_string()),
            }),
        )
        .await;
        let Err(err) = result else {
            panic!("expected an error for a non-existent presentation, got Ok");
        };
        assert_eq!(
            err.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "an internal failure must be 500, not 400 (#615)"
        );
    }
}
