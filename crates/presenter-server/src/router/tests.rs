use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration as ChronoDuration, Local, Timelike, Utc};
use presenter_core::{
    BiblePassage, BibleReference, BibleTranslation, Library, LibrarySummary, SearchMatchField,
    SearchResult, SearchResultKind, Slide, TimerState, DEFAULT_STAGE_LAYOUT_CODE,
};
use serde::Deserialize;
use serde_json::json;
use tower::ServiceExt;
// Bring types from feature modules and core used only in tests
use crate::router::bible::BibleImportSummaryDto;
use crate::router::libraries::CreateLibraryPresentationResponse;
use crate::router::playlists::UpdatePlaylistRequest;
use crate::router::presentations::PresentationDetailDto;
use crate::router::stage::StageLayoutResponse;
use presenter_core::playlist::PlaylistEntryKind;
use presenter_core::TimersOverview;
use presenter_core::{Playlist, PlaylistEntry, PlaylistEntryId};
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

/// GET `uri` and deserialize the 200 JSON body.
async fn get_json<T: serde::de::DeserializeOwned>(app: &Router, uri: &str) -> T {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "GET {uri}");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Send a JSON `body` to `uri` with `method`; assert 200 and deserialize the reply.
async fn send_json<T: serde::de::DeserializeOwned>(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    body: serde_json::Value,
) -> T {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method.clone())
                .uri(uri)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{method} {uri}");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// DELETE `uri`; assert 204 No Content.
async fn delete_no_content(app: &Router, uri: &str) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(axum::http::Method::DELETE)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT, "DELETE {uri}");
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

/// Regression for #333 item 7: `/healthz` must report NDI pipeline state so
/// dashboards can detect activation failures within seconds instead of
/// inferring from "operator sees red error". Field shape:
/// `ndi_pipelines: [{source_id, state, last_error?}]`. Always present (empty
/// array when no NDI manager is loaded).
#[tokio::test]
async fn health_endpoint_reports_ndi_pipelines_field() {
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("/healthz returns JSON");
    let pipelines = body
        .get("ndi_pipelines")
        .expect("/healthz must include ndi_pipelines field for #333 item 7");
    assert!(
        pipelines.is_array(),
        "ndi_pipelines must be an array, got: {pipelines:?}"
    );
}

/// Schema regression for #333 item 7 (deep-review 🟡 #3): the snapshot
/// renderer must produce stable JSON for every PipelineState variant and
/// MUST include `last_error` only on the `errored` variant. Previously
/// the test only asserted the field name; this test pins the per-variant
/// shape so mutations to state labels or last_error inclusion are caught.
#[test]
fn render_ndi_pipeline_entry_emits_correct_shape_per_variant() {
    use presenter_ndi::pipeline::PipelineState;

    let starting = super::render_ndi_pipeline_entry("src-1", &PipelineState::Starting);
    assert_eq!(starting["source_id"], "src-1");
    assert_eq!(starting["state"], "starting");
    assert!(
        starting.get("last_error").is_none(),
        "Starting must NOT include last_error: {starting:?}"
    );

    let streaming = super::render_ndi_pipeline_entry("src-2", &PipelineState::Streaming);
    assert_eq!(streaming["state"], "streaming");
    assert!(streaming.get("last_error").is_none());

    let stopped = super::render_ndi_pipeline_entry("src-3", &PipelineState::Stopped);
    assert_eq!(stopped["state"], "stopped");
    assert!(stopped.get("last_error").is_none());

    let errored = super::render_ndi_pipeline_entry(
        "src-4",
        &PipelineState::Errored("pipeline died: ndisrc EOS".to_string()),
    );
    assert_eq!(errored["state"], "errored");
    assert_eq!(
        errored["last_error"], "pipeline died: ndisrc EOS",
        "Errored MUST carry last_error verbatim so dashboards surface the cause"
    );
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
async fn home_route_links_match_live_routes() {
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

    assert!(
        body.contains("/ui/camera"),
        "landing page must link to /ui/camera"
    );

    assert!(
        !body.contains("/ui/stage-design"),
        "landing page must not link to /ui/stage-design (no such route)"
    );

    let settings_link_count = body.matches("\"/ui/settings\"").count();
    assert_eq!(
        settings_link_count, 1,
        "landing page must link /ui/settings exactly once (was {settings_link_count})"
    );

    for path in [
        "/ui/operator",
        "/ui/tablet",
        "/ui/camera",
        "/ui/bible",
        "/ui/settings",
        "/stage",
        "/overlays/timer",
    ] {
        let target_response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = target_response.status();
        // 200..=399 always acceptable. WASM-shell paths return 503 in
        // tests when the bundle isn't built — still proves the route is
        // registered. 500/502/404 = bug.
        let ok = matches!(status.as_u16(), 200..=399) || status == StatusCode::SERVICE_UNAVAILABLE;
        assert!(
            ok,
            "landing-page link {path} returned {status} — broken link",
        );
    }
}

#[tokio::test]
async fn favicon_is_served_so_no_404_console_error() {
    // Regression for #361: browsers auto-request /favicon.ico on every route.
    // Without a handler this returned 404, logging a console error on every page
    // and masking real console errors. Assert a real image is served instead.
    let app = build_router(AppState::in_memory().await.unwrap());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/favicon.ico")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/favicon.ico must return 200 (was {}) — a 404 logs a browser console error on every route",
        response.status()
    );
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("image/"),
        "/favicon.ico must be served as an image (was {content_type:?})"
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        !bytes.is_empty(),
        "/favicon.ico must return a non-empty icon body"
    );
}

