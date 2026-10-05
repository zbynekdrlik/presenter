//! #814 Companion catalog: the layout + stream scene CHOICES the Bitfocus module
//! builds its dropdowns from — content, change-gated refresh, and the real
//! `/companion/ws` push (sent on connect, re-sent only when a stream config
//! change alters it).

use super::catalog::{
    catalog_layouts, initial_catalog, refresh_catalog, CatalogLayout, CatalogOutput, CatalogScene,
    CompanionCatalog,
};
use super::variables::CompanionVariableState;
use super::*;
use futures_util::{SinkExt, StreamExt};
use presenter_core::SceneKind;
use serde_json::Value;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message as WsMessage;

type ClientWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A fully ISOLATED `AppState` over its own temp-file SQLite DB.
/// `AppState::in_memory()` shares ONE DB process-wide
/// (`.claude/rules/stream-graphics.md`), so a parallel test creating a stream
/// output would change THIS test's catalog and race the "unchanged → no
/// re-send" assertions. Keep the returned `TempDir` alive for the test.
async fn isolated_state(companion_enabled: bool) -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("catalog.db").display()
    );
    let repo = presenter_persistence::Repository::connect(
        &presenter_persistence::DatabaseSettings::new(url),
    )
    .await
    .expect("isolated repository");
    let state = AppState::new(
        repo,
        None,
        companion_enabled,
        18_175,
        crate::resolume::ResolumeRegistry::new().expect("resolume registry"),
        crate::android_stage::AndroidStageRegistry::new(),
        crate::osc::OscBridge::new(&crate::config::OscConfig::default()),
        crate::ableset::AbleSetBridge::new(),
    );
    (state, dir)
}

/// Seed an output with a base scene "Chvaly" and an overlay scene "Verse".
/// Returns `(base_id, overlay_id)`.
async fn seed_output(state: &AppState, slug: &str) -> (i64, i64) {
    let repo = state.repository();
    repo.create_stream_output(slug, "Test 814").await.unwrap();
    let base = repo
        .create_stream_scene(slug, "Chvaly", SceneKind::Base)
        .await
        .unwrap();
    let overlay = repo
        .create_stream_scene(slug, "Verse", SceneKind::Overlay)
        .await
        .unwrap();
    (base.id, overlay.id)
}

