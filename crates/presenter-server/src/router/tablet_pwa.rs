use axum::{http::header, response::IntoResponse, Json};

const ICON_192: &[u8] = include_bytes!("../ui/tablet_icons/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../ui/tablet_icons/icon-512.png");
const APPLE_TOUCH_ICON: &[u8] = include_bytes!("../ui/tablet_icons/apple-touch-icon.png");
const SERVICE_WORKER: &str = include_str!("../ui/tablet_sw.js");

pub async fn tablet_manifest() -> impl IntoResponse {
    let manifest = serde_json::json!({
        "id": "/ui/tablet",
        "name": "Bible Tablet",
        "short_name": "Bible",
        "description": "Touch-optimized Bible controller",
        "start_url": "/ui/tablet",
        "scope": "/ui/tablet",
        "display": "standalone",
        // #699: the Bible Tablet FOLLOWS device rotation and is usable in both
        // portrait and landscape (reversing the #569/#638 landscape lock — the
        // owner now wants rotation to work). "any" lets an installed/standalone
        // PWA render at whatever orientation the device is held in; the former
        // force-rotate CSS + screen.orientation.lock gesture were removed too
        // (see tablet.css / the deleted tablet_orientation.rs).
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

/// Serve `/favicon.ico` for every route.
///
/// Browsers automatically request `/favicon.ico` regardless of any `<link rel="icon">`,
/// so without this handler every page (operator, tablet, stage, bible…) logged a 404
/// console error. We reuse the embedded 192px app icon (served as PNG, which all modern
/// browsers accept for a favicon) rather than shipping a separate `.ico` binary.
pub async fn favicon() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        ICON_192,
    )
}