#[tokio::test]
async fn settings_route_serves_wasm_shell() {
    // #347: /ui/settings was migrated from the SSR settings_script.js page to a
    // Leptos WASM component, so it now serves the WASM app shell (index.html).
    // In unit tests the trunk bundle isn't built, so the shell returns 503 —
    // still proving the route is registered and reaches the WASM handler (not a
    // 404/500). The page's links/behavior are exercised by the Playwright E2E
    // (tests/e2e/settings.spec.ts) against the real built bundle.
    let app = build_router(AppState::in_memory().await.unwrap());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ui/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    assert!(
        matches!(status.as_u16(), 200..=399) || status == StatusCode::SERVICE_UNAVAILABLE,
        "/ui/settings must serve the WASM shell (got {status})"
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        !body.contains("/ui/stage-design"),
        "/ui/settings must not reference /ui/stage-design (no such route)"
    );
}

#[tokio::test]
async fn resolume_host_endpoints_crud() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let base = "/integrations/resolume/hosts";

    let empty_hosts: Vec<TestResolumeHostDto> = get_json(&app, base).await;
    assert!(empty_hosts.is_empty());

    let created: TestResolumeHostDto = send_json(
        &app,
        axum::http::Method::POST,
        base,
        json!({ "label": "Arena", "host": "resolume.lan", "port": 8090, "isEnabled": true }),
    )
    .await;
    assert_eq!(created.label, "Arena");
    assert_eq!(created.host, "resolume.lan");
    assert_eq!(created.port, 8090);
    assert!(created.is_enabled);
    assert!(!created.status.state.is_empty());

    let hosts: Vec<TestResolumeHostDto> = get_json(&app, base).await;
    assert_eq!(hosts.len(), 1);
    assert!(!hosts[0].status.state.is_empty());

    let updated: TestResolumeHostDto = send_json(
        &app,
        axum::http::Method::PUT,
        &format!("{base}/{}", created.id),
        json!({ "label": "Arena North", "host": "resolume.lan", "port": 8090, "isEnabled": false }),
    )
    .await;
    assert_eq!(updated.label, "Arena North");
    assert!(!updated.is_enabled);
    assert_eq!(updated.host, "resolume.lan");
    assert!(!updated.status.state.is_empty());

    delete_no_content(&app, &format!("{base}/{}", updated.id)).await;

    let after_delete_hosts: Vec<TestResolumeHostDto> = get_json(&app, base).await;
    assert!(after_delete_hosts.is_empty());
}