/// `(name, kind)` of every scene of `slug` in a serialised catalog message.
fn scenes_of(catalog: &Value, slug: &str) -> Vec<(String, String)> {
    let output = catalog["stream"]
        .as_array()
        .expect("catalog.stream must be an array")
        .iter()
        .find(|output| output["slug"] == slug)
        .unwrap_or_else(|| panic!("catalog has no output {slug:?}: {catalog}"));
    assert_eq!(output["name"], "Test 814");
    output["scenes"]
        .as_array()
        .expect("scenes must be an array")
        .iter()
        .map(|scene| {
            (
                scene["name"].as_str().unwrap().to_string(),
                scene["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(name, kind)| (name.to_string(), kind.to_string()))
        .collect()
}

#[test]
fn catalog_layouts_are_the_operator_selectable_set_incl_api_layouts() {
    let layouts = catalog_layouts();
    let codes: Vec<&str> = layouts.iter().map(|l| l.code.as_str()).collect();
    let expected: Vec<String> = presenter_core::StageDisplayLayout::operator_selectable()
        .into_iter()
        .map(|l| l.code)
        .collect();
    assert_eq!(
        codes, expected,
        "catalog layouts = what stage.layout accepts"
    );
    // The #799 API layouts the old hardcoded module list never had.
    assert!(layouts.contains(&CatalogLayout {
        code: "api".into(),
        name: "API".into(),
    }));
    assert!(layouts.contains(&CatalogLayout {
        code: "api-ambient".into(),
        name: "API + CG VIDEO".into(),
    }));
    // camera-crew is refused by `validate_operator_selectable`, so a dropdown
    // entry for it would be a button that always errors.
    assert!(!codes.contains(&"camera-crew"));
}

#[tokio::test]
async fn catalog_message_carries_layouts_and_stream_scenes() {
    let (state, _dir) = isolated_state(false).await;
    seed_output(&state, "t814-msg").await;

    let catalog = initial_catalog(&state).await;
    let message = serde_json::to_value(OutgoingMessage::Catalog {
        layouts: catalog.layouts,
        stream: catalog.stream,
    })
    .unwrap();

    assert_eq!(message["type"], "catalog");
    let layouts = message["layouts"].as_array().expect("layouts array");
    assert!(
        layouts.contains(&serde_json::json!({ "code": "api-ambient", "name": "API + CG VIDEO" })),
        "{message}"
    );
    assert!(
        layouts.contains(&serde_json::json!({ "code": "worship-snv", "name": "WORSHIP SNV" })),
        "{message}"
    );
    assert_eq!(
        scenes_of(&message, "t814-msg"),
        pairs(&[("Chvaly", "base"), ("Verse", "overlay")]),
    );
}

#[test]
fn apply_catalog_reports_only_content_changes() {
    let catalog = |overlay: &str| CompanionCatalog {
        layouts: catalog_layouts(),
        stream: vec![CatalogOutput {
            slug: "stream".into(),
            name: "Stream".into(),
            scenes: vec![
                CatalogScene {
                    name: "Chvaly".into(),
                    kind: SceneKind::Base,
                },
                CatalogScene {
                    name: overlay.into(),
                    kind: SceneKind::Overlay,
                },
            ],
        }],
    };
    let mut variables = CompanionVariableState::default();
    assert!(variables.apply_catalog(catalog("Verse")), "first catalog");
    assert!(
        !variables.apply_catalog(catalog("Verse")),
        "identical → no send"
    );
    assert!(
        variables.apply_catalog(catalog("Verš")),
        "renamed overlay → send"
    );
    assert_eq!(variables.catalog(), &catalog("Verš"));
}

#[tokio::test]
async fn refresh_catalog_resends_only_when_outputs_or_scenes_change() {
    let (state, _dir) = isolated_state(false).await;
    let (base, _overlay) = seed_output(&state, "t814-refresh").await;
    let mut variables = CompanionVariableState::default();
    variables.apply_catalog(initial_catalog(&state).await);

    // Nothing changed since the snapshot → no re-send.
    assert!(!refresh_catalog(&state, &mut variables).await);

    // A config write that does not touch names/kinds (a scene transition — like
    // an element edit) bumps config_revision but leaves the catalog identical.
    state
        .repository()
        .set_stream_scene_transition(base, Some(250))
        .await
        .unwrap();
    assert!(!refresh_catalog(&state, &mut variables).await);

    // A new overlay scene → re-send, and the stored catalog carries it.
    state
        .repository()
        .create_stream_scene("t814-refresh", "Logo", SceneKind::Overlay)
        .await
        .unwrap();
    assert!(refresh_catalog(&state, &mut variables).await);
    let stored = serde_json::to_value(variables.catalog()).unwrap();
    assert_eq!(
        scenes_of(&stored, "t814-refresh"),
        pairs(&[
            ("Chvaly", "base"),
            ("Verse", "overlay"),
            ("Logo", "overlay")
        ]),
    );

    // A renamed base scene → re-send.
    state
        .repository()
        .rename_stream_scene(base, "Chvály")
        .await
        .unwrap();
    assert!(refresh_catalog(&state, &mut variables).await);
    let stored = serde_json::to_value(variables.catalog()).unwrap();
    assert_eq!(scenes_of(&stored, "t814-refresh")[0].0, "Chvály");
}

/// The next JSON text frame from the companion socket (non-text frames skipped).
async fn next_frame(ws: &mut ClientWs) -> Value {
    loop {
        let frame = timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("timed out waiting for a companion frame")
            .expect("companion socket closed")
            .expect("companion socket error");
        if let WsMessage::Text(text) = frame {
            return serde_json::from_str(text.as_str()).expect("JSON frame");
        }
    }
}

/// Read frames until the next `catalog` message (other message types — welcome,
/// variables, nameplates — are skipped).
async fn next_catalog(ws: &mut ClientWs) -> Value {
    loop {
        let frame = next_frame(ws).await;
        if frame["type"] == "catalog" {
            return frame;
        }
    }
}

/// The `broadcast_live` value carried by a `variables` frame, if any.
fn broadcast_live_value(frame: &Value) -> Option<&str> {
    frame["values"]
        .as_array()?
        .iter()
        .find(|var| var["name"] == "broadcast_live")?["value"]
        .as_str()
}

// The real `/companion/ws` session: the catalog arrives on connect, a
// catalog-neutral config write sends NOTHING (proved with an ordering barrier),
// and a scene add re-sends it.
// Multi-thread runtime: a real axum server + a real WS client run concurrently
// (same reason as `router/tests.rs::live_ws_connection_registers_stage_presence`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn companion_socket_sends_catalog_on_connect_and_resends_on_config_change() {
    let (state, _dir) = isolated_state(true).await;
    let (base, _overlay) = seed_output(&state, "t814-ws").await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let (mut ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}/companion/ws"))
        .await
        .unwrap();
    let hello = serde_json::json!({ "type": "hello", "client": "catalog-test" });
    ws.send(WsMessage::Text(hello.to_string().into()))
        .await
        .unwrap();

    let first = next_catalog(&mut ws).await;
    assert!(
        first["layouts"]
            .as_array()
            .expect("layouts array")
            .iter()
            .any(|layout| layout["code"] == "api"),
        "{first}"
    );
    assert_eq!(
        scenes_of(&first, "t814-ws"),
        pairs(&[("Chvaly", "base"), ("Verse", "overlay")]),
    );

    // A catalog-neutral config write (a scene transition — like an element
    // edit): it bumps config_revision and fires StreamConfigChanged, but the
    // re-resolved catalog is identical, so the session must send NO catalog.
    state
        .repository()
        .set_stream_scene_transition(base, Some(250))
        .await
        .unwrap();
    state.stream_config_write_notify("t814-ws").await.unwrap();
    // Ordering barrier: the session subscribed to the live hub before its
    // initial snapshot and handles live events one at a time in hub order, so
    // the `variables` frame for this broadcast toggle arrives only AFTER the
    // notify above was fully handled. Any `catalog` frame before it means the
    // neutral write re-sent — the change gate in `handle_live_event` is broken.
    state.set_broadcast_live(true);
    loop {
        let frame = next_frame(&mut ws).await;
        assert_ne!(
            frame["type"], "catalog",
            "a catalog-neutral config write must send no catalog: {frame}"
        );
        if frame["type"] == "variables" && broadcast_live_value(&frame) == Some("true") {
            break;
        }
    }

    // A new overlay scene → the catalog is re-sent, carrying it.
    state
        .repository()
        .create_stream_scene("t814-ws", "Logo", SceneKind::Overlay)
        .await
        .unwrap();
    state.stream_config_write_notify("t814-ws").await.unwrap();
    let second = next_catalog(&mut ws).await;
    assert_eq!(
        scenes_of(&second, "t814-ws"),
        pairs(&[
            ("Chvaly", "base"),
            ("Verse", "overlay"),
            ("Logo", "overlay")
        ]),
    );

    ws.close(None).await.unwrap();
    server.abort();
}
