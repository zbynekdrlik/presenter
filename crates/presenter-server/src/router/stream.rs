//! Stream-graphics REST API — everything under `/stream/api/*` (epic #718,
//! ADR 0009 §5; PR-3 #707). Output/scene/element CRUD + reorder + activation +
//! the full cold-load def, as a nested `Router<AppState>` mounted with one
//! `.merge(stream::router())` line in `router.rs`.
//!
//! Handler shape follows the established `router/playlists.rs` pattern: thin
//! free `async fn(State<AppState>, Path, Json) -> Result<Json<T>, AppError>`.
//! Config-mutating handlers call the repository then
//! [`AppState::stream_config_write_notify`] (publishes `StreamConfigChanged`);
//! activation handlers call the #706 StreamManager methods (which publish
//! `StreamState`). Typed refusals map to 404/409/422 for free via the central
//! `From<anyhow::Error> for AppError` in `router.rs`
//! (`.claude/rules/repository-error-pattern.md`) — a bare `?` is enough.
//!
//! Only static `/stream/api/*` routes live here: the `/stream/{slug}` output
//! page and `/ui/stream` editor land with their WASM lib.rs branches in later
//! PRs, and `/stream/assets` with the asset issue — so no dead route deploys.

use super::AppError;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch, post, put},
    Json, Router,
};
use presenter_core::{
    SceneKind, StreamElementDef, StreamElementProps, StreamOutputDef, StreamOutputSummary,
    StreamSceneDef, StreamShowState,
};
use serde::{Deserialize, Deserializer};
use tracing::instrument;

/// The `/stream/api/*` sub-router, merged into `build_router` before
/// `.with_state`. Static prefixes only (arch §5); slug reservation +
/// static-before-`{slug}` ordering keep it collision-free with the future
/// `/stream/{slug}` page route.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/stream/api/outputs", get(list_outputs).post(create_output))
        .route(
            "/stream/api/outputs/{slug}",
            patch(patch_output).delete(delete_output),
        )
        .route("/stream/api/outputs/{slug}/def", get(get_output_def))
        .route("/stream/api/outputs/{slug}/scenes", post(create_scene))
        .route(
            "/stream/api/outputs/{slug}/scenes/order",
            put(reorder_scenes),
        )
        .route(
            "/stream/api/scenes/{id}",
            patch(patch_scene).delete(delete_scene),
        )
        .route(
            "/stream/api/scenes/{scene_id}/elements",
            post(create_element),
        )
        .route(
            "/stream/api/scenes/{scene_id}/elements/order",
            put(reorder_elements),
        )
        .route(
            "/stream/api/elements/{id}",
            patch(patch_element).delete(delete_element),
        )
        .route(
            "/stream/api/outputs/{slug}/active-scene",
            put(set_active_scene),
        )
        .route(
            "/stream/api/outputs/{slug}/overlays/{scene_id}",
            put(set_overlay),
        )
        .route("/stream/api/outputs/{slug}/clear", post(clear_output))
}