#[tokio::test]
async fn android_stage_display_endpoints_crud() {
    std::env::set_var("PRESENTER_ANDROID_ADB_BIN", "true");
    let app = build_router(AppState::in_memory().await.unwrap());
    let base = "/integrations/android-stage/displays";

    let initial_displays: Vec<TestAndroidDisplayDto> = get_json(&app, base).await;
    let initial_count = initial_displays.len();

    let created: TestAndroidDisplayDto = send_json(
        &app,
        axum::http::Method::POST,
        base,
        json!({
            "label": "Stage Left",
            "host": "test-stage.invalid",
            "port": 5555,
            "launchComponent": "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity",
            "isEnabled": true
        }),
    )
    .await;
    assert_eq!(created.label, "Stage Left");
    assert_eq!(created.host, "test-stage.invalid");
    assert_eq!(created.port, 5555);
    assert_eq!(
        created.launch_component,
        "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity"
    );

    let displays: Vec<TestAndroidDisplayDto> = get_json(&app, base).await;
    assert_eq!(displays.len(), initial_count + 1);
    assert!(displays.iter().any(|d| d.id == created.id));

    let updated: TestAndroidDisplayDto = send_json(
        &app,
        axum::http::Method::PUT,
        &format!("{base}/{}", created.id),
        json!({
            "label": "Stage Right",
            "host": "other-stage.invalid",
            "port": 5566,
            "launchComponent": "com.example/.Main",
            "isEnabled": false
        }),
    )
    .await;
    assert_eq!(updated.label, "Stage Right");
    assert_eq!(updated.host, "other-stage.invalid");
    assert_eq!(updated.port, 5566);
    assert_eq!(updated.launch_component, "com.example/.Main");
    assert!(!updated.is_enabled);

    delete_no_content(&app, &format!("{base}/{}", updated.id)).await;

    let after_delete_displays: Vec<TestAndroidDisplayDto> = get_json(&app, base).await;
    assert_eq!(after_delete_displays.len(), initial_count);
    assert!(after_delete_displays.iter().all(|d| d.id != created.id));
}

#[tokio::test]
async fn libraries_endpoint_returns_seed() {
    let state = AppState::in_memory().await.unwrap();
    crate::state::seed_sample_library(&state).await.unwrap();
    let app = build_router(state);
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
    let state = AppState::in_memory().await.unwrap();
    crate::state::seed_sample_library(&state).await.unwrap();
    let app = build_router(state);
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
    let presentation_bytes = axum::body::to_bytes(presentation_response.into_body(), usize::MAX)
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
        .create_presentation(library.id, "Original Name", None)
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
        .create_presentation(library.id, "Search Anthem", None)
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
            None, // metadata
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
    assert!(
        results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Library)),
        "expected a Library result"
    );
    assert!(
        results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Presentation)),
        "expected a Presentation result"
    );
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
async fn tablet_ui_endpoint_serves_wasm_shell() {
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
    // WASM shell returns 200 OK (or 503 if not built)
    let status = response.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::SERVICE_UNAVAILABLE,
        "Expected 200 or 503, got {status}"
    );
}

