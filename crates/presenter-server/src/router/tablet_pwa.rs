use axum::{http::header, response::IntoResponse, Json};

const ICON_192: &[u8] = include_bytes!("../ui/tablet_icons/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../ui/tablet_icons/icon-512.png");
const APPLE_TOUCH_ICON: &[u8] = include_bytes!("../ui/tablet_icons/apple-touch-icon.png");
const SERVICE_WORKER: &str = include_str!("../ui/tablet_sw.js");

pub async fn tablet_manifest() -> impl IntoResponse {
    let manifest = serde_json::json!({
        "name": "Bible Tablet",
        "short_name": "Bible",
        "description": "Touch-optimized Bible controller",
        "start_url": "/ui/tablet",
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

pub async fn icon_192() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], ICON_192)
}

pub async fn icon_512() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], ICON_512)
}

pub async fn apple_touch_icon() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], APPLE_TOUCH_ICON)
}

pub async fn service_worker() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        SERVICE_WORKER,
    )
}