// ---- Request DTOs (WRITE payloads are camelCase, per the repo convention) --

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateOutputRequest {
    slug: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PatchOutputRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    default_transition_ms: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateSceneRequest {
    name: String,
    kind: SceneKind,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PatchSceneRequest {
    #[serde(default)]
    name: Option<String>,
    /// Present (incl. `null`) sets the per-scene transition override — `null`
    /// clears it back to the output default; absent leaves it unchanged. The
    /// double-`Option` distinguishes "absent" from "explicit null".
    #[serde(default, deserialize_with = "deserialize_double_option")]
    transition_ms: Option<Option<u32>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReorderScenesRequest {
    ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReorderElementsRequest {
    ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SetActiveSceneRequest {
    /// The base scene to activate; `null`/absent clears the base (transparent).
    #[serde(default)]
    scene_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SetOverlayRequest {
    active: bool,
}

/// serde double-`Option` deserializer: an ABSENT field is `None`, an explicit
/// `null` is `Some(None)`, a value is `Some(Some(v))` — the only way to tell
/// "leave unchanged" from "clear" for an optional-with-null patch field.
fn deserialize_double_option<'de, D>(deserializer: D) -> Result<Option<Option<u32>>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Some(Option::<u32>::deserialize(deserializer)?))
}

// ---- Output handlers ------------------------------------------------------

#[instrument(skip_all)]
pub(super) async fn list_outputs(
    State(state): State<AppState>,
) -> Result<Json<Vec<StreamOutputSummary>>, AppError> {
    Ok(Json(state.repository().list_stream_outputs().await?))
}

#[instrument(skip_all)]
pub(super) async fn create_output(
    State(state): State<AppState>,
    Json(req): Json<CreateOutputRequest>,
) -> Result<Json<StreamOutputSummary>, AppError> {
    let summary = state
        .repository()
        .create_stream_output(&req.slug, &req.name)
        .await?;
    state.stream_config_write_notify(&summary.slug).await?;
    Ok(Json(summary))
}

#[instrument(skip_all)]
pub(super) async fn patch_output(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<PatchOutputRequest>,
) -> Result<Json<StreamOutputSummary>, AppError> {
    let mut updated = None;
    if let Some(name) = req.name.as_deref() {
        updated = Some(state.repository().rename_stream_output(&slug, name).await?);
    }
    if let Some(ms) = req.default_transition_ms {
        updated = Some(
            state
                .repository()
                .set_stream_output_transition(&slug, ms)
                .await?,
        );
    }
    let summary = updated.ok_or_else(|| {
        AppError::bad_request_message("patch requires name or defaultTransitionMs")
    })?;
    state.stream_config_write_notify(&slug).await?;
    Ok(Json(summary))
}

#[instrument(skip_all)]
pub(super) async fn delete_output(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<StatusCode, AppError> {
    state.repository().delete_stream_output(&slug).await?;
    // Drop any cached show-state so a later read can't serve a stale snapshot.
    state.stream_evict_output(&slug).await;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn get_output_def(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<StreamOutputDef>, AppError> {
    Ok(Json(state.repository().load_output_def(&slug).await?))
}

// ---- Scene handlers -------------------------------------------------------

#[instrument(skip_all)]
pub(super) async fn create_scene(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<CreateSceneRequest>,
) -> Result<Json<StreamSceneDef>, AppError> {
    let scene = state
        .repository()
        .create_stream_scene(&slug, &req.name, req.kind)
        .await?;
    state.stream_config_write_notify(&slug).await?;
    Ok(Json(scene))
}

#[instrument(skip_all)]
pub(super) async fn patch_scene(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<PatchSceneRequest>,
) -> Result<Json<StreamSceneDef>, AppError> {
    let slug = state.repository().stream_scene_output_slug(id).await?;
    let mut updated = None;
    if let Some(name) = req.name.as_deref() {
        updated = Some(state.repository().rename_stream_scene(id, name).await?);
    }
    if let Some(transition_ms) = req.transition_ms {
        updated = Some(
            state
                .repository()
                .set_stream_scene_transition(id, transition_ms)
                .await?,
        );
    }
    let scene = updated
        .ok_or_else(|| AppError::bad_request_message("patch requires name or transitionMs"))?;
    state.stream_config_write_notify(&slug).await?;
    Ok(Json(scene))
}

#[instrument(skip_all)]
pub(super) async fn delete_scene(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    // Resolve the owning output BEFORE the delete — the row is gone afterwards.
    let slug = state.repository().stream_scene_output_slug(id).await?;
    state.repository().delete_stream_scene(id).await?;
    state.stream_config_write_notify(&slug).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn reorder_scenes(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<ReorderScenesRequest>,
) -> Result<StatusCode, AppError> {
    state.repository().set_scene_order(&slug, req.ids).await?;
    state.stream_config_write_notify(&slug).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Element handlers -----------------------------------------------------

#[instrument(skip_all)]
pub(super) async fn create_element(
    State(state): State<AppState>,
    Path(scene_id): Path<i64>,
    Json(props): Json<StreamElementProps>,
) -> Result<Json<StreamElementDef>, AppError> {
    let element = state
        .repository()
        .create_stream_element(scene_id, props)
        .await?;
    let slug = state
        .repository()
        .stream_scene_output_slug(scene_id)
        .await?;
    state.stream_config_write_notify(&slug).await?;
    Ok(Json(element))
}

#[instrument(skip_all)]
pub(super) async fn patch_element(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(props): Json<StreamElementProps>,
) -> Result<Json<StreamElementDef>, AppError> {
    let element = state.repository().update_stream_element(id, props).await?;
    let slug = state.repository().stream_element_output_slug(id).await?;
    state.stream_config_write_notify(&slug).await?;
    Ok(Json(element))
}

#[instrument(skip_all)]
pub(super) async fn delete_element(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    // Resolve the owning output BEFORE the delete — the row is gone afterwards.
    let slug = state.repository().stream_element_output_slug(id).await?;
    state.repository().delete_stream_element(id).await?;
    state.stream_config_write_notify(&slug).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn reorder_elements(
    State(state): State<AppState>,
    Path(scene_id): Path<i64>,
    Json(req): Json<ReorderElementsRequest>,
) -> Result<StatusCode, AppError> {
    state
        .repository()
        .set_element_order(scene_id, req.ids)
        .await?;
    let slug = state
        .repository()
        .stream_scene_output_slug(scene_id)
        .await?;
    state.stream_config_write_notify(&slug).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Activation handlers (route through the #706 StreamManager) ------------

#[instrument(skip_all)]
pub(super) async fn set_active_scene(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<SetActiveSceneRequest>,
) -> Result<Json<StreamShowState>, AppError> {
    Ok(Json(
        state.stream_activate_scene(&slug, req.scene_id).await?,
    ))
}

#[instrument(skip_all)]
pub(super) async fn set_overlay(
    State(state): State<AppState>,
    Path((slug, scene_id)): Path<(String, i64)>,
    Json(req): Json<SetOverlayRequest>,
) -> Result<Json<StreamShowState>, AppError> {
    Ok(Json(
        state
            .stream_set_overlay(&slug, scene_id, req.active)
            .await?,
    ))
}

#[instrument(skip_all)]
pub(super) async fn clear_output(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<StreamShowState>, AppError> {
    Ok(Json(state.stream_clear(&slug).await?))
}