#[tokio::test]
async fn operator_ui_endpoint_serves_wasm_shell() {
    let state = AppState::in_memory().await.unwrap();
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
    // WASM shell returns OK if dist is built, or 503 if not — both are valid
    let status = response.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::SERVICE_UNAVAILABLE,
        "unexpected status: {status}"
    );
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
    crate::state::seed_sample_library(&state).await.unwrap();
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
    crate::state::seed_sample_library(&state).await.unwrap();
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
    // camera-crew is excluded from the operator layout picker (Issue 1 fix).
    // Count is built_in() minus camera-crew = 7.
    assert_eq!(payload.len(), 7);
    assert!(payload
        .iter()
        .any(|layout| layout.code == DEFAULT_STAGE_LAYOUT_CODE));
    assert!(payload.iter().any(|layout| layout.code == "ndi-fullscreen"));
    assert!(payload.iter().any(|layout| layout.code == "bible"));
    assert!(payload.iter().any(|layout| layout.code == "api"));
    assert!(
        !payload.iter().any(|layout| layout.code == "camera-crew"),
        "camera-crew must not appear in operator layout picker"
    );

    // /stage now serves the WASM shell (or 503 if dist/ not built).
    // In unit tests without a Trunk build, it returns 503 with a fallback message.
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
    // Accept either 200 (dist/ exists) or 503 (dist/ not built)
    assert!(
        response.status() == StatusCode::OK || response.status() == StatusCode::SERVICE_UNAVAILABLE
    );

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
    crate::state::seed_sample_library(&state).await.unwrap();
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
    assert_eq!(payload.code, DEFAULT_STAGE_LAYOUT_CODE);

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
        .stage_display_snapshot(DEFAULT_STAGE_LAYOUT_CODE)
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
    crate::state::seed_sample_library(&state).await.unwrap();
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
    crate::state::seed_sample_library(&state).await.unwrap();
    let libraries = state.libraries().await.unwrap();
    let presentation = &libraries[0].presentations[0];
    let app = build_router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/presentations/{}", presentation.id))
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
async fn timers_command_set_countdown_target_local() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let future_hour = (Local::now().hour() + 2) % 24;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/timers/command")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command": "set_countdown_target_local",
                        "hours": future_hour,
                        "minutes": 0
                    })
                    .to_string(),
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

    let payload: TimersOverview = serde_json::from_slice(&body).unwrap();
    assert!(payload.countdown_to_start.seconds_remaining > 0);
    assert!(!payload.countdown_to_start.target_local.is_empty());
}

#[tokio::test]
async fn timers_command_adjust_countdown_target() {
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let initial: TimersOverview = serde_json::from_slice(&bytes).unwrap();
    let initial_remaining = initial.countdown_to_start.seconds_remaining;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/timers/command")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command": "adjust_countdown_target",
                        "offset_minutes": 5
                    })
                    .to_string(),
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

    let payload: TimersOverview = serde_json::from_slice(&body).unwrap();
    let diff = payload.countdown_to_start.seconds_remaining - initial_remaining;
    assert!(
        (295..=305).contains(&diff),
        "expected ~300s increase, got {diff}"
    );
}

#[tokio::test]
async fn timers_overview_includes_target_local() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let response = app
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
    assert!(
        payload.countdown_to_start.target_local.len() >= 7,
        "expected HH:MM:SS format, got: {}",
        payload.countdown_to_start.target_local
    );
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
    let current: Option<presenter_core::BibleBroadcast> = serde_json::from_slice(&bytes).unwrap();
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
    let current: Option<presenter_core::BibleBroadcast> = serde_json::from_slice(&bytes).unwrap();
    assert!(current.is_none());
}

#[tokio::test]
async fn bible_ui_endpoint_redirects_to_operator_bible() {
    let state = AppState::in_memory().await.unwrap();
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
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap();
    assert_eq!(location, "/ui/operator/bible");
}

fn sample_ingestion_batch() -> presenter_core::bible::BibleIngestionBatch {
    use presenter_core::{BiblePassage, BibleTranslation};
    let translation = BibleTranslation::new("test", "Test", "en");
    let reference = BibleReference::new("John", 3, 16, 16).unwrap();
    let passage = BiblePassage::new(reference, translation.clone(), "Text".to_string());
    presenter_core::bible::BibleIngestionBatch::new(translation, vec![passage]).unwrap()
}

