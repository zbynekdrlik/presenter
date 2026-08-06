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

/// Maps a `assign_slide_stage_layout` error to its HTTP status via TWO typed
/// error types — never a blanket `AppError::bad_request` (#615: that hid a
/// genuine internal failure, e.g. a DB error, as a benign client-side 400).
/// `StageLayoutRefusal` (bad/unknown layout code) maps to `bad_request_message`
/// (400) — the SAME client-error status this route has ALWAYS returned for
/// bad layout codes. `RepositoryError::NotFound` (a missing `presentation_id`
/// or `slide_id` — both URL-path resources) maps to `not_found` (404, #629):
/// before #629 these were bare `anyhow!`s that also fell through to 500. Any
/// other error still falls through to the router's default 500 mapping.
fn map_slide_stage_layout_error(err: anyhow::Error) -> AppError {
    if let Some(refusal) = err.downcast_ref::<StageLayoutRefusal>() {
        return AppError::bad_request_message(refusal.to_string());
    }
    if let Some(presenter_persistence::RepositoryError::NotFound(msg)) =
        err.downcast_ref::<presenter_persistence::RepositoryError>()
    {
        return AppError::not_found(*msg);
    }
    err.into()
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

    /// #629 (was wrong before this fix): the ORIGINAL version of this test
    /// used a fake/non-existent presentation UUID and asserted 500 — but a
    /// missing presentation is a MISSING URL RESOURCE, not an internal
    /// failure, and cementing 500 as its "correct" response is exactly the
    /// regression #629 reports (see the sibling 404 tests below for the
    /// actual fix). A genuine internal failure — unrelated to any missing id
    /// or bad layout code — is a real presentation + real slide whose
    /// `slide_stage_layouts` table has been dropped out from under the
    /// final upsert, so `set_slide_stage_layout`'s INSERT hits a raw DbErr.
    #[tokio::test]
    async fn put_returns_500_on_internal_failure_not_400() {
        let (state, presentation) = seeded_state().await;
        let slide_id = presentation.slides[0].id;

        let conn = state.repository().connection();
        sea_orm::ConnectionTrait::execute(
            conn,
            sea_orm::Statement::from_string(
                sea_orm::ConnectionTrait::get_database_backend(conn),
                "DROP TABLE slide_stage_layouts".to_string(),
            ),
        )
        .await
        .unwrap();

        let result = set_slide_stage_layout(
            State(state),
            Path((presentation.id.to_string(), slide_id.to_string())),
            Json(SlideStageLayoutRequest {
                layout_code: Some("fulltext".to_string()),
            }),
        )
        .await;
        let Err(err) = result else {
            panic!("expected an error from the dropped table, got Ok");
        };
        assert_eq!(
            err.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "a genuine internal failure must be 500, not 400 (#615) and not 404 (#629)"
        );
    }

    /// #629: `presentation_id` is a URL-path resource — a non-existent one
    /// must be a 404, matching the same check's typed refusal everywhere
    /// else in the codebase (`state/slides/edit_ops.rs`, `state/bible.rs`),
    /// never fall through to the router's default 500.
    #[tokio::test]
    async fn put_returns_404_for_unknown_presentation() {
        let (state, _presentation) = seeded_state().await;
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
            StatusCode::NOT_FOUND,
            "a non-existent presentation_id must be 404, not 500 (#629)"
        );
    }

    /// #629: same as above, but for a `slide_id` that doesn't belong to an
    /// otherwise-real presentation.
    #[tokio::test]
    async fn put_returns_404_for_unknown_slide() {
        let (state, presentation) = seeded_state().await;
        let random_slide = presenter_core::SlideId::new();

        let result = set_slide_stage_layout(
            State(state),
            Path((presentation.id.to_string(), random_slide.to_string())),
            Json(SlideStageLayoutRequest {
                layout_code: Some("fulltext".to_string()),
            }),
        )
        .await;
        let Err(err) = result else {
            panic!("expected an error for a non-existent slide, got Ok");
        };
        assert_eq!(
            err.into_response().status(),
            StatusCode::NOT_FOUND,
            "a non-existent slide_id must be 404, not 500 (#629)"
        );
    }

    /// #652 F8: GET and PUT must agree on an unknown presentation — they
    /// name the SAME URL resource. PUT already 404s (see
    /// `put_returns_404_for_unknown_presentation` above); GET currently
    /// returns a misleading 200 `{}` instead.
    #[tokio::test]
    async fn get_returns_404_for_unknown_presentation() {
        let state = crate::state::AppState::in_memory().await.unwrap();
        let random_uuid = uuid::Uuid::new_v4();

        let result = list_slide_stage_layouts(State(state), Path(random_uuid.to_string())).await;
        let Err(err) = result else {
            panic!("expected an error for a non-existent presentation, got Ok");
        };
        assert_eq!(
            err.into_response().status(),
            StatusCode::NOT_FOUND,
            "GET on an unknown presentation must be 404, matching PUT (#652 F8)"
        );
    }
}
