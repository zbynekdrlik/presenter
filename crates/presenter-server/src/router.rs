mod bible;
mod features;
mod integrations;
mod libraries;
mod playlists;
mod presentations;
mod search;
mod stage;
mod timers;
mod ui_routes;
use crate::state::AppState;
use axum::{
    extract::{ws::WebSocketUpgrade, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post, put},
    Json, Router,
};
use presenter_core::{LibraryId, Presentation};
use serde::{Deserialize, Serialize};
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
            "/libraries/{id}/favorite",
            post(libraries::set_library_favorite),
        )
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
        .route(
            "/playlists",
            get(playlists::list_playlists).post(playlists::create_playlist),
        )
        .route(
            "/playlists/{id}",
            patch(playlists::update_playlist).delete(playlists::delete_playlist),
        )
        .route(
            "/playlists/{id}/entries",
            put(playlists::replace_playlist_entries),
        )
        .route("/ui/operator", get(ui_routes::operator_ui))
        .route("/ui/tablet", get(ui_routes::tablet_ui))
        .route("/ui/bible", get(bible::bible_ui))
        .route("/ui/settings", get(ui_routes::settings_ui))
        .route("/overlays/timer", get(ui_routes::timer_overlay))
        .route("/stage-displays", get(stage::list_stage_displays))
        .route(
            "/stage/layout",
            get(stage::get_stage_layout).post(stage::set_stage_layout),
        )
        .route("/stage/connections", get(stage::list_stage_connections))
        .route("/stage", get(stage::stage_display_selected_html))
        .route(
            "/stage/snapshot",
            get(stage::stage_display_selected_snapshot_json),
        )
        .route("/stage/state", post(stage::update_stage_state))
        .route("/stage/clear", post(stage::clear_stage_state))
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
        .route("/timers/overview", get(timers::get_timers_overview))
        .route("/timers/command", post(timers::execute_timer_command))
        .route("/live/ws", get(live_websocket))
        .route(
            "/settings/features",
            get(features::get_feature_settings).post(features::update_feature_settings),
        )
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
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

// resolume DTOs moved to integrations/resolume.rs

// android stage DTOs moved to integrations/android_stage.rs

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
mod tests;

#[cfg(test)]
mod tests_old {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::{Duration as ChronoDuration, Utc};
    use presenter_core::{
        BiblePassage, BibleReference, BibleTranslation, Library, LibrarySummary, SearchResult,
        SearchResultKind, Slide, TimerState,
    };
    use serde::Deserialize;
    use serde_json::json;
    use tower::ServiceExt;
    // Bring types from feature modules and core used only in tests
    use crate::router::libraries::CreateLibraryPresentationResponse;
    use crate::router::playlists::UpdatePlaylistRequest;
    use crate::router::stage::StageLayoutResponse;
    use presenter_core::Playlist;
    use presenter_core::TimersOverview;
    use presenter_core::{StageDisplayLayout, StageDisplaySnapshot};

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