fn mock_ingestion() -> std::sync::Arc<dyn crate::state::TestBibleIngestion + Send + Sync + 'static>
{
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

#[tokio::test]
async fn get_playlist_returns_playlist_when_present() {
    let app = build_router(AppState::in_memory().await.unwrap());

    // Create a playlist
    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/playlists")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"GET test","showInDashboard":false}"#.to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_resp.status(), StatusCode::OK);
    let create_bytes = axum::body::to_bytes(create_resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&create_bytes).unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    // GET it back
    let get_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/playlists/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);
    let get_bytes = axum::body::to_bytes(get_resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&get_bytes).unwrap();
    assert_eq!(fetched["id"].as_str().unwrap(), id);
    assert_eq!(fetched["name"].as_str().unwrap(), "GET test");
}

#[tokio::test]
async fn get_playlist_returns_404_when_missing() {
    let app = build_router(AppState::in_memory().await.unwrap());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/playlists/00000000-0000-0000-0000-000000000000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[test]
fn update_playlist_request_defaults_flags() {
    let payload: UpdatePlaylistRequest = serde_json::from_str(r"{}").expect("deserialises");
    assert!(payload.name.is_none());
    assert!(payload.show_in_dashboard.is_none());
}

#[tokio::test]
async fn network_mode_endpoint_returns_local_for_direct_request() {
    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/network-mode")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["mode"], "local");
}

#[tokio::test]
async fn network_mode_endpoint_returns_remote_with_foreign_cf_ip() {
    // State without a configured local_public_ip → falls back to private-range check.
    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/network-mode")
                .header("CF-Connecting-IP", "198.51.100.10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["mode"], "remote");
}

#[tokio::test]
async fn get_playlist_response_includes_presentation_name() {
    let state = AppState::in_memory().await.unwrap();
    // Seed: create one library + presentation
    let library = state
        .create_library("Test Library")
        .await
        .expect("create library");
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "My Song", None)
        .await
        .expect("create presentation");
    // Create playlist with that presentation as an entry
    let playlist = state
        .create_playlist("Test Playlist", true)
        .await
        .expect("create playlist");
    let entries = vec![PlaylistEntry {
        id: PlaylistEntryId::new(),
        kind: PlaylistEntryKind::Presentation {
            presentation_id: presentation.id,
            midi_binding: None,
            presentation_name: None,
        },
    }];
    state
        .replace_playlist_entries(playlist.id, entries)
        .await
        .expect("replace entries");

    let app = build_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/playlists/{}", playlist.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .unwrap(),
    )
    .unwrap();
    let entry = &body["entries"][0];
    assert_eq!(entry["type"], "presentation");
    assert_eq!(
        entry["presentation_name"], "My Song",
        "presentation_name must be present in playlist GET response"
    );
}

#[tokio::test]
async fn list_playlists_response_includes_presentation_names() {
    let state = AppState::in_memory().await.unwrap();
    let library = state.create_library("Lib").await.unwrap();
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "Track One", None)
        .await
        .unwrap();
    let playlist = state.create_playlist("PL", true).await.unwrap();
    state
        .replace_playlist_entries(
            playlist.id,
            vec![PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Presentation {
                    presentation_id: presentation.id,
                    midi_binding: None,
                    presentation_name: None,
                },
            }],
        )
        .await
        .unwrap();
    let app = build_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/playlists")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .unwrap(),
    )
    .unwrap();
    let pl = body
        .as_array()
        .unwrap()
        .iter()
        .find(|pl| pl["name"] == "PL")
        .expect("playlist named PL must be present");
    assert_eq!(
        pl["entries"][0]["presentation_name"], "Track One",
        "presentation_name must be present in list response"
    );
}

#[tokio::test]
async fn search_slide_text_match_returns_parent_presentation() {
    let state = AppState::in_memory().await.unwrap();
    let library = state.create_library("Whole Songs Library").await.unwrap();
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "Some Anthem", None)
        .await
        .unwrap();
    let slide_id = presentation.slides.first().unwrap().id;
    state
        .update_slide_content(
            presentation.id,
            slide_id,
            "test283-slide-marker is in the lyrics".to_string(),
            String::new(),
            String::new(),
            None,
            None,
        )
        .await
        .unwrap();

    let app = build_router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/search?query=test283-slide-marker")
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

    let presentation_results: Vec<&SearchResult> = results
        .iter()
        .filter(|result| matches!(result.kind, SearchResultKind::Presentation))
        .collect();
    assert_eq!(
        presentation_results.len(),
        1,
        "expected exactly one Presentation result, got: {:?}",
        results
    );
    assert_eq!(
        presentation_results[0].presentation_name.as_deref(),
        Some("Some Anthem")
    );
}

