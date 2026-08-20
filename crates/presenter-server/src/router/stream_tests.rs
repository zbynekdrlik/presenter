//! Full-router integration tests for the stream-graphics REST API (#707),
//! driving `build_router(AppState::in_memory())` via `tower::oneshot` — the
//! same harness the rest of `router/tests.rs` uses. Each test operates on its
//! OWN uniquely-slugged output (or asserts only existence on the seeded
//! `stream` output) so it is independent of the in-memory DB by construction.

use crate::router::build_router;
use crate::state::AppState;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

/// Send one request; return `(status, json_body)` (Null body when empty).
async fn req(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(b) => builder
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(request).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

fn text_style() -> Value {
    json!({
        "fontFamily": "Inter", "sizePct": 8.0, "color": "#ffffff",
        "weight": 700, "align": "center", "lineHeight": 1.2
    })
}

fn frame() -> Value {
    json!({"xPct": 10.0, "yPct": 20.0, "wPct": 50.0, "hPct": 30.0})
}

fn image_props() -> Value {
    json!({"kind": "image", "asset_id": 7, "fit": "contain", "frame": frame(), "opacity": 1.0})
}

fn countdown_props() -> Value {
    json!({"kind": "countdown", "timer_id": 3, "style": text_style(), "frame": frame(),
           "content_transition": {"mode": "cut"}})
}

fn lyrics_props() -> Value {
    json!({"kind": "lyrics", "show_main": true, "show_translation": false,
           "main_style": text_style(), "translation_style": text_style(), "frame": frame()})
}

fn verse_props() -> Value {
    json!({"kind": "verse", "show_secondary": true, "text_style": text_style(),
           "secondary_style": text_style(), "reference_style": text_style(), "frame": frame()})
}

/// Create a fresh, uniquely-slugged output; return the app + the slug.
async fn app_with_output(slug: &str) -> Router {
    let app = build_router(AppState::in_memory().await.unwrap());
    let (status, _) = req(
        &app,
        Method::POST,
        "/stream/api/outputs",
        Some(json!({"slug": slug, "name": "Test"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create output {slug}");
    app
}

#[tokio::test]
async fn def_of_seeded_output_returns_200() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let (status, def) = req(&app, Method::GET, "/stream/api/outputs/stream/def", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(def["slug"], json!("stream"));
    assert!(def["scenes"].is_array(), "def carries a scenes array");
    assert!(def["configRevision"].is_number());
}

#[tokio::test]
async fn outputs_list_includes_seeded_stream() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let (status, list) = req(&app, Method::GET, "/stream/api/outputs", None).await;
    assert_eq!(status, StatusCode::OK);
    let slugs: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["slug"].as_str())
        .collect();
    assert!(
        slugs.contains(&"stream"),
        "seed output present, got {slugs:?}"
    );
}

#[tokio::test]
async fn create_output_then_patch_renames() {
    let app = app_with_output("t-patch").await;
    let (status, summary) = req(
        &app,
        Method::PATCH,
        "/stream/api/outputs/t-patch",
        Some(json!({"name": "Renamed", "defaultTransitionMs": 600})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(summary["name"], json!("Renamed"));
    assert_eq!(summary["defaultTransitionMs"], json!(600));
}

#[tokio::test]
async fn full_scene_and_element_flow_with_def_ordering() {
    let app = app_with_output("t-flow").await;

    // Overlay created FIRST, base SECOND — the def must still order base first.
    let (_, overlay) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-flow/scenes",
        Some(json!({"name": "Overlay", "kind": "overlay"})),
    )
    .await;
    let (_, base) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-flow/scenes",
        Some(json!({"name": "Base", "kind": "base"})),
    )
    .await;
    let base_id = base["id"].as_i64().unwrap();
    assert!(overlay["id"].is_number());

    // All four element kinds on the base scene, in this order.
    for props in [
        image_props(),
        countdown_props(),
        lyrics_props(),
        verse_props(),
    ] {
        let (status, _) = req(
            &app,
            Method::POST,
            &format!("/stream/api/scenes/{base_id}/elements"),
            Some(props),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create element");
    }

    let (status, def) = req(&app, Method::GET, "/stream/api/outputs/t-flow/def", None).await;
    assert_eq!(status, StatusCode::OK);
    let scenes = def["scenes"].as_array().unwrap();
    assert_eq!(scenes.len(), 2);
    // Base before overlay (kind rank), regardless of creation order.
    assert_eq!(scenes[0]["kind"], json!("base"));
    assert_eq!(scenes[1]["kind"], json!("overlay"));
    // Elements ordered by z_order = creation order.
    let elements = scenes[0]["elements"].as_array().unwrap();
    let kinds: Vec<&str> = elements
        .iter()
        .filter_map(|e| e["props"]["kind"].as_str())
        .collect();
    assert_eq!(kinds, vec!["image", "countdown", "lyrics", "verse"]);
    let z: Vec<i64> = elements
        .iter()
        .filter_map(|e| e["zOrder"].as_i64())
        .collect();
    assert_eq!(z, vec![0, 1, 2, 3], "elements ordered by z_order");
}

#[tokio::test]
async fn reorder_scenes_reflected_in_def() {
    let app = app_with_output("t-reorder").await;
    let (_, a) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-reorder/scenes",
        Some(json!({"name": "A", "kind": "base"})),
    )
    .await;
    let (_, b) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-reorder/scenes",
        Some(json!({"name": "B", "kind": "base"})),
    )
    .await;
    let (a_id, b_id) = (a["id"].as_i64().unwrap(), b["id"].as_i64().unwrap());

    // Reverse the order (full set required by the repository).
    let (status, _) = req(
        &app,
        Method::PUT,
        "/stream/api/outputs/t-reorder/scenes/order",
        Some(json!({"ids": [b_id, a_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, def) = req(&app, Method::GET, "/stream/api/outputs/t-reorder/def", None).await;
    let ids: Vec<i64> = def["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["id"].as_i64())
        .collect();
    assert_eq!(ids, vec![b_id, a_id], "def reflects the new L->R order");
}

#[tokio::test]
async fn activation_flow_reflected_in_def_read() {
    let app = app_with_output("t-activate").await;
    let (_, base) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-activate/scenes",
        Some(json!({"name": "Base", "kind": "base"})),
    )
    .await;
    let (_, overlay) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-activate/scenes",
        Some(json!({"name": "Overlay", "kind": "overlay"})),
    )
    .await;
    let base_id = base["id"].as_i64().unwrap();
    let overlay_id = overlay["id"].as_i64().unwrap();

    // Set base + overlay on.
    let (s1, show) = req(
        &app,
        Method::PUT,
        "/stream/api/outputs/t-activate/active-scene",
        Some(json!({"sceneId": base_id})),
    )
    .await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(show["activeSceneId"], json!(base_id));
    let (s2, _) = req(
        &app,
        Method::PUT,
        &format!("/stream/api/outputs/t-activate/overlays/{overlay_id}"),
        Some(json!({"active": true})),
    )
    .await;
    assert_eq!(s2, StatusCode::OK);

    // A def read reflects both.
    let (_, def) = req(
        &app,
        Method::GET,
        "/stream/api/outputs/t-activate/def",
        None,
    )
    .await;
    assert_eq!(def["activeSceneId"], json!(base_id));
    let overlay_active = def["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"].as_i64() == Some(overlay_id))
        .map(|s| s["isActive"].as_bool().unwrap())
        .unwrap();
    assert!(overlay_active, "overlay is_active in the def");

    // Clear resets both.
    let (s3, cleared) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-activate/clear",
        None,
    )
    .await;
    assert_eq!(s3, StatusCode::OK);
    assert_eq!(cleared["activeSceneId"], Value::Null);
    let (_, def2) = req(
        &app,
        Method::GET,
        "/stream/api/outputs/t-activate/def",
        None,
    )
    .await;
    assert_eq!(def2["activeSceneId"], Value::Null);
}

#[tokio::test]
async fn unknown_slug_def_is_404() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let (status, _) = req(
        &app,
        Method::GET,
        "/stream/api/outputs/does-not-exist/def",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn duplicate_scene_name_is_409() {
    let app = app_with_output("t-dup").await;
    let (s1, _) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-dup/scenes",
        Some(json!({"name": "Base", "kind": "base"})),
    )
    .await;
    assert_eq!(s1, StatusCode::OK);
    // Case-insensitive duplicate.
    let (s2, _) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-dup/scenes",
        Some(json!({"name": "base", "kind": "overlay"})),
    )
    .await;
    assert_eq!(s2, StatusCode::CONFLICT, "duplicate scene name -> 409");
}

#[tokio::test]
async fn duplicate_output_slug_is_409() {
    let app = app_with_output("t-dupout").await;
    let (status, _) = req(
        &app,
        Method::POST,
        "/stream/api/outputs",
        Some(json!({"slug": "t-dupout", "name": "Again"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn invalid_element_props_is_422() {
    let app = app_with_output("t-badprops").await;
    let (_, base) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-badprops/scenes",
        Some(json!({"name": "Base", "kind": "base"})),
    )
    .await;
    let base_id = base["id"].as_i64().unwrap();
    // opacity 5.0 is out of the 0..=1 range -> core validation -> Invalid -> 422.
    let bad = json!({"kind": "image", "asset_id": 1, "fit": "cover",
                     "frame": frame(), "opacity": 5.0});
    let (status, _) = req(
        &app,
        Method::POST,
        &format!("/stream/api/scenes/{base_id}/elements"),
        Some(bad),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn wrong_kind_activation_is_422() {
    let app = app_with_output("t-wrongkind").await;
    let (_, overlay) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-wrongkind/scenes",
        Some(json!({"name": "Overlay", "kind": "overlay"})),
    )
    .await;
    let overlay_id = overlay["id"].as_i64().unwrap();
    // Activating an overlay AS the base scene is semantically invalid -> 422.
    let (status, _) = req(
        &app,
        Method::PUT,
        "/stream/api/outputs/t-wrongkind/active-scene",
        Some(json!({"sceneId": overlay_id})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn element_on_unknown_scene_is_404() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let (status, _) = req(
        &app,
        Method::POST,
        "/stream/api/scenes/999999/elements",
        Some(image_props()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn scene_delete_then_gone_from_def() {
    let app = app_with_output("t-del").await;
    let (_, scene) = req(
        &app,
        Method::POST,
        "/stream/api/outputs/t-del/scenes",
        Some(json!({"name": "Base", "kind": "base"})),
    )
    .await;
    let scene_id = scene["id"].as_i64().unwrap();
    let (status, _) = req(
        &app,
        Method::DELETE,
        &format!("/stream/api/scenes/{scene_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, def) = req(&app, Method::GET, "/stream/api/outputs/t-del/def", None).await;
    assert!(
        def["scenes"].as_array().unwrap().is_empty(),
        "scene removed from def"
    );
}
