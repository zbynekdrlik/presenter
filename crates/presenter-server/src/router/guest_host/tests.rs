//! The guest hostname gets only the guest surface; the normal hosts never get
//! the guest page (#999 ROZHODNUTÉ 27.11). Through the real `build_router`.

use axum::body::Body;
use axum::http::{Request as HttpRequest, StatusCode};
use tower::ServiceExt;

use crate::listen::translate::stats::DayMeter;
use crate::listen::translate::test_fake::{guest, FakeGemini};
use crate::router::build_router;
use crate::state::AppState;

const GUEST: &str = "listen.example.org";

async fn router(with_host: bool) -> axum::Router {
    let state = AppState::in_memory().await.unwrap();
    let fake = FakeGemini::new(vec![]);
    let service = guest(&fake, Some("test-key-not-real"), DayMeter::default());
    if with_host {
        state.set_guest_translate(service);
    } else {
        let mut env = crate::listen::translate::test_fake::env(Some("test-key-not-real"));
        env.guest_host = None;
        state.set_guest_translate(crate::listen::translate::GuestTranslate::with_deps(
            crate::listen::translate::session::SessionDeps {
                env,
                connector: std::sync::Arc::new(
                    crate::listen::translate::test_fake::DuplexConnector(fake),
                ),
                meter: DayMeter::default(),
                today: crate::listen::translate::test_fake::today,
            },
        ));
    }
    build_router(state)
}

async fn get(
    app: &axum::Router,
    host: &str,
    path: &str,
    forwarded: Option<&str>,
) -> (StatusCode, String) {
    let mut request = HttpRequest::get(path).header("host", host);
    if let Some(forwarded) = forwarded {
        request = request.header("x-forwarded-host", forwarded);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn the_guest_host_serves_only_the_guest_surface() {
    let app = router(true).await;
    let shell = super::super::wasm_ui::wasm_ui_shell().await;
    let shell_status = shell.status();
    let shell_body = String::from_utf8_lossy(
        &axum::body::to_bytes(shell.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned();
    let (status, body) = get(&app, GUEST, "/", None).await;
    assert_eq!(
        (status, body),
        (shell_status, shell_body.clone()),
        "the root is the guest page"
    );
    let (status, body) = get(&app, "Listen.Example.org:443", "/manifest.json", None).await;
    assert_eq!(status, StatusCode::OK);
    let manifest: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(manifest["start_url"], "/");
    assert_eq!(manifest["scope"], "/");
    for path in [
        "/sw.js",
        "/icon-192.png",
        "/icon-512.png",
        "/healthz",
        "/listen/guest/languages",
    ] {
        assert_eq!(
            get(&app, GUEST, path, None).await.0,
            StatusCode::OK,
            "{path}"
        );
    }
    for path in [
        "/ui/operator",
        "/ui/settings",
        "/listen/status",
        "/listen/mics",
        "/listen/guest/qr",
        "/ui/guest-qr",
        "/bible/presentations",
        "/libraries/summary",
        "/stage",
        "/nothing",
    ] {
        // the GUEST router's 404 (its own body), not a normal route that happens to 404
        assert_eq!(
            get(&app, GUEST, path, None).await,
            (
                StatusCode::NOT_FOUND,
                super::super::listen_guest::GUEST_NOT_FOUND.to_string()
            ),
            "{path}"
        );
    }
    // a path parameter of the guest router is its own, never the normal
    // router's (a `Router::layer` dispatch answered 500 here)
    assert_eq!(
        get(&app, GUEST, "/ui-pkg/no-such-asset.js", None).await,
        (StatusCode::NOT_FOUND, "Asset not found".to_string())
    );
    // an absolute-form request naming the guest host is a guest request too
    let absolute = HttpRequest::get(format!("http://{GUEST}/ui/operator"))
        .header("host", "10.77.9.205")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(absolute).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    // the public health shows the version, never NDI or the AI backend
    let (_, health) = get(&app, GUEST, "/healthz", None).await;
    let health: serde_json::Value = serde_json::from_str(&health).unwrap();
    assert_eq!(health["status"], "ok");
    assert!(health["version"].is_string());
    assert!(
        health.get("ndi_pipelines").is_none() && health.get("ai").is_none(),
        "{health}"
    );

    // the normal hosts: the home page at `/`, never the guest page or its manifest
    let (status, home) = get(&app, "10.77.9.205", "/", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(home, shell_body);
    assert_eq!(
        get(&app, "10.77.9.205", "/manifest.json", None).await.0,
        StatusCode::NOT_FOUND
    );
    // X-Forwarded-Host never moves a request to the guest router
    let (_, forwarded) = get(&app, "10.77.9.205", "/", Some(GUEST)).await;
    assert_eq!(forwarded, home);
    assert_eq!(
        get(&app, "10.77.9.205", "/listen/status", Some(GUEST))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn without_a_guest_host_nothing_changes() {
    let app = router(false).await;
    let (status, body) = get(&app, GUEST, "/", None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, home) = get(&app, "10.77.9.205", "/", None).await;
    assert_eq!(body, home);
    assert_eq!(
        get(&app, GUEST, "/listen/status", None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        get(&app, GUEST, "/manifest.json", None).await.0,
        StatusCode::NOT_FOUND
    );
}