#[tokio::test]
async fn search_dedupes_when_song_matches_both_name_and_slides() {
    let state = AppState::in_memory().await.unwrap();
    let library = state.create_library("Dedupe Library").await.unwrap();
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "Marker Song", None)
        .await
        .unwrap();
    let slide_id = presentation.slides.first().unwrap().id;
    state
        .update_slide_content(
            presentation.id,
            slide_id,
            "the marker is also here".to_string(),
            String::new(),
            String::new(),
            None,
            None,
        )
        .await
        .unwrap();

    let app = build_router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/search?query=marker")
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

    let presentation_results: Vec<&SearchResult> = results
        .iter()
        .filter(|result| matches!(result.kind, SearchResultKind::Presentation))
        .collect();
    assert_eq!(
        presentation_results.len(),
        1,
        "song matched by both name and slide text must appear ONCE; got: {:?}",
        results
    );
}

#[tokio::test]
async fn search_slide_translation_match_returns_parent_presentation() {
    // Slide text only in the translation field — verifies the
    // TranslationText branch of match_field selection in search_slides.
    // Library and presentation names are intentionally neutral so they
    // don't overlap with the marker tokens and cause false deduplication.
    let state = AppState::in_memory().await.unwrap();
    let library = state.create_library("Worship Library").await.unwrap();
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "Sunday Anthem", None)
        .await
        .unwrap();
    let slide_id = presentation.slides.first().unwrap().id;
    state
        .update_slide_content(
            presentation.id,
            slide_id,
            String::new(),
            "test283-translation-marker is in the translation".to_string(),
            String::new(),
            None,
            None,
        )
        .await
        .unwrap();

    let app = build_router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/search?query=test283-translation-marker")
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

    let presentation_results: Vec<&SearchResult> = results
        .iter()
        .filter(|result| matches!(result.kind, SearchResultKind::Presentation))
        .collect();
    assert_eq!(
        presentation_results.len(),
        1,
        "expected exactly one Presentation result, got: {:?}",
        results
    );
    assert_eq!(
        presentation_results[0].match_field,
        SearchMatchField::TranslationText,
        "match_field should indicate TranslationText caused the match"
    );
}

#[tokio::test]
async fn search_slide_stage_match_returns_parent_presentation() {
    // Slide text only in the stage field — verifies the StageText
    // branch of match_field selection in search_slides.
    // Library and presentation names are intentionally neutral so they
    // don't overlap with the marker tokens and cause false deduplication.
    let state = AppState::in_memory().await.unwrap();
    let library = state.create_library("Hymns Library").await.unwrap();
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "Evening Song", None)
        .await
        .unwrap();
    let slide_id = presentation.slides.first().unwrap().id;
    state
        .update_slide_content(
            presentation.id,
            slide_id,
            String::new(),
            String::new(),
            "test283-stage-marker is in the stage".to_string(),
            None,
            None,
        )
        .await
        .unwrap();

    let app = build_router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/search?query=test283-stage-marker")
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

    let presentation_results: Vec<&SearchResult> = results
        .iter()
        .filter(|result| matches!(result.kind, SearchResultKind::Presentation))
        .collect();
    assert_eq!(
        presentation_results.len(),
        1,
        "expected exactly one Presentation result, got: {:?}",
        results
    );
    assert_eq!(
        presentation_results[0].match_field,
        SearchMatchField::StageText,
        "match_field should indicate StageText caused the match"
    );
}

