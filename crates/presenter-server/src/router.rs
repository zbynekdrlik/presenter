mod bible;
mod libraries;
mod playlists;
mod presentations;
use crate::{
    ableset::AbleSetStatusSnapshot, android_stage::AndroidStageDisplayStatusSnapshot,
    osc::OscStatusSnapshot, resolume::ResolumeConnectionSnapshot,
    stage_connections::StageClientSnapshot, stage_ui, state::AppState, ui,
};
use anyhow::Error as AnyhowError;
use axum::{
    extract::{ws::WebSocketUpgrade, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use presenter_core::{
    playlist::{MidiBinding, PlaylistEntryKind},
    AbleSetSettings, AbleSetSettingsDraft, AndroidStageDisplay, AndroidStageDisplayDraft,
    AndroidStageDisplayId, BiblePassage, BibleReference, BibleTranslation, Library, LibraryId,
    LibrarySummary, OscSettings, OscSettingsDraft, Playlist, PlaylistEntry, PlaylistEntryId,
    PlaylistId, Presentation, PresentationId, ResolumeHost, ResolumeHostDraft, ResolumeHostId,
    SearchResult, Slide, SlideId, StageDisplayLayout, StageDisplaySnapshot, TimersOverview,
    VelocityMode, DEFAULT_ADB_PORT, DEFAULT_LAUNCH_COMPONENT,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;
use uuid::Uuid;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/", get(home))
        .route("/search", get(search_presenter_endpoint))
        .route("/libraries/summary", get(libraries::list_library_summaries))
        .route("/libraries", get(libraries::list_libraries).post(libraries::create_library))
        .route(
            "/libraries/{id}",
            patch(libraries::rename_library).delete(libraries::delete_library),
        )
        .route("/libraries/{id}/favorite", post(libraries::set_library_favorite))
        .route(
            "/libraries/{id}/presentations",
            post(libraries::create_library_presentation),
        )
        .route("/bible/translations", get(bible::list_bible_translations))
        .route("/bible/search", get(bible::search_bible_passages))
        .route("/bible/passage", get(bible::get_bible_passage))
        .route(
            "/bible/translations/refresh",
            post(bible::refresh_bible_translations),
        )
        .route("/bible/active", get(bible::get_active_bible_broadcast))
        .route("/bible/trigger", post(bible::trigger_bible_broadcast))
        .route("/bible/clear", post(bible::clear_bible_broadcast))
        .route("/playlists", get(playlists::list_playlists).post(playlists::create_playlist))
        .route(
            "/playlists/{id}",
            patch(playlists::update_playlist).delete(playlists::delete_playlist),
        )
        .route("/playlists/{id}/entries", put(playlists::replace_playlist_entries))
        .route("/ui/operator", get(operator_ui))
        .route("/ui/tablet", get(tablet_ui))
        .route("/ui/bible", get(bible::bible_ui))
        .route("/ui/settings", get(settings_ui))
        .route("/overlays/timer", get(timer_overlay))
        .route("/stage-displays", get(list_stage_displays))
        .route(
            "/stage/layout",
            get(get_stage_layout).post(set_stage_layout),
        )
        .route("/stage/connections", get(list_stage_connections))
        .route("/stage", get(stage_display_selected_html))
        .route("/stage/snapshot", get(stage_display_selected_snapshot_json))
        .route("/stage/state", post(update_stage_state))
        .route("/stage/clear", post(clear_stage_state))
        .route(
            "/integrations/resolume/hosts",
            get(list_resolume_hosts).post(create_resolume_host),
        )
        .route(
            "/integrations/resolume/hosts/{id}",
            put(update_resolume_host).delete(delete_resolume_host),
        )
        .route(
            "/integrations/android-stage/displays",
            get(list_android_stage_displays).post(create_android_stage_display),
        )
        .route(
            "/integrations/android-stage/displays/{id}",
            put(update_android_stage_display).delete(delete_android_stage_display),
        )
        .route(
            "/integrations/osc/settings",
            get(get_osc_settings).put(update_osc_settings),
        )
        .route("/integrations/osc/status", get(get_osc_status))
        .route(
            "/integrations/ableset/settings",
            get(get_ableset_settings).put(update_ableset_settings),
        )
        .route("/integrations/ableset/status", get(get_ableset_status))
        .route("/integrations/ableset/follow", post(set_ableset_follow))
        .route(
            "/presentations/{id}",
            get(presentations::get_presentation_detail).patch(presentations::update_presentation),
        )
        .route(
            "/presentations/{presentation_id}/slides",
            post(presentations::insert_slide),
        )
        .route(
            "/presentations/{presentation_id}/slides/{slide_id}/duplicate",
            post(presentations::duplicate_slide),
        )
        .route(
            "/presentations/{presentation_id}/slides/{slide_id}",
            patch(presentations::update_slide_content).delete(presentations::delete_slide),
        )
        .route(
            "/presentations/{presentation_id}/slides/reorder",
            post(presentations::reorder_slides),
        )
        .route("/timers/overview", get(get_timers_overview))
        .route("/timers/command", post(execute_timer_command))
        .route("/live/ws", get(live_websocket))
        .route(
            "/settings/features",
            get(get_feature_settings).post(update_feature_settings),
        )
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

#[instrument(skip_all)]
async fn home(State(_state): State<AppState>) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_home_ui().await?;
    Ok(html)
}

#[instrument(skip_all)]
async fn timer_overlay(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_timer_overlay(&state).await?;
    Ok(html)
}


#[derive(Debug, Deserialize)]
struct SearchQueryParams {
    #[serde(default, alias = "q", alias = "query")]
    query: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct AbleSetFollowPayload {
    enabled: bool,
}


#[instrument(skip_all)]
async fn search_presenter_endpoint(
    State(state): State<AppState>,
    Query(params): Query<SearchQueryParams>,
) -> Result<Json<Vec<SearchResult>>, AppError> {
    let query = params.query.unwrap_or_default();
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let limit = params.limit.unwrap_or(25).clamp(1, 100) as u64;
    let results = state.search_presenter(trimmed, limit).await?;
    Ok(Json(results))
}








#[instrument(skip_all)]
async fn list_playlists(State(state): State<AppState>) -> Result<Json<Vec<Playlist>>, AppError> {
    let playlists = state.playlists().await?;
    Ok(Json(playlists))
}

#[instrument(skip_all)]
async fn create_playlist(
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
    Ok(Json(playlist))
}

#[instrument(skip_all)]
async fn update_playlist(
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
        state.set_playlist_favorite(playlist_id, favorite).await?;
    }

    let updated = state
        .playlists()
        .await?
        .into_iter()
        .find(|playlist| playlist.id == playlist_id)
        .ok_or_else(|| AppError::not_found("playlist not found"))?;

    Ok(Json(updated))
}

#[instrument(skip_all)]
async fn delete_playlist(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    state.delete_playlist(PlaylistId::from_uuid(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
async fn replace_playlist_entries(
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
    let playlist = state
        .replace_playlist_entries(PlaylistId::from_uuid(id), entries)
        .await?;
    Ok(Json(playlist))
}


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePlaylistRequest {
    name: String,
    #[serde(default)]
    show_in_dashboard: bool,
}



#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePlaylistRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    show_in_dashboard: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePlaylistEntriesRequest {
    entries: Vec<PlaylistEntryPayload>,
}


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenamePresentationRequest {
    name: String,
}


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
enum PlaylistEntryPayload {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSlideRequest {
    position: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderSlidesRequest {
    slide_ids: Vec<Uuid>,
}

#[instrument(skip_all)]
async fn trigger_bible_broadcast(
    State(state): State<AppState>,
    Json(payload): Json<bible::BibleTriggerRequest>,
) -> Result<Json<presenter_core::BibleBroadcast>, AppError> {
    bible::trigger_bible_broadcast(State(state), Json(payload)).await
}

#[instrument(skip_all)]
async fn clear_bible_broadcast(State(state): State<AppState>) -> Result<StatusCode, AppError> {
    bible::clear_bible_broadcast(State(state)).await
}

#[instrument(skip_all)]
async fn clear_stage_state(State(state): State<AppState>) -> Result<StatusCode, AppError> {
    state.clear_stage().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
async fn list_resolume_hosts(
    State(state): State<AppState>,
) -> Result<Json<Vec<ResolumeHostDto>>, AppError> {
    let hosts = state.list_resolume_hosts().await?;
    let statuses = state.resolume_status_snapshot().await;
    let payload = hosts
        .into_iter()
        .map(|host| {
            let status = statuses
                .get(&host.id)
                .cloned()
                .unwrap_or_else(ResolumeConnectionSnapshot::disabled);
            ResolumeHostDto::from_host(host, status)
        })
        .collect::<Vec<_>>();
    Ok(Json(payload))
}

#[instrument(skip_all)]
async fn create_resolume_host(
    State(state): State<AppState>,
    Json(payload): Json<ResolumeHostRequest>,
) -> Result<Json<ResolumeHostDto>, AppError> {
    let draft = ResolumeHostDraft::new(payload.label, payload.host, payload.port)
        .with_enabled(payload.is_enabled);
    let host = state.create_resolume_host(draft).await?;
    let status = state.resolume_status_for(host.id).await;
    Ok(Json(ResolumeHostDto::from_host(host, status)))
}

#[instrument(skip_all)]
async fn update_resolume_host(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<ResolumeHostRequest>,
) -> Result<Json<ResolumeHostDto>, AppError> {
    let draft = ResolumeHostDraft::new(payload.label, payload.host, payload.port)
        .with_enabled(payload.is_enabled);
    let host = state
        .update_resolume_host(ResolumeHostId::from_uuid(id), draft)
        .await?;
    let status = state.resolume_status_for(host.id).await;
    Ok(Json(ResolumeHostDto::from_host(host, status)))
}

#[instrument(skip_all)]
async fn delete_resolume_host(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    state
        .delete_resolume_host(ResolumeHostId::from_uuid(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
async fn list_android_stage_displays(
    State(state): State<AppState>,
) -> Result<Json<Vec<AndroidStageDisplayDto>>, AppError> {
    let displays = state.list_android_stage_displays().await?;
    let statuses = state.android_stage_status_snapshot().await;
    let payload = displays
        .into_iter()
        .map(|display| {
            let status = statuses
                .get(&display.id)
                .cloned()
                .unwrap_or_else(AndroidStageDisplayStatusSnapshot::disabled);
            AndroidStageDisplayDto::from_display(display, status)
        })
        .collect();
    Ok(Json(payload))
}

#[instrument(skip_all)]
async fn create_android_stage_display(
    State(state): State<AppState>,
    Json(payload): Json<AndroidStageDisplayRequest>,
) -> Result<Json<AndroidStageDisplayDto>, AppError> {
    let draft = AndroidStageDisplayDraft::new(payload.label, payload.host)
        .with_port(payload.port)
        .with_launch_component(normalize_launch_component(&payload.launch_component))
        .with_enabled(payload.is_enabled);
    let display = state.create_android_stage_display(draft).await?;
    let status = state.android_stage_status_for(display.id).await;
    Ok(Json(AndroidStageDisplayDto::from_display(display, status)))
}

#[instrument(skip_all)]
async fn update_android_stage_display(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<AndroidStageDisplayRequest>,
) -> Result<Json<AndroidStageDisplayDto>, AppError> {
    let draft = AndroidStageDisplayDraft::new(payload.label, payload.host)
        .with_port(payload.port)
        .with_launch_component(normalize_launch_component(&payload.launch_component))
        .with_enabled(payload.is_enabled);
    let display = state
        .update_android_stage_display(AndroidStageDisplayId::from_uuid(id), draft)
        .await?;
    let status = state.android_stage_status_for(display.id).await;
    Ok(Json(AndroidStageDisplayDto::from_display(display, status)))
}

#[instrument(skip_all)]
async fn delete_android_stage_display(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    state
        .delete_android_stage_display(AndroidStageDisplayId::from_uuid(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
async fn get_osc_settings(
    State(state): State<AppState>,
) -> Result<Json<OscSettingsResponse>, AppError> {
    let settings = state.osc_settings().await?;
    Ok(Json(OscSettingsResponse::from(settings)))
}

#[instrument(skip_all)]
async fn update_osc_settings(
    State(state): State<AppState>,
    Json(payload): Json<UpdateOscSettingsRequest>,
) -> Result<Json<OscSettingsResponse>, AppError> {
    if payload.address_pattern.trim().is_empty() {
        return Err(AppError::bad_request_message(
            "address pattern cannot be empty",
        ));
    }
    if payload.listen_port == 0 {
        return Err(AppError::bad_request_message(
            "listener port must be between 1 and 65535",
        ));
    }
    let draft = OscSettingsDraft {
        enabled: payload.enabled,
        listen_port: payload.listen_port,
        address_pattern: payload.address_pattern.trim().to_string(),
        velocity_mode: payload.velocity_mode,
    };
    let settings = state
        .update_osc_settings(draft)
        .await
        .map_err(|err| AppError::bad_request_message(err.to_string()))?;
    Ok(Json(OscSettingsResponse::from(settings)))
}

#[instrument(skip_all)]
async fn get_osc_status(
    State(state): State<AppState>,
) -> Result<Json<OscStatusSnapshot>, AppError> {
    Ok(Json(state.osc_status_snapshot().await))
}

#[instrument(skip_all)]
async fn get_ableset_settings(
    State(state): State<AppState>,
) -> Result<Json<AbleSetSettings>, AppError> {
    let settings = state.ableset_settings().await?;
    Ok(Json(settings))
}

#[instrument(skip_all)]
async fn update_ableset_settings(
    State(state): State<AppState>,
    Json(payload): Json<AbleSetSettingsDraft>,
) -> Result<Json<AbleSetSettings>, AppError> {
    let settings = state
        .update_ableset_settings(payload)
        .await
        .map_err(|err| AppError::bad_request_message(err.to_string()))?;
    Ok(Json(settings))
}

#[instrument(skip_all)]
async fn get_ableset_status(
    State(state): State<AppState>,
) -> Result<Json<AbleSetStatusSnapshot>, AppError> {
    Ok(Json(state.ableset_status_snapshot().await))
}

#[instrument(skip_all)]
async fn set_ableset_follow(
    State(state): State<AppState>,
    Json(payload): Json<AbleSetFollowPayload>,
) -> Result<Json<AbleSetStatusSnapshot>, AppError> {
    let snapshot = state.set_ableset_follow(payload.enabled).await;
    Ok(Json(snapshot))
}

#[instrument(skip_all)]
async fn operator_ui(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_operator_ui(&state).await?;
    Ok(html)
}

#[instrument(skip_all)]
async fn settings_ui(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_settings_ui(&state).await?;
    Ok(html)
}

async fn tablet_ui(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_tablet_ui(&state).await?;
    Ok(html)
}

#[instrument(skip_all)]
async fn bible_ui(State(state): State<AppState>) -> Result<axum::response::Html<String>, AppError> {
    bible::bible_ui(State(state)).await
}

#[instrument(skip_all)]
async fn stage_display_selected_html(State(state): State<AppState>) -> Result<Response, AppError> {
    match state.selected_stage_display_snapshot().await? {
        Some(snapshot) => {
            Ok(stage_ui::render_stage_display(snapshot, state.heartbeat_config()).into_response())
        }
        None => Ok((StatusCode::SERVICE_UNAVAILABLE, "Stage display unavailable").into_response()),
    }
}

#[instrument(skip_all)]
async fn stage_display_selected_snapshot_json(
    State(state): State<AppState>,
) -> Result<Json<StageDisplaySnapshot>, AppError> {
    match state.selected_stage_display_snapshot().await? {
        Some(snapshot) => Ok(Json(snapshot)),
        None => Err(AppError::not_found("Stage display unavailable")),
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StageLayoutResponse {
    code: String,
    layout: StageDisplayLayout,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StageLayoutUpdateRequest {
    code: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeatureSettingsResponse {
    companion_enabled: bool,
    companion_port: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeatureSettingsRequest {
    #[serde(alias = "enabled", alias = "companion_enabled")]
    companion_enabled: bool,
    #[serde(default, alias = "companion_port", alias = "port")]
    companion_port: Option<u16>,
}

#[instrument(skip_all)]
async fn get_stage_layout(
    State(state): State<AppState>,
) -> Result<Json<StageLayoutResponse>, AppError> {
    let code = state.stage_layout_code().await;
    let layouts = state.stage_displays().await?;
    let layout = layouts
        .into_iter()
        .find(|layout| layout.code == code)
        .unwrap_or_else(|| {
            StageDisplayLayout::built_in()
                .into_iter()
                .next()
                .expect("stage layouts")
        });
    Ok(Json(StageLayoutResponse {
        code: layout.code.clone(),
        layout,
    }))
}

#[instrument(skip_all)]
async fn set_stage_layout(
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
        .map_err(|err| AppError::not_found(err.to_string()))?;
    Ok(Json(StageLayoutResponse {
        code: layout.code.clone(),
        layout,
    }))
}

#[instrument(skip_all)]
async fn get_feature_settings(
    State(state): State<AppState>,
) -> Result<Json<FeatureSettingsResponse>, AppError> {
    Ok(Json(FeatureSettingsResponse {
        companion_enabled: state.companion_enabled(),
        companion_port: state.companion_port(),
    }))
}

#[instrument(skip_all)]
async fn update_feature_settings(
    State(state): State<AppState>,
    Json(payload): Json<FeatureSettingsRequest>,
) -> Result<Json<FeatureSettingsResponse>, AppError> {
    let requested_port = payload
        .companion_port
        .unwrap_or_else(|| state.companion_port());
    if requested_port == 0 {
        return Err(AppError::bad_request_message(
            "companionPort must be between 1 and 65535",
        ));
    }

    state
        .set_companion_settings(payload.companion_enabled, requested_port)
        .await?;
    Ok(Json(FeatureSettingsResponse {
        companion_enabled: state.companion_enabled(),
        companion_port: state.companion_port(),
    }))
}

#[instrument(skip_all)]
async fn get_presentation_detail(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PresentationDetailDto>, AppError> {
    let uuid = Uuid::parse_str(&id)
        .map_err(|_| AppError::bad_request_message("presentationId must be a valid UUID"))?;
    let presentation_id = PresentationId::from_uuid(uuid);
    match state.presentation_detail(presentation_id).await? {
        Some((library_id, library_name, presentation)) => Ok(Json(PresentationDetailDto {
            library_id,
            library_name,
            presentation,
        })),
        None => Err(AppError::not_found(format!(
            "presentation {} not found",
            id
        ))),
    }
}

#[instrument(skip_all)]
async fn update_presentation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<RenamePresentationRequest>,
) -> Result<StatusCode, AppError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request_message("name cannot be empty"));
    }
    let presentation_uuid = parse_uuid("presentationId", &id)?;
    state
        .rename_presentation(PresentationId::from_uuid(presentation_uuid), name)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
async fn insert_slide_handler(
    State(state): State<AppState>,
    Path(presentation_id): Path<String>,
    Json(payload): Json<CreateSlideRequest>,
) -> Result<Json<Vec<Slide>>, AppError> {
    let presentation_uuid = parse_uuid("presentationId", &presentation_id)?;
    let slides = state
        .insert_blank_slide(
            PresentationId::from_uuid(presentation_uuid),
            payload.position,
        )
        .await?;
    Ok(Json(slides))
}

#[instrument(skip_all)]
async fn duplicate_slide_handler(
    State(state): State<AppState>,
    Path((presentation_id, slide_id)): Path<(String, String)>,
) -> Result<Json<Vec<Slide>>, AppError> {
    let presentation_uuid = parse_uuid("presentationId", &presentation_id)?;
    let slide_uuid = parse_uuid("slideId", &slide_id)?;
    let slides = state
        .duplicate_slide(
            PresentationId::from_uuid(presentation_uuid),
            SlideId::from_uuid(slide_uuid),
        )
        .await?;
    Ok(Json(slides))
}

#[instrument(skip_all)]
async fn delete_slide_handler(
    State(state): State<AppState>,
    Path((presentation_id, slide_id)): Path<(String, String)>,
) -> Result<Json<Vec<Slide>>, AppError> {
    let presentation_uuid = parse_uuid("presentationId", &presentation_id)?;
    let slide_uuid = parse_uuid("slideId", &slide_id)?;
    let slides = state
        .delete_slide(
            PresentationId::from_uuid(presentation_uuid),
            SlideId::from_uuid(slide_uuid),
        )
        .await?;
    Ok(Json(slides))
}

#[instrument(skip_all)]
async fn reorder_slides_handler(
    State(state): State<AppState>,
    Path(presentation_id): Path<String>,
    Json(payload): Json<ReorderSlidesRequest>,
) -> Result<Json<Vec<Slide>>, AppError> {
    let presentation_uuid = parse_uuid("presentationId", &presentation_id)?;
    let order = payload
        .slide_ids
        .into_iter()
        .map(SlideId::from_uuid)
        .collect();
    let slides = state
        .reorder_slides(PresentationId::from_uuid(presentation_uuid), order)
        .await?;
    Ok(Json(slides))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlideContentUpdateRequest {
    main: String,
    translation: String,
    stage: String,
    #[serde(default)]
    group: Option<String>,
}

#[instrument(skip_all)]
async fn update_slide_content_handler(
    State(state): State<AppState>,
    Path((presentation_id, slide_id)): Path<(String, String)>,
    Json(payload): Json<SlideContentUpdateRequest>,
) -> Result<Json<Slide>, AppError> {
    let presentation_uuid = parse_uuid("presentationId", &presentation_id)?;
    let slide_uuid = parse_uuid("slideId", &slide_id)?;
    let updated = state
        .update_slide_content(
            PresentationId::from_uuid(presentation_uuid),
            SlideId::from_uuid(slide_uuid),
            payload.main,
            payload.translation,
            payload.stage,
            payload.group,
        )
        .await?;
    Ok(Json(updated))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OscSettingsResponse {
    enabled: bool,
    listen_port: u16,
    address_pattern: String,
    velocity_mode: VelocityMode,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<OscSettings> for OscSettingsResponse {
    fn from(settings: OscSettings) -> Self {
        Self {
            enabled: settings.enabled,
            listen_port: settings.listen_port,
            address_pattern: settings.address_pattern,
            velocity_mode: settings.velocity_mode,
            created_at: settings.created_at,
            updated_at: settings.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateOscSettingsRequest {
    enabled: bool,
    listen_port: u16,
    address_pattern: String,
    velocity_mode: VelocityMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StageStateRequest {
    presentation_id: String,
    current_slide_id: String,
    #[serde(default)]
    next_slide_id: Option<String>,
}

#[instrument(skip_all)]
async fn update_stage_state(
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

    state
        .update_stage_state(presentation_id, current_slide_id, next_slide_id)
        .await
        .map_err(|err| AppError::bad_request(err))?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
async fn list_stage_connections(
    State(state): State<AppState>,
) -> Result<Json<Vec<StageClientSnapshot>>, AppError> {
    let snapshot = state.stage_connections_snapshot().await;
    Ok(Json(snapshot))
}

#[instrument(skip_all)]
async fn list_stage_displays(
    State(state): State<AppState>,
) -> Result<Json<Vec<StageDisplayLayout>>, AppError> {
    let displays = state.stage_displays().await?;
    Ok(Json(displays))
}

#[instrument(skip_all)]
async fn get_timers_overview(
    State(state): State<AppState>,
) -> Result<Json<TimersOverview>, AppError> {
    let overview = state.timers_overview().await?;
    Ok(Json(overview))
}

#[instrument(skip_all)]
async fn execute_timer_command(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<Json<TimersOverview>, AppError> {
    let command: presenter_core::TimerCommand =
        serde_json::from_value(payload).map_err(AnyhowError::new)?;
    match state.execute_timer_command(command).await {
        Ok(overview) => Ok(Json(overview)),
        Err(err) => {
            if let Some(timer_err) = err.downcast_ref::<presenter_core::timer::TimerError>() {
                return Err(AppError::bad_request_message(timer_err.to_string()));
            }
            Err(err.into())
        }
    }
}

#[instrument(skip_all)]
async fn live_websocket(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let hub = state.live_hub();
    let connections = state.stage_connections_handle();
    ws.on_upgrade(move |socket| async move {
        crate::live::serve_websocket(hub, connections, socket).await;
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct BibleImportSummaryDto {
    translation_code: String,
    passage_count: usize,
}

impl From<presenter_bible::BibleImportSummary> for BibleImportSummaryDto {
    fn from(summary: presenter_bible::BibleImportSummary) -> Self {
        Self {
            translation_code: summary.translation_code,
            passage_count: summary.passage_count,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresentationDetailDto {
    library_id: LibraryId,
    library_name: String,
    presentation: Presentation,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolumeHostDto {
    id: ResolumeHostId,
    label: String,
    host: String,
    port: u16,
    is_enabled: bool,
    created_at: String,
    updated_at: String,
    status: ResolumeConnectionSnapshot,
}

impl ResolumeHostDto {
    fn from_host(host: ResolumeHost, status: ResolumeConnectionSnapshot) -> Self {
        Self {
            id: host.id,
            label: host.label,
            host: host.host,
            port: host.port,
            is_enabled: host.is_enabled,
            created_at: host.created_at.to_rfc3339(),
            updated_at: host.updated_at.to_rfc3339(),
            status,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolumeHostRequest {
    label: String,
    host: String,
    #[serde(default = "default_resolume_port")]
    port: u16,
    #[serde(default = "default_true")]
    is_enabled: bool,
}

const fn default_resolume_port() -> u16 {
    8090
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AndroidStageDisplayDto {
    id: AndroidStageDisplayId,
    label: String,
    host: String,
    port: u16,
    launch_component: String,
    is_enabled: bool,
    created_at: String,
    updated_at: String,
    status: AndroidStageDisplayStatusSnapshot,
}

impl AndroidStageDisplayDto {
    fn from_display(
        display: AndroidStageDisplay,
        status: AndroidStageDisplayStatusSnapshot,
    ) -> Self {
        Self {
            id: display.id,
            label: display.label,
            host: display.host,
            port: display.port,
            launch_component: display.launch_component,
            is_enabled: display.is_enabled,
            created_at: display.created_at.to_rfc3339(),
            updated_at: display.updated_at.to_rfc3339(),
            status,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidStageDisplayRequest {
    label: String,
    host: String,
    #[serde(default = "default_android_stage_port")]
    port: u16,
    #[serde(default = "default_android_stage_launch_component")]
    launch_component: String,
    #[serde(default = "default_true")]
    is_enabled: bool,
}

const fn default_android_stage_port() -> u16 {
    DEFAULT_ADB_PORT
}

fn default_android_stage_launch_component() -> String {
    DEFAULT_LAUNCH_COMPONENT.to_string()
}

fn normalize_launch_component(component: &str) -> String {
    let trimmed = component.trim();
    if trimmed.is_empty() {
        DEFAULT_LAUNCH_COMPONENT.to_string()
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug)]
struct AppError {
    status: StatusCode,
    error: anyhow::Error,
}

impl AppError {
    fn new(status: StatusCode, error: anyhow::Error) -> Self {
        Self { status, error }
    }

    fn bad_request<E>(error: E) -> Self
    where
        E: Into<anyhow::Error>,
    {
        Self::new(StatusCode::BAD_REQUEST, error.into())
    }

    fn bad_request_message(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, anyhow::anyhow!(message.into()))
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, anyhow::anyhow!(message.into()))
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, err)
    }
}

#[derive(Serialize)]
struct ErrorBody {
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = Json(ErrorBody {
            message: self.error.to_string(),
        });
        (self.status, body).into_response()
    }
}

fn parse_uuid(field: &str, value: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value)
        .map_err(|_| AppError::bad_request_message(format!("{field} must be a valid UUID")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::{Duration as ChronoDuration, Utc};
    use presenter_core::{SearchResult, SearchResultKind, TimerState};
    use serde::Deserialize;
    use serde_json::json;
    use tower::ServiceExt;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TestResolumeHostDto {
        id: String,
        label: String,
        host: String,
        port: u16,
        is_enabled: bool,
        status: TestHostStatus,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TestHostStatus {
        state: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TestAndroidDisplayDto {
        id: String,
        label: String,
        host: String,
        port: u16,
        launch_component: String,
        is_enabled: bool,
    }

    #[tokio::test]
    async fn health_endpoint_returns_ok() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn home_route_returns_menu() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let response = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("Presenter Demo Environment"));
        assert!(body.contains("Operator UI"));
        assert!(body.contains("Tablet UI"));
        assert!(body.contains("Bible Control"));
    }

    #[tokio::test]
    async fn resolume_host_endpoints_crud() {
        let app = build_router(AppState::in_memory().await.unwrap());

        let list_empty = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/integrations/resolume/hosts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list_empty.status(), StatusCode::OK);
        let empty_bytes = axum::body::to_bytes(list_empty.into_body(), usize::MAX)
            .await
            .unwrap();
        let empty_hosts: Vec<TestResolumeHostDto> = serde_json::from_slice(&empty_bytes).unwrap();
        assert!(empty_hosts.is_empty());

        let create_body = json!({
            "label": "Arena",
            "host": "resolume.lan",
            "port": 8090,
            "isEnabled": true
        })
        .to_string();
        let created_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/integrations/resolume/hosts")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created_response.status(), StatusCode::OK);
        let created_bytes = axum::body::to_bytes(created_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: TestResolumeHostDto = serde_json::from_slice(&created_bytes).unwrap();
        assert_eq!(created.label, "Arena");
        assert_eq!(created.host, "resolume.lan");
        assert_eq!(created.port, 8090);
        assert!(created.is_enabled);
        assert!(!created.status.state.is_empty());

        let list_after_create = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/integrations/resolume/hosts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_bytes = axum::body::to_bytes(list_after_create.into_body(), usize::MAX)
            .await
            .unwrap();
        let hosts: Vec<TestResolumeHostDto> = serde_json::from_slice(&list_bytes).unwrap();
        assert_eq!(hosts.len(), 1);
        assert!(!hosts[0].status.state.is_empty());

        let update_body = json!({
            "label": "Arena North",
            "host": "resolume.lan",
            "port": 8090,
            "isEnabled": false
        })
        .to_string();
        let update_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::PUT)
                    .uri(format!("/integrations/resolume/hosts/{}", created.id))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(update_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update_response.status(), StatusCode::OK);
        let updated_bytes = axum::body::to_bytes(update_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let updated: TestResolumeHostDto = serde_json::from_slice(&updated_bytes).unwrap();
        assert_eq!(updated.label, "Arena North");
        assert!(!updated.is_enabled);
        assert_eq!(updated.host, "resolume.lan");
        assert!(!updated.status.state.is_empty());

        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::DELETE)
                    .uri(format!("/integrations/resolume/hosts/{}", updated.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let list_after_delete = app
            .oneshot(
                Request::builder()
                    .uri("/integrations/resolume/hosts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_after_delete_bytes =
            axum::body::to_bytes(list_after_delete.into_body(), usize::MAX)
                .await
                .unwrap();
        let after_delete_hosts: Vec<TestResolumeHostDto> =
            serde_json::from_slice(&list_after_delete_bytes).unwrap();
        assert!(after_delete_hosts.is_empty());
    }

    #[tokio::test]
    async fn android_stage_display_endpoints_crud() {
        std::env::set_var("PRESENTER_ANDROID_ADB_BIN", "true");
        let app = build_router(AppState::in_memory().await.unwrap());

        let list_empty = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/integrations/android-stage/displays")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list_empty.status(), StatusCode::OK);
        let empty_bytes = axum::body::to_bytes(list_empty.into_body(), usize::MAX)
            .await
            .unwrap();
        let empty_displays: Vec<TestAndroidDisplayDto> =
            serde_json::from_slice(&empty_bytes).unwrap();
        assert!(empty_displays.is_empty());

        let create_body = json!({
            "label": "Stage Left",
            "host": "sd1l.lan",
            "port": 5555,
            "launchComponent": "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity",
            "isEnabled": true
        })
        .to_string();
        let created_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/integrations/android-stage/displays")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created_response.status(), StatusCode::OK);
        let created_bytes = axum::body::to_bytes(created_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: TestAndroidDisplayDto = serde_json::from_slice(&created_bytes).unwrap();
        assert_eq!(created.label, "Stage Left");
        assert_eq!(created.host, "sd1l.lan");
        assert_eq!(created.port, 5555);
        assert_eq!(
            created.launch_component,
            "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity"
        );

        let list_after_create = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/integrations/android-stage/displays")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_bytes = axum::body::to_bytes(list_after_create.into_body(), usize::MAX)
            .await
            .unwrap();
        let displays: Vec<TestAndroidDisplayDto> = serde_json::from_slice(&list_bytes).unwrap();
        assert_eq!(displays.len(), 1);

        let update_body = json!({
            "label": "Stage Right",
            "host": "sd2l.lan",
            "port": 5566,
            "launchComponent": "com.example/.Main",
            "isEnabled": false
        })
        .to_string();
        let update_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::PUT)
                    .uri(format!(
                        "/integrations/android-stage/displays/{}",
                        created.id
                    ))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(update_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update_response.status(), StatusCode::OK);
        let updated_bytes = axum::body::to_bytes(update_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let updated: TestAndroidDisplayDto = serde_json::from_slice(&updated_bytes).unwrap();
        assert_eq!(updated.label, "Stage Right");
        assert_eq!(updated.host, "sd2l.lan");
        assert_eq!(updated.port, 5566);
        assert_eq!(updated.launch_component, "com.example/.Main");
        assert!(!updated.is_enabled);

        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::DELETE)
                    .uri(format!(
                        "/integrations/android-stage/displays/{}",
                        updated.id
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let list_after_delete = app
            .oneshot(
                Request::builder()
                    .uri("/integrations/android-stage/displays")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_after_delete_bytes =
            axum::body::to_bytes(list_after_delete.into_body(), usize::MAX)
                .await
                .unwrap();
        let after_delete_displays: Vec<TestAndroidDisplayDto> =
            serde_json::from_slice(&list_after_delete_bytes).unwrap();
        assert!(after_delete_displays.is_empty());
    }

    #[tokio::test]
    async fn libraries_endpoint_returns_seed() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/libraries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Vec<Library> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0].presentations.len(), 1);
    }

    #[tokio::test]
    async fn create_library_endpoint_persists_empty_library() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let body = serde_json::json!({ "name": "Autotest Library" }).to_string();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/libraries")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: Library = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(created.name, "Autotest Library");
        assert!(created.presentations.is_empty());

        let libraries_response = app
            .oneshot(
                Request::builder()
                    .uri("/libraries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let libraries_bytes = axum::body::to_bytes(libraries_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let libraries: Vec<Library> = serde_json::from_slice(&libraries_bytes).unwrap();
        assert_eq!(libraries.len(), 2);
        assert!(libraries.iter().any(|lib| lib.id == created.id));
    }

    #[tokio::test]
    async fn create_presentation_endpoint_creates_presentation() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let create_library_body = serde_json::json!({ "name": "Test Library" }).to_string();
        let library_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/libraries")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_library_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let library_bytes = axum::body::to_bytes(library_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let library: Library = serde_json::from_slice(&library_bytes).unwrap();

        let create_presentation_body = serde_json::json!({ "name": "Opening Song" }).to_string();
        let presentation_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri(format!("/libraries/{}/presentations", library.id))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_presentation_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(presentation_response.status(), StatusCode::OK);
        let presentation_bytes =
            axum::body::to_bytes(presentation_response.into_body(), usize::MAX)
                .await
                .unwrap();
        let payload: CreateLibraryPresentationResponse =
            serde_json::from_slice(&presentation_bytes).unwrap();
        assert_eq!(payload.library_id, library.id.into_uuid());
        assert_eq!(payload.presentation.name, "Opening Song");
        assert_eq!(payload.presentation.slides.len(), 1);

        let libraries_response = app
            .oneshot(
                Request::builder()
                    .uri("/libraries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let libraries_bytes = axum::body::to_bytes(libraries_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let libraries: Vec<Library> = serde_json::from_slice(&libraries_bytes).unwrap();
        let updated = libraries
            .into_iter()
            .find(|item| item.id == library.id)
            .expect("library present");
        assert!(updated
            .presentations
            .iter()
            .any(|presentation| presentation.name == "Opening Song"));
    }

    #[tokio::test]
    async fn update_presentation_endpoint_renames_presentation() {
        let state = AppState::in_memory().await.unwrap();
        let library = state.create_library("Rename Library").await.unwrap();
        let (_, _, presentation, _) = state
            .create_presentation(library.id, "Original Name")
            .await
            .unwrap();
        let app = build_router(state);

        let rename_body = serde_json::json!({ "name": "Renamed Song" }).to_string();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::PATCH)
                    .uri(format!("/presentations/{}", presentation.id))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(rename_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let detail_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/presentations/{}", presentation.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detail_response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(detail_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: PresentationDetailDto = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload.presentation.name, "Renamed Song");

        let libraries_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/libraries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let libraries_bytes = axum::body::to_bytes(libraries_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let libraries: Vec<Library> = serde_json::from_slice(&libraries_bytes).unwrap();
        let renamed = libraries
            .into_iter()
            .find(|lib| lib.id == library.id)
            .expect("library present");
        assert!(renamed
            .presentations
            .iter()
            .any(|item| item.name == "Renamed Song"));
    }

    #[tokio::test]
    async fn search_endpoint_returns_results() {
        let state = AppState::in_memory().await.unwrap();
        let library = state.create_library("Search Library").await.unwrap();
        let (_, _, presentation, _) = state
            .create_presentation(library.id, "Search Anthem")
            .await
            .unwrap();
        let slide_id = presentation.slides.first().unwrap().id;
        state
            .update_slide_content(
                presentation.id,
                slide_id,
                "Search line main".to_string(),
                "Search translation".to_string(),
                "Stage".to_string(),
                None,
            )
            .await
            .unwrap();

        let app = build_router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/search?query=Search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        const MAX: usize = usize::MAX;
        let bytes = axum::body::to_bytes(response.into_body(), MAX)
            .await
            .unwrap();
        let results: Vec<SearchResult> = serde_json::from_slice(&bytes).unwrap();
        assert!(results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Library)));
        assert!(results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Presentation)));
        assert!(results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Slide)));
    }

    #[tokio::test]
    async fn rename_library_endpoint_updates_name() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let create_body = serde_json::json!({ "name": "Original" }).to_string();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/libraries")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: Library = serde_json::from_slice(&bytes).unwrap();

        let rename_body = serde_json::json!({ "name": "Renamed" }).to_string();
        let rename_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::PATCH)
                    .uri(format!("/libraries/{}", created.id))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(rename_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rename_response.status(), StatusCode::NO_CONTENT);

        let libraries_response = app
            .oneshot(
                Request::builder()
                    .uri("/libraries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let libraries_bytes = axum::body::to_bytes(libraries_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let libraries: Vec<Library> = serde_json::from_slice(&libraries_bytes).unwrap();
        assert!(libraries
            .iter()
            .any(|library| library.id == created.id && library.name == "Renamed"));
    }

    #[tokio::test]
    async fn delete_library_endpoint_removes_library() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let create_body = serde_json::json!({ "name": "Disposable" }).to_string();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/libraries")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: Library = serde_json::from_slice(&bytes).unwrap();

        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::DELETE)
                    .uri(format!("/libraries/{}", created.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let libraries_response = app
            .oneshot(
                Request::builder()
                    .uri("/libraries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let libraries_bytes = axum::body::to_bytes(libraries_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let libraries: Vec<Library> = serde_json::from_slice(&libraries_bytes).unwrap();
        assert!(libraries.iter().all(|library| library.id != created.id));
    }

    #[tokio::test]
    async fn create_playlist_endpoint_supports_dashboard_flag() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let create_body = serde_json::json!({
            "name": "Root",
            "showInDashboard": true
        })
        .to_string();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/playlists")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: Playlist = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(created.name, "Root");
        assert!(created.show_in_dashboard);

        let list_response = app
            .oneshot(
                Request::builder()
                    .uri("/playlists")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_bytes = axum::body::to_bytes(list_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let playlists: Vec<Playlist> = serde_json::from_slice(&list_bytes).unwrap();
        assert!(playlists
            .iter()
            .any(|playlist| playlist.id == created.id && playlist.show_in_dashboard));
    }

    #[tokio::test]
    async fn update_playlist_endpoint_updates_metadata() {
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state.clone());

        let create_body = serde_json::json!({"name": "Original"}).to_string();
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/playlists")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let create_bytes = axum::body::to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let playlist: Playlist = serde_json::from_slice(&create_bytes).unwrap();

        let update_body = serde_json::json!({
            "name": "Updated Name",
            "showInDashboard": true
        })
        .to_string();

        let update_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::PATCH)
                    .uri(format!("/playlists/{}", playlist.id))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(update_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update_response.status(), StatusCode::OK);
        let update_bytes = axum::body::to_bytes(update_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let updated: Playlist = serde_json::from_slice(&update_bytes).unwrap();
        assert_eq!(updated.name, "Updated Name");
        assert!(updated.show_in_dashboard);

        let list_response = app
            .oneshot(
                Request::builder()
                    .uri("/playlists")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_bytes = axum::body::to_bytes(list_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let playlists: Vec<Playlist> = serde_json::from_slice(&list_bytes).unwrap();
        let found = playlists
            .iter()
            .find(|item| item.id == playlist.id)
            .expect("playlist present after update");
        assert_eq!(found.name, "Updated Name");
        assert!(found.show_in_dashboard);
    }

    #[tokio::test]
    async fn bible_translations_endpoint_returns_list() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .replace_bible_translation_passages(&sample_ingestion_batch())
            .await
            .unwrap();
        let app = build_router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/bible/translations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Vec<BibleTranslation> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0].code, "test");
    }

    #[tokio::test]
    async fn bible_search_endpoint_returns_matches() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .replace_bible_translation_passages(&sample_ingestion_batch())
            .await
            .unwrap();
        let app = build_router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/bible/search?translation=test&query=Text")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Vec<BiblePassage> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload.len(), 1);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/bible/search?translation=test&query=x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn bible_passage_endpoint_returns_reference() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .replace_bible_translation_passages(&sample_ingestion_batch())
            .await
            .unwrap();
        let app = build_router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/bible/passage?translation=test&book=John&chapter=3&verse_start=16")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Option<BiblePassage> = serde_json::from_slice(&bytes).unwrap();
        assert!(payload.is_some());
        let passage = payload.unwrap();
        assert_eq!(passage.translation.code, "test");
    }

    #[tokio::test]
    async fn tablet_ui_endpoint_renders_html() {
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ui/tablet")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("Presenter Tablet"));
        assert!(body.contains("Libraries"));
        assert!(body.contains("Presentations"));
    }

    #[tokio::test]
    async fn operator_ui_endpoint_renders_html() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .replace_bible_translation_passages(&sample_ingestion_batch())
            .await
            .unwrap();
        let app = build_router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ui/operator")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("Presenter Operator"));
        assert!(body.contains("Sample Library"));
        assert!(body.contains("Slides"));
        assert!(body.contains("Timers"));
        assert!(body.contains("data-mode=\"live\""));
    }

    #[tokio::test]
    async fn timer_overlay_endpoint_renders_html() {
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/overlays/timer")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("Presenter Timer Overlay"));
        assert!(body.contains("timer-value"));
    }

    #[tokio::test]
    async fn update_slide_content_endpoint_updates_slide() {
        let state = AppState::in_memory().await.unwrap();
        let libraries = state.libraries().await.unwrap();
        let presentation = &libraries[0].presentations[0];
        let slide = &presentation.slides[0];

        let app = build_router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!(
                        "/presentations/{}/slides/{}",
                        presentation.id, slide.id
                    ))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "main": "API main",
                            "translation": "API translation",
                            "stage": "API stage",
                            "group": "API Group"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let updated: Slide = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(updated.id, slide.id);
        assert_eq!(updated.content.main.value(), "API main");
        assert_eq!(updated.content.translation.value(), "API translation");
        assert_eq!(updated.content.stage.value(), "API stage");
        assert_eq!(
            updated.content.group.as_ref().map(|group| group.name()),
            Some("API Group")
        );

        let detail = state
            .presentation_detail(presentation.id)
            .await
            .unwrap()
            .expect("presentation detail");
        let stored = detail
            .2
            .slides
            .into_iter()
            .find(|candidate| candidate.id == slide.id)
            .expect("slide present");

        assert_eq!(stored.content.main.value(), "API main");
        assert_eq!(stored.content.translation.value(), "API translation");
        assert_eq!(stored.content.stage.value(), "API stage");
        assert_eq!(
            stored.content.group.as_ref().map(|group| group.name()),
            Some("API Group")
        );
    }

    #[tokio::test]
    async fn stage_displays_endpoint_returns_builtins() {
        let state = AppState::in_memory().await.unwrap();
        let libraries = state.libraries().await.unwrap();
        let presentation = &libraries[0].presentations[0];
        let current_slide = presentation.slides[0].id;
        let next_slide = presentation.slides.get(1).map(|slide| slide.id);

        let app = build_router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/stage/state")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "presentationId": presentation.id.to_string(),
                            "currentSlideId": current_slide.to_string(),
                            "nextSlideId": next_slide.map(|slide| slide.to_string()),
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/stage-displays")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Vec<StageDisplayLayout> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload.len(), 4);
        assert!(payload.iter().any(|layout| layout.code == "worship-snv"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/stage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains(&presentation.name));
        assert!(html.contains("Intro"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/stage/layout")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "code": "unknown" }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn stage_snapshot_endpoint_returns_json() {
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/stage/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let snapshot: StageDisplaySnapshot = serde_json::from_slice(&bytes).unwrap();
        assert!(snapshot.presentation_id.is_some());
        assert!(snapshot.current_slide_id.is_some());
    }

    #[tokio::test]
    async fn stage_layout_endpoint_reports_and_sets_layout() {
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/stage/layout")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let mut payload: StageLayoutResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload.code, "worship-snv");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/stage/layout")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "code": "timer" }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        payload = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload.code, "timer");
        assert_eq!(payload.layout.code, "timer");

        let current = state.stage_layout_code().await;
        assert_eq!(current, "timer");
    }

    #[tokio::test]
    async fn stage_clear_endpoint_blanks_outputs() {
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/stage/clear")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let snapshot = state
            .stage_display_snapshot("worship-snv")
            .await
            .unwrap()
            .expect("snapshot");
        assert!(snapshot.presentation_id.is_none());
        assert!(snapshot.current.is_none());
        assert!(snapshot.next.is_none());
    }

    #[tokio::test]
    async fn library_summary_endpoint_supports_filter() {
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/libraries/summary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let summaries: Vec<LibrarySummary> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].presentation_count, 1);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/libraries/summary?q=nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let filtered: Vec<LibrarySummary> = serde_json::from_slice(&bytes).unwrap();
        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn presentation_detail_endpoint_returns_data() {
        let state = AppState::in_memory().await.unwrap();
        let libraries = state.libraries().await.unwrap();
        let presentation = &libraries[0].presentations[0];
        let app = build_router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&format!("/presentations/{}", presentation.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let detail: PresentationDetailDto = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(detail.presentation.name, presentation.name);
        assert_eq!(detail.presentation.slides.len(), presentation.slides.len());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/presentations/00000000-0000-0000-0000-000000000000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn stage_state_rejects_invalid_uuids() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/stage/state")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "presentationId": "not-a-uuid",
                            "currentSlideId": "also-bad",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("presentationId"));
    }

    #[tokio::test]
    async fn timers_overview_endpoint_returns_snapshot() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/timers/overview")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: TimersOverview = serde_json::from_slice(&bytes).unwrap();
        assert!(payload.countdown_to_start.seconds_remaining > 0);
        assert_eq!(payload.preach_timer.state, TimerState::Idle);
    }

    #[tokio::test]
    async fn timers_command_endpoint_updates_state() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let target = (Utc::now() + ChronoDuration::minutes(30)).to_rfc3339();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/timers/command")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "command": "set_countdown_target", "target": target }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "error body: {}",
            String::from_utf8_lossy(&body)
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/timers/command")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "command": "start_countdown" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "error body: {}",
            String::from_utf8_lossy(&body)
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/timers/overview")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: TimersOverview = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload.countdown_to_start.state, TimerState::Running);
    }

    #[tokio::test]
    async fn timers_command_endpoint_rejects_past_targets() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let past = (Utc::now() - ChronoDuration::minutes(5)).to_rfc3339();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/timers/command")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "command": "set_countdown_target", "target": past }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn refresh_bible_translations_endpoint_uses_ingestion() {
        let mut state = AppState::in_memory().await.unwrap();
        state.set_test_bible_ingestion(mock_ingestion());
        let app = build_router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bible/translations/refresh")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Vec<BibleImportSummaryDto> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0].translation_code, "mock");
    }

    #[tokio::test]
    async fn bible_trigger_endpoint_returns_broadcast() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .replace_bible_translation_passages(&sample_ingestion_batch())
            .await
            .unwrap();
        let app = build_router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bible/trigger")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "translation": "test",
                            "book": "John",
                            "chapter": 3,
                            "verseStart": 16
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: presenter_core::BibleBroadcast = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.passage.reference.book, "John");

        let active = app
            .oneshot(
                Request::builder()
                    .uri("/bible/active")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(active.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(active.into_body(), usize::MAX)
            .await
            .unwrap();
        let current: Option<presenter_core::BibleBroadcast> =
            serde_json::from_slice(&bytes).unwrap();
        assert!(current.is_some());
    }

    #[tokio::test]
    async fn bible_clear_endpoint_resets_state() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .replace_bible_translation_passages(&sample_ingestion_batch())
            .await
            .unwrap();
        let app = build_router(state.clone());

        let trigger_request = Request::builder()
            .method("POST")
            .uri("/bible/trigger")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "translation": "test",
                    "book": "John",
                    "chapter": 3,
                    "verseStart": 16
                })
                .to_string(),
            ))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(trigger_request).await.unwrap().status(),
            StatusCode::OK
        );

        let clear = Request::builder()
            .method("POST")
            .uri("/bible/clear")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(clear).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let active = app
            .oneshot(
                Request::builder()
                    .uri("/bible/active")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(active.into_body(), usize::MAX)
            .await
            .unwrap();
        let current: Option<presenter_core::BibleBroadcast> =
            serde_json::from_slice(&bytes).unwrap();
        assert!(current.is_none());
    }

    #[tokio::test]
    async fn bible_ui_endpoint_renders_document() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .replace_bible_translation_passages(&sample_ingestion_batch())
            .await
            .unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ui/bible")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("Presenter Bible"));
    }

    fn sample_ingestion_batch() -> presenter_core::bible::BibleIngestionBatch {
        use presenter_core::{BiblePassage, BibleTranslation};
        let translation = BibleTranslation::new("test", "Test", "en");
        let reference = BibleReference::new("John", 3, 16, 16).unwrap();
        let passage = BiblePassage::new(reference, translation.clone(), "Text".to_string());
        presenter_core::bible::BibleIngestionBatch::new(translation, vec![passage]).unwrap()
    }

    fn mock_ingestion(
    ) -> std::sync::Arc<dyn crate::state::TestBibleIngestion + Send + Sync + 'static> {
        struct Mock;
        #[async_trait::async_trait]
        impl crate::state::TestBibleIngestion for Mock {
            async fn ingest_default_translations(
                &self,
            ) -> anyhow::Result<Vec<presenter_bible::BibleImportSummary>> {
                Ok(vec![presenter_bible::BibleImportSummary {
                    translation_code: "mock".to_string(),
                    passage_count: 1,
                }])
            }
        }
        std::sync::Arc::new(Mock)
    }

    #[test]
    fn update_playlist_request_defaults_flags() {
        let payload: UpdatePlaylistRequest = serde_json::from_str(r#"{}"#).expect("deserialises");
        assert!(payload.name.is_none());
        assert!(payload.show_in_dashboard.is_none());
    }
}
