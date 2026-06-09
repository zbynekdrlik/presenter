//! Routes for serving the WASM-based UI at `/ui/operator`.

use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};
use include_dir::{include_dir, Dir};

/// Embedded WASM UI assets built by Trunk.
/// The `dist/` directory is created by running `trunk build` in `crates/presenter-ui/`.
/// If the directory doesn't exist yet, the build still compiles — routes return 503.
static WASM_UI_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../presenter-ui/dist");

/// Serve the WASM app shell for `/ui/operator` routes.
/// The WASM app handles client-side routing internally.
pub(super) async fn wasm_ui_shell() -> Response {
    serve_index_html()
}

/// Serve the WASM app shell for routes with a sub-path like `/ui/operator/bible`.
pub(super) async fn wasm_ui_shell_with_path(Path(_path): Path<String>) -> Response {
    serve_index_html()
}

/// Serve static assets from the embedded WASM build directory.
/// Handles paths like `/pkg/presenter_ui_bg.wasm`, `/pkg/presenter_ui.js`, CSS files, etc.
pub(super) async fn wasm_ui_asset(Path(path): Path<String>) -> Response {
    match WASM_UI_DIR.get_file(&path) {
        Some(file) => {
            let content_type = mime_from_path(&path);
            // Trunk fingerprints asset filenames (presenter-ui-<hash>.js/.wasm),
            // so a given URL's bytes never change — cache them aggressively.
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, content_type),
                    (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
                ],
                file.contents(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "Asset not found").into_response(),
    }
}

/// Serve the `index.html` from the embedded WASM build.
///
/// `Cache-Control: no-store` is deliberate: index.html references the
/// content-hashed WASM/JS bundle by filename, so it MUST always be re-fetched —
/// otherwise a browser (especially an always-on stage-display TV) keeps a
/// cached index pointing at the OLD bundle hash and never picks up a deploy,
/// even though the new bundle is sitting right there. Without this, every
/// deploy required a manual hard-refresh on every display to take effect.
fn serve_index_html() -> Response {
    match WASM_UI_DIR.get_file("index.html") {
        Some(file) => {
            let html = String::from_utf8_lossy(file.contents());
            (
                [(header::CACHE_CONTROL, "no-store")],
                Html(html.into_owned()),
            )
                .into_response()
        }
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Html(
                "<!DOCTYPE html><html><body>\
                 <h1>WASM UI not built yet</h1>\
                 <p>Run <code>trunk build</code> in <code>crates/presenter-ui/</code> first.</p>\
                 </body></html>"
                    .to_string(),
            ),
        )
            .into_response(),
    }
}

/// Determine MIME type from file extension.
fn mime_from_path(path: &str) -> &'static str {
    if path.ends_with(".wasm") {
        "application/wasm"
    } else if path.ends_with(".js") {
        "application/javascript"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".html") {
        "text/html"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else {
        "application/octet-stream"
    }
}