#[tokio::test]
async fn ndi_client_stats_beacon_returns_no_content() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ndi/client-stats")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "sourceId": "test-src",
                        "displayId": "a1b2c3d4e5f60718",
                        "codec": "video/H264",
                        "profile": "compat",
                        "screen": "1280x720",
                        "framesDecoded": 100,
                        "fps": 30.0,
                        "jitterBufferMs": 12.5,
                        "freezeCount": 0,
                        "framesDropped": 1,
                        "lite": true
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

// ── Stage page (standard WASM shell at /stage) ──────────────────────────────

#[tokio::test]
async fn stage_serves_normal_page_for_default_layout() {
    // Default layout (worship-snv): /stage keeps serving the WASM shell —
    // 200 with the built dist, 503 on a checkout where the dist isn't built.
    // It must NEVER redirect to the lite page.
    let app = build_router(AppState::in_memory().await.unwrap());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/stage")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        matches!(
            response.status(),
            StatusCode::OK | StatusCode::SERVICE_UNAVAILABLE
        ),
        "default layout must serve the normal stage page, got {}",
        response.status()
    );
    assert!(
        response
            .headers()
            .get(axum::http::header::LOCATION)
            .is_none(),
        "default layout must not redirect"
    );
}

#[test]
fn normalize_ws_surface_keeps_known_surface() {
    // A real surface label passes through unchanged so the connect log records it.
    assert_eq!(
        normalize_ws_surface(Some("operator".to_string())),
        "operator"
    );
    assert_eq!(normalize_ws_surface(Some("tablet".to_string())), "tablet");
    assert_eq!(normalize_ws_surface(Some("stage".to_string())), "stage");
}

#[test]
fn normalize_ws_surface_falls_back_to_unknown() {
    // Missing or empty surface query param becomes "unknown" (never a blank label).
    assert_eq!(normalize_ws_surface(None), "unknown");
    assert_eq!(normalize_ws_surface(Some(String::new())), "unknown");
}

#[test]
fn ws_is_preview_only_true_for_one_or_true() {
    // The operator-header preview mirror tags its socket `?preview=1` (#460) so
    // the server excludes it from the stage-monitor count. Only "1"/"true" mark
    // a preview; everything else (missing, empty, "0", "false", noise) is a real
    // stage client that DOES count.
    assert!(ws_is_preview(Some("1".to_string())));
    assert!(ws_is_preview(Some("true".to_string())));
    assert!(!ws_is_preview(None));
    assert!(!ws_is_preview(Some(String::new())));
    assert!(!ws_is_preview(Some("0".to_string())));
    assert!(!ws_is_preview(Some("false".to_string())));
    assert!(!ws_is_preview(Some("yes".to_string())));
}

// #471: serve_websocket must actually run on a /live/ws connection — it logs the
// client IP + surface on connect/disconnect and registers stage presence. A real
// WebSocket client connection is the only way to exercise the upgrade handler +
// serve_websocket body (a `oneshot` request can't perform the WS upgrade), and it
// kills the "replace serve_websocket with ()" mutant: with the body stubbed out,
// the StagePresence is never registered and the assertion below fails.
// Multi-thread runtime: this test runs a real axum server (tokio::spawn) AND a
// real WS client concurrently. On the default current-thread runtime the server
// task is only scheduled cooperatively, so under the full parallel test suite it
// gets starved and the presence never registers in time. A dedicated worker pool
// lets the server and client truly run in parallel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_ws_connection_registers_stage_presence() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let state = AppState::in_memory().await.unwrap();
    let connections = state.stage_connections_handle();
    let app = build_router(state);

    // Bind an ephemeral port and serve the real router.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Connect a real WS client to /live/ws, tagging the surface (#471).
    let url = format!("ws://{addr}/live/ws?surface=stage");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Send a StagePresence — serve_websocket must register it.
    // InboundMessage is tagged snake_case (crates/presenter-core/src/live.rs).
    let client_id = uuid::Uuid::new_v4().to_string();
    let presence = serde_json::json!({
        "type": "stage_presence",
        "client_id": client_id,
        "layout_code": "worship-snv",
    });
    ws.send(WsMessage::Text(presence.to_string().into()))
        .await
        .unwrap();

    // Poll the connection tracker until the client appears (serve_websocket ran
    // its body). The `()` mutant never registers it → this times out → fails.
    // Budget is generous (≈6s, breaks early on success) because this is a real
    // TCP+WebSocket round-trip: under the full parallel test suite the connect +
    // first-message handling can take well over 1s of wall-clock even though the
    // work itself is near-instant in isolation.
    let mut registered = false;
    for _ in 0..300 {
        let snapshot = connections.snapshot().await;
        if snapshot.iter().any(|c| c.id.to_string() == client_id) {
            registered = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        registered,
        "serve_websocket must register the stage presence sent over /live/ws",
    );

    // Closing the socket drives serve_websocket through its disconnect path.
    ws.close(None).await.unwrap();
    server.abort();
}

