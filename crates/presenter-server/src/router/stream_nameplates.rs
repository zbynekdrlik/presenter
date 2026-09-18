//! Lower-third nameplate REST API (#779, epic #718) — `/stream/api/*`, mounted
//! with one `.merge(stream_nameplates::router())` in `router.rs`.
//!
//! Two halves:
//!  - the plate LIST (config, addressed by output slug or plate id): list /
//!    create / update / delete / reorder. Each write broadcasts
//!    `StreamNameplatesChanged` (NOT `config_revision`) so the editor + Companion
//!    refetch the list.
//!  - SHOW-STATE (which plate is on air): `GET .../nameplates/active` (cold load)
//!    + `PUT .../nameplates/active` (`{source:"person",id}` | `{source:"song"}` |
//!    `{source:null}` = hide), routed through the #779 `NameplateManager` so each
//!    change broadcasts `StreamNameplate` + schedules auto-hide.
//!
//! Typed refusals map to 404/409/422 for free via the central
//! `From<anyhow::Error> for AppError` (a bare `?`), EXCEPT the state-layer
//! `NameplateError::EmptySong` (an empty song plate) → 409, mapped explicitly
//! here per `repository-error-pattern.md` (a state-layer domain error, not a
//! `RepositoryError`).

use super::AppError;
use crate::state::stream_nameplates::NameplateError;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch, put},
    Json, Router,
};
use presenter_core::{ActiveNameplate, Nameplate};
use serde::Deserialize;
use tracing::instrument;

/// The nameplate sub-router (static `/stream/api/*` prefixes only).
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/stream/api/outputs/{slug}/nameplates",
            get(list_nameplates).post(create_nameplate),
        )
        .route(
            "/stream/api/outputs/{slug}/nameplates/order",
            put(reorder_nameplates),
        )
        .route(
            "/stream/api/outputs/{slug}/nameplates/active",
            get(get_active).put(set_active),
        )
        .route(
            "/stream/api/nameplates/{id}",
            patch(patch_nameplate).delete(delete_nameplate),
        )
}

// ---- Request DTOs (camelCase write payloads) ------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateNameplateRequest {
    primary_text: String,
    #[serde(default)]
    secondary_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PatchNameplateRequest {
    primary_text: String,
    #[serde(default)]
    secondary_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReorderNameplatesRequest {
    ids: Vec<i64>,
}

/// `PUT .../nameplates/active`: `{source:"person", id}` shows a person plate,
/// `{source:"song"}` shows the song plate, `{source:null}`/absent hides.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SetActiveNameplateRequest {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    id: Option<i64>,
}

/// Map the state-layer empty-song refusal to 409; everything else routes through
/// the central `From<anyhow::Error> for AppError` (404/409/422/500).
fn map_nameplate_error(err: anyhow::Error) -> AppError {
    if err.downcast_ref::<NameplateError>().is_some() {
        return AppError::conflict(err.to_string());
    }
    AppError::from(err)
}

// ---- List / config handlers -----------------------------------------------

#[instrument(skip_all)]
pub(super) async fn list_nameplates(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<Nameplate>>, AppError> {
    Ok(Json(
        state.repository().list_stream_nameplates(&slug).await?,
    ))
}

#[instrument(skip_all)]
pub(super) async fn create_nameplate(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<CreateNameplateRequest>,
) -> Result<Json<Nameplate>, AppError> {
    let plate = state
        .repository()
        .create_stream_nameplate(&slug, &req.primary_text, &req.secondary_text)
        .await?;
    state.stream_nameplates_changed(&slug);
    Ok(Json(plate))
}

#[instrument(skip_all)]
pub(super) async fn patch_nameplate(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<PatchNameplateRequest>,
) -> Result<Json<Nameplate>, AppError> {
    let plate = state
        .repository()
        .update_stream_nameplate(id, &req.primary_text, &req.secondary_text)
        .await?;
    let slug = state.repository().stream_nameplate_output_slug(id).await?;
    state.stream_nameplates_changed(&slug);
    Ok(Json(plate))
}

#[instrument(skip_all)]
pub(super) async fn delete_nameplate(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    // Resolve the owning output BEFORE the delete — the row is gone afterwards.
    let slug = state.repository().stream_nameplate_output_slug(id).await?;
    state.repository().delete_stream_nameplate(id).await?;
    state.stream_nameplates_changed(&slug);
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn reorder_nameplates(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<ReorderNameplatesRequest>,
) -> Result<StatusCode, AppError> {
    state
        .repository()
        .set_nameplate_order(&slug, req.ids)
        .await?;
    state.stream_nameplates_changed(&slug);
    Ok(StatusCode::NO_CONTENT)
}

// ---- Show-state handlers (route through the NameplateManager) --------------

#[instrument(skip_all)]
pub(super) async fn get_active(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Option<ActiveNameplate>>, AppError> {
    Ok(Json(state.stream_nameplate_active(&slug).await))
}

#[instrument(skip_all)]
pub(super) async fn set_active(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<SetActiveNameplateRequest>,
) -> Result<Json<Option<ActiveNameplate>>, AppError> {
    let active = match req.source.as_deref() {
        None => state
            .stream_nameplate_hide(&slug)
            .await
            .map_err(AppError::from)?,
        Some("song") => state
            .stream_nameplate_show_song(&slug)
            .await
            .map_err(map_nameplate_error)?,
        Some("person") => {
            let id = req
                .id
                .ok_or_else(|| AppError::unprocessable("source \"person\" requires an id"))?;
            state
                .stream_nameplate_show_person(&slug, id)
                .await
                .map_err(map_nameplate_error)?
        }
        Some(other) => {
            return Err(AppError::unprocessable(format!(
                "unknown nameplate source {other:?}"
            )))
        }
    };
    Ok(Json(active))
}
