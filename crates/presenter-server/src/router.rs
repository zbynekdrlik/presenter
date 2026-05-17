mod ai;
mod api_stage;
mod bible;
mod features;
mod integrations;
mod libraries;
pub mod network_mode;
mod playlists;
mod presentations;
mod search;
pub(crate) mod stage;
mod tablet_pwa;
mod timers;
mod ui_routes;
mod wasm_ui;
use crate::state::AppState;
use axum::{
    extract::{ws::WebSocketUpgrade, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post, put},
    Json, Router,
};
use serde::Serialize;
use tracing::instrument;
use uuid::Uuid;
// Feature modules host their own request/DTO types

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/", get(ui_routes::home))
        .route("/search", get(search::search_presenter_endpoint))
        .route("/libraries/summary", get(libraries::list_library_summaries))
        .route(
            "/libraries",
            get(libraries::list_libraries).post(libraries::create_library),
        )
        .route(
            "/libraries/{id}",
            patch(libraries::rename_library).delete(libraries::delete_library),
        )
        .route(
            "/libraries/favorites",
            get(libraries::list_library_favorites),
        )
        .route(
            "/libraries/{id}/favorite",
            post(libraries::set_library_favorite),
        )
        .route(
            "/libraries/{id}/presentations",
            post(libraries::create_library_presentation),
        )
        .route(
            "/libraries/{id}/presentations/import",
            post(libraries::import_presentation),
        )
        .route("/bible/translations", get(bible::list_bible_translations))
        .route(
            "/bible/translations/{code}",
            patch(bible::update_bible_translation),
        )
        .route("/bible/books", get(bible::list_bible_books))
        .route("/bible/search", get(bible::search_bible_passages))
        .route("/bible/passage", get(bible::get_bible_passage))
        .route("/bible/resolve", post(bible::resolve_bible_slides))
        .route(
            "/bible/translations/refresh",
            post(bible::refresh_bible_translations),
        )
        .route(
            "/bible/presentations",
            get(bible::list_bible_presentations).post(bible::create_bible_presentation_handler),
        )
        .route(
            "/bible/presentations/{id}",
            get(bible::get_bible_presentation)
                .patch(bible::rename_bible_presentation_handler)
                .delete(bible::delete_bible_presentation_handler),
        )
        .route(
            "/bible/presentations/{id}/append",
            post(bible::append_bible_presentation_handler),
        )
        .route(
            "/bible/presentations/{id}/slides/reorder",
            post(bible::reorder_bible_presentation_slides),
        )
        .route(
            "/bible/presentations/{id}/slides/{slide_id}",
            patch(bible::update_bible_slide).delete(bible::delete_bible_presentation_slide),
        )
        .route(
            "/bible/presentations/{id}/slides/{slide_id}/trigger",
            post(bible::trigger_presentation_slide),
        )
        .route("/bible/active", get(bible::get_active_bible_broadcast))
        .route(
            "/bible/active-slide",
            get(bible::get_active_bible_slide_output),
        )
        .route("/bible/trigger", post(bible::trigger_bible_broadcast))
        .route("/bible/trigger-slide", post(bible::trigger_bible_slide))
        .route("/bible/clear", post(bible::clear_bible_broadcast))
        .route(
            "/bible/preferences",
            get(bible::get_bible_preferences).put(bible::update_bible_preferences),
        )
        .route(
            "/playlists",
            get(playlists::list_playlists).post(playlists::create_playlist),
        )
        .route(
            "/playlists/{id}",
            get(playlists::get_playlist)
                .patch(playlists::update_playlist)
                .delete(playlists::delete_playlist),
        )
        .route(
            "/playlists/{id}/entries",
            put(playlists::replace_playlist_entries),
        )
        // WASM UI is the default operator interface
        .route("/ui/operator", get(wasm_ui::wasm_ui_shell))
        .route(
            "/ui/operator/{*path}",
            get(wasm_ui::wasm_ui_shell_with_path),
        )
        .route("/ui-pkg/{*path}", get(wasm_ui::wasm_ui_asset))
        .route("/ui/tablet", get(wasm_ui::wasm_ui_shell))
        .route("/ui/camera", get(wasm_ui::wasm_ui_shell))
        .route("/ui/tablet/manifest.json", get(tablet_pwa::tablet_manifest))
        .route("/ui/tablet/icon-192.png", get(tablet_pwa::icon_192))
        .route("/ui/tablet/icon-512.png", get(tablet_pwa::icon_512))
        .route(
            "/ui/tablet/apple-touch-icon.png",
            get(tablet_pwa::apple_touch_icon),
        )
        .route("/ui/tablet/sw.js", get(tablet_pwa::service_worker))
        .route(
            "/ui/bible",
            get(|| async { axum::response::Redirect::permanent("/ui/operator/bible") }),
        )
        .route("/ui/settings", get(ui_routes::settings_ui))
        .route("/overlays/timer", get(ui_routes::timer_overlay))
        .route("/stage-displays", get(stage::list_stage_displays))
        .route(
            "/stage/layout",
            get(stage::get_stage_layout).post(stage::set_stage_layout),
        )
        .route("/stage/connections", get(stage::list_stage_connections))
        .route("/stage", get(wasm_ui::wasm_ui_shell))
        .route(
            "/stage/snapshot",
            get(stage::stage_display_selected_snapshot_json),
        )
        .route("/stage/state", post(stage::update_stage_state))
        .route("/stage/clear", post(stage::clear_stage_state))
        .route(
            "/stage/broadcast-live",
            get(stage::get_broadcast_live).patch(stage::set_broadcast_live),
        )
        .route("/api/stage", put(api_stage::update_api_stage))
        .route(
            "/integrations/resolume/hosts",
            get(integrations::resolume::list_resolume_hosts)
                .post(integrations::resolume::create_resolume_host),
        )
        .route(
            "/integrations/resolume/hosts/{id}",
            put(integrations::resolume::update_resolume_host)
                .delete(integrations::resolume::delete_resolume_host),
        )
        .route(
            "/integrations/resolume/hosts/{id}/test",
            post(integrations::resolume::test_resolume_host),
        )
        .route(
            "/integrations/android-stage/displays",
            get(integrations::android_stage::list_android_stage_displays)
                .post(integrations::android_stage::create_android_stage_display),
        )
        .route(
            "/integrations/android-stage/displays/{id}",
            put(integrations::android_stage::update_android_stage_display)
                .delete(integrations::android_stage::delete_android_stage_display),
        )
        .route(
            "/integrations/android-stage/displays/{id}/launch-now",
            post(integrations::android_stage::launch_now_android_stage_display),
        )
        .route(
            "/integrations/osc/settings",
            get(integrations::osc::get_osc_settings).put(integrations::osc::update_osc_settings),
        )
        .route(
            "/integrations/osc/status",
            get(integrations::osc::get_osc_status),
        )
        .route(
            "/integrations/ableset/settings",
            get(integrations::ableset::get_ableset_settings)
                .put(integrations::ableset::update_ableset_settings),
        )
        .route(
            "/integrations/ableset/status",
            get(integrations::ableset::get_ableset_status),
        )
        .route(
            "/integrations/ableset/follow",
            post(integrations::ableset::set_ableset_follow),
        )
        .route(
            "/integrations/video-sources",
            get(integrations::video_source::list_video_sources)
                .post(integrations::video_source::create_video_source),
        )
        .route(
            "/integrations/video-sources/deactivate",
            post(integrations::video_source::deactivate_video_sources),
        )
        .route(
            "/integrations/video-sources/{id}",
            put(integrations::video_source::update_video_source)
                .delete(integrations::video_source::delete_video_source),
        )
        .route(
            "/integrations/video-sources/{id}/activate",
            post(integrations::video_source::activate_video_source),
        )
        .route("/ndi/sources", get(integrations::ndi::discover_ndi_sources))
        .route("/ndi/status", get(integrations::ndi::ndi_status))
        .route("/ndi/stream", get(integrations::ndi::mjpeg_ws))
        .route("/ndi/mjpeg", get(integrations::ndi::mjpeg_http))
        .route("/group-colors", get(presentations::get_group_colors))
        .route(
            "/presentations/{id}",
            get(presentations::get_presentation_detail)
                .patch(presentations::update_presentation)
                .delete(presentations::delete_presentation),
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
        .route("/timers/overview", get(timers::get_timers_overview))
        .route("/timers/command", post(timers::execute_timer_command))
        .route("/live/ws", get(live_websocket))
        .route(
            "/settings/features",
            get(features::get_feature_settings).post(features::update_feature_settings),
        )
        .route("/ai/chat", post(ai::chat))
        .route(
            "/ai/settings",
            get(ai::get_settings).put(ai::update_settings),
        )
        .route("/ai/conversation", get(ai::get_conversation))
        .route("/ai/clear", post(ai::clear_conversation))
        .route("/ai/status", get(ai::check_status))
        .route("/ai/proxy/start", post(ai::proxy_start))
        .route("/ai/proxy/stop", post(ai::proxy_stop))
        .route("/ai/proxy/login", post(ai::proxy_login))
        .route("/ai/proxy/complete-login", post(ai::proxy_complete_login))
        .route("/api/network-mode", get(network_mode::get_network_mode))
        .with_state(state)
}

/// Application version from Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build channel: "dev" (default) or "release" (set via PRESENTER_BUILD_CHANNEL env at compile time)
pub const BUILD_CHANNEL: &str = match option_env!("PRESENTER_BUILD_CHANNEL") {
    Some(ch) => ch,
    None => "dev",
};

async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "version": VERSION,
            "channel": BUILD_CHANNEL
        })),
    )
}

// request structs moved to feature modules

// Playlist handlers live in router/playlists.rs

// Presentation request types and handlers live in router/presentations.rs

// Bible UI handler is implemented in router/bible.rs

// stage request/response moved to stage.rs

// feature settings moved to features.rs

// (implementations removed: moved to router/presentations.rs)

// osc settings moved to integrations/osc.rs

// stage state request moved to stage.rs

#[instrument(skip_all)]
async fn live_websocket(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let hub = state.live_hub();
    let connections = state.stage_connections_handle();
    ws.on_upgrade(move |socket| async move {
        crate::live::serve_websocket(hub, connections, socket).await;
    })
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

    fn internal(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            anyhow::anyhow!(message.into()),
        )
    }

    fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            anyhow::anyhow!(message.into()),
        )
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
mod tests;