// #460: a PREVIEW stage client (`/live/ws?surface=stage&preview=1`, the operator
// header's small live mirror) sends a StagePresence just like a real stage TV —
// but the server MUST NOT register it, so it never inflates the operator's
// "N stage displays connected" monitor count. This connects TWO clients: a real
// one (preview off → registers) and a preview one (preview on → excluded), then
// asserts only the real client appears in the tracker. It kills the mutant that
// drops the `if preview` guard (which would register the preview client too).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_ws_preview_client_is_excluded_from_stage_count() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let state = AppState::in_memory().await.unwrap();
    let connections = state.stage_connections_handle();
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let send_presence = |id: &str| {
        serde_json::json!({
            "type": "stage_presence",
            "client_id": id,
            "layout_code": "worship-snv",
        })
        .to_string()
    };

    // Preview mirror — preview on → must NOT register. Connect + send its
    // presence FIRST so it has the head start: under the guard-removal mutant
    // (always register) it would register at ~T0, well before the real client
    // below, so the final snapshot reliably catches it (snapshot.len()==2) and
    // the mutant dies. Sending it LAST would be the worst case — the mutant's
    // late registration could miss the snapshot and survive.
    let preview_id = uuid::Uuid::new_v4().to_string();
    let preview_url = format!("ws://{addr}/live/ws?surface=stage&preview=1");
    let (mut preview_ws, _) = tokio_tungstenite::connect_async(&preview_url)
        .await
        .unwrap();
    preview_ws
        .send(WsMessage::Text(send_presence(&preview_id).into()))
        .await
        .unwrap();

    // Real stage TV — preview off → must register. Connected + sent AFTER the
    // preview, so once it registers the preview's (earlier) presence has had
    // strictly more time to be handled — a sound happens-before barrier.
    let real_id = uuid::Uuid::new_v4().to_string();
    let real_url = format!("ws://{addr}/live/ws?surface=stage");
    let (mut real_ws, _) = tokio_tungstenite::connect_async(&real_url).await.unwrap();
    real_ws
        .send(WsMessage::Text(send_presence(&real_id).into()))
        .await
        .unwrap();

    // Wait until the REAL client registers (the barrier: the preview presence was
    // sent earlier, so by now it too has been handled), then assert the preview
    // client never registered.
    let mut real_registered = false;
    for _ in 0..300 {
        let snapshot = connections.snapshot().await;
        if snapshot.iter().any(|c| c.id.to_string() == real_id) {
            real_registered = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(real_registered, "real stage client must register");

    let snapshot = connections.snapshot().await;
    assert!(
        !snapshot.iter().any(|c| c.id.to_string() == preview_id),
        "preview stage client (?preview=1) must be EXCLUDED from the stage-monitor count"
    );
    assert_eq!(
        snapshot.len(),
        1,
        "only the real stage client counts; the preview must not appear",
    );

    real_ws.close(None).await.unwrap();
    preview_ws.close(None).await.unwrap();
    server.abort();
}
