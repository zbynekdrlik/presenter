//! The `/ui/study` page (#851 spec §6.1): the WASM shell, its web app manifest
//! and its service worker. The icons are the tablet's. The page reads only;
//! nothing here (or on the page) reaches the walls.

use axum::{http::header, response::IntoResponse, routing::get, Json, Router};

use super::wasm_ui;
use crate::state::AppState;

const SERVICE_WORKER: &str = include_str!("../ui/study_sw.js");

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/ui/study", get(wasm_ui::wasm_ui_shell))
        .route("/ui/study/manifest.json", get(study_manifest))
        .route("/ui/study/sw.js", get(service_worker))
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
            { "src": "/ui/tablet/icon-192.png", "sizes": "192x192", "type": "image/png" },
            { "src": "/ui/tablet/icon-512.png", "sizes": "512x512", "type": "image/png" }
        ]
    });
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        Json(manifest),
    )
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
    }

    #[test]
    fn the_service_worker_caches_nothing_and_handles_no_request() {
        assert!(!SERVICE_WORKER.contains("addEventListener(\"fetch\""));
        assert!(!SERVICE_WORKER.contains("caches."));
    }
}
