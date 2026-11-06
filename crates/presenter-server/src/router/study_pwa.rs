//! The `/ui/study` page (#851 spec §6.1): the WASM shell, its web app manifest
//! its service worker and its own icons (an open book, so the app on the home
//! screen is not mistaken for the tablet's "B"; owner 2026-11-06). The page
//! reads only; nothing here (or on the page) reaches the walls.

use axum::{http::header, response::IntoResponse, routing::get, Json, Router};

use super::wasm_ui;
use crate::state::AppState;

const SERVICE_WORKER: &str = include_str!("../ui/study_sw.js");
const ICON_192: &[u8] = include_bytes!("../ui/study_icons/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../ui/study_icons/icon-512.png");
const APPLE_TOUCH_ICON: &[u8] = include_bytes!("../ui/study_icons/apple-touch-icon.png");

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/ui/study", get(wasm_ui::wasm_ui_shell))
        .route("/ui/study/manifest.json", get(study_manifest))
        .route("/ui/study/sw.js", get(service_worker))
        .route("/ui/study/icon-192.png", get(|| png(ICON_192)))
        .route("/ui/study/icon-512.png", get(|| png(ICON_512)))
        .route(
            "/ui/study/apple-touch-icon.png",
            get(|| png(APPLE_TOUCH_ICON)),
        )
}

async fn study_manifest() -> impl IntoResponse {
    let manifest = serde_json::json!({
        "id": "/ui/study",
        "name": "Presenter Štúdium",
        "short_name": "Štúdium",
        "description": "Čítanie a hľadanie v Biblii",
        "start_url": "/ui/study",
        "scope": "/ui/study",
        "display": "standalone",
        "orientation": "any",
        "background_color": "#0f172a",
        "theme_color": "#0f172a",
        "icons": [
            { "src": "/ui/study/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any" },
            { "src": "/ui/study/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any" },
            { "src": "/ui/study/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
        ]
    });
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        Json(manifest),
    )
}

async fn png(bytes: &'static [u8]) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], bytes)
}

async fn service_worker() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        SERVICE_WORKER,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_manifest_installs_the_study_page_under_its_own_scope() {
        let response = study_manifest().await.into_response();
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .map(|value| value.as_bytes()),
            Some(b"application/manifest+json".as_slice())
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the manifest body");
        let manifest: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
        assert_eq!(manifest["id"], "/ui/study");
        assert_eq!(manifest["scope"], "/ui/study");
        assert_eq!(manifest["start_url"], "/ui/study");
        assert_eq!(manifest["short_name"], "Štúdium");
        assert_eq!(manifest["orientation"], "any");
        let icons = manifest["icons"].as_array().expect("icons");
        assert!(icons.iter().all(|icon| icon["src"]
            .as_str()
            .is_some_and(|src| src.starts_with("/ui/study/"))));
        assert!(icons.iter().any(|icon| icon["purpose"] == "maskable"));
    }

    #[tokio::test]
    async fn the_study_icons_are_served_as_its_own_pngs() {
        use tower::ServiceExt;
        let state = crate::state::AppState::in_memory().await.expect("state");
        let app = routes().with_state(state);
        for (path, size) in [
            ("/ui/study/icon-192.png", 192),
            ("/ui/study/icon-512.png", 512),
            ("/ui/study/apple-touch-icon.png", 180),
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::get(path)
                        .body(axum::body::Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), 200, "{path}");
            assert_eq!(
                response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .map(|v| v.as_bytes()),
                Some(b"image/png".as_slice()),
                "{path}"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            assert_eq!(&body[..8], b"\x89PNG\r\n\x1a\n", "{path}");
            // IHDR width, big-endian, at bytes 16..20.
            let width = u32::from_be_bytes([body[16], body[17], body[18], body[19]]);
            assert_eq!(width, size, "{path}");
        }
        assert_ne!(
            ICON_192,
            include_bytes!("../ui/tablet_icons/icon-192.png").as_slice()
        );
    }

    #[test]
    fn the_service_worker_caches_nothing_and_handles_no_request() {
        assert!(!SERVICE_WORKER.contains("\"fetch\""));
        assert!(!SERVICE_WORKER.contains("'fetch'"));
        assert!(!SERVICE_WORKER.contains("caches."));
    }
}
