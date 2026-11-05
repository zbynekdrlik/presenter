//! Routes for serving the WASM-based UI at `/ui/operator`.

use axum::{
    extract::Path,
    http::{header, HeaderMap, HeaderValue, StatusCode},
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
/// #850 B1c: `scripts/build-ui.sh` writes `.br` and `.gz` siblings of every
/// `.wasm` / `.js` / `.css`; the one the client accepts is sent as it is (a
/// 5.4 MB wasm is 1.3 MB as brotli, 1.9 MB as gzip), never compressed per
/// request on the N100.
pub(super) async fn wasm_ui_asset(Path(path): Path<String>, headers: HeaderMap) -> Response {
    let accept = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let Some(asset) = pick_asset(&path, accept, |p| {
        WASM_UI_DIR.get_file(p).map(|file| file.contents())
    }) else {
        return (StatusCode::NOT_FOUND, "Asset not found").into_response();
    };
    // Trunk fingerprints asset filenames (presenter-ui-<hash>.js/.wasm),
    // so a given URL's bytes never change — cache them aggressively.
    let mut response = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime_from_path(&path)),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        asset.body,
    )
        .into_response();
    let response_headers = response.headers_mut();
    if let Some(coding) = asset.encoding {
        response_headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static(coding));
    }
    if asset.varies {
        response_headers.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    }
    response
}

/// The bytes one asset request gets.
#[derive(Debug, PartialEq, Eq)]
struct PickedAsset<'a> {
    body: &'a [u8],
    /// `br` / `gzip` for a precompressed sibling, `None` for the file itself.
    encoding: Option<&'static str>,
    /// A sibling exists, so the answer depends on Accept-Encoding.
    varies: bool,
}

/// `<path>.br` when the client accepts brotli, else `<path>.gz` when it
/// accepts gzip, else the file; `None` when the file does not exist.
fn pick_asset<'a>(
    path: &str,
    accept: &str,
    lookup: impl Fn(&str) -> Option<&'a [u8]>,
) -> Option<PickedAsset<'a>> {
    let original = lookup(path)?;
    let mut varies = false;
    for (coding, suffix) in [("br", ".br"), ("gzip", ".gz")] {
        if let Some(body) = lookup(&format!("{path}{suffix}")) {
            varies = true;
            if accepts(accept, coding) {
                return Some(PickedAsset {
                    body,
                    encoding: Some(coding),
                    varies,
                });
            }
        }
    }
    Some(PickedAsset {
        body: original,
        encoding: None,
        varies,
    })
}

/// Whether an Accept-Encoding value allows `coding`: named without `q=0`,
/// or not named and allowed by `*`. An explicit entry wins over `*`.
fn accepts(accept: &str, coding: &str) -> bool {
    let mut wildcard = false;
    for entry in accept.split(',') {
        let mut parts = entry.split(';');
        let name = parts.next().unwrap_or_default().trim();
        let allowed = !parts.any(is_zero_quality);
        if name.eq_ignore_ascii_case(coding) {
            return allowed;
        }
        if name == "*" {
            wildcard = allowed;
        }
    }
    wildcard
}

fn is_zero_quality(parameter: &str) -> bool {
    let parameter = parameter.trim();
    parameter
        .strip_prefix("q=")
        .or_else(|| parameter.strip_prefix("Q="))
        .and_then(|q| q.trim().parse::<f32>().ok())
        .is_some_and(|q| q <= 0.0)
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
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".woff") {
        "font/woff"
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const WASM: &[u8] = b"raw wasm";
    const WASM_BR: &[u8] = b"brotli";
    const WASM_GZ: &[u8] = b"gzip";

    fn dist(files: &[(&'static str, &'static [u8])]) -> HashMap<&'static str, &'static [u8]> {
        files.iter().copied().collect()
    }

    fn pick(
        files: &HashMap<&'static str, &'static [u8]>,
        path: &str,
        accept: &str,
    ) -> Option<PickedAsset<'static>> {
        pick_asset(path, accept, |p| files.get(p).copied())
    }

    #[test]
    fn brotli_wins_then_gzip_then_the_file_itself() {
        let files = dist(&[
            ("a.wasm", WASM),
            ("a.wasm.br", WASM_BR),
            ("a.wasm.gz", WASM_GZ),
        ]);
        let chrome = pick(&files, "a.wasm", "gzip, deflate, br, zstd").unwrap();
        assert_eq!(
            (chrome.body, chrome.encoding, chrome.varies),
            (WASM_BR, Some("br"), true)
        );
        let old = pick(&files, "a.wasm", "gzip, deflate").unwrap();
        assert_eq!((old.body, old.encoding), (WASM_GZ, Some("gzip")));
        let none = pick(&files, "a.wasm", "").unwrap();
        assert_eq!((none.body, none.encoding, none.varies), (WASM, None, true));
    }

    #[test]
    fn q_zero_refuses_and_an_explicit_entry_beats_the_wildcard() {
        let files = dist(&[("a.js", WASM), ("a.js.br", WASM_BR), ("a.js.gz", WASM_GZ)]);
        assert_eq!(
            pick(&files, "a.js", "br;q=0, gzip").unwrap().encoding,
            Some("gzip")
        );
        assert_eq!(
            pick(&files, "a.js", "br;q=0, *").unwrap().encoding,
            Some("gzip")
        );
        assert_eq!(pick(&files, "a.js", "*").unwrap().encoding, Some("br"));
        assert_eq!(
            pick(&files, "a.js", "BR;q=0.5").unwrap().encoding,
            Some("br")
        );
        assert_eq!(pick(&files, "a.js", "identity").unwrap().encoding, None);
        assert_eq!(
            pick(&files, "a.js", "br;Q=0, gzip").unwrap().encoding,
            Some("gzip")
        );
    }

    #[test]
    fn a_file_without_siblings_is_sent_as_it_is_and_a_missing_one_is_none() {
        let files = dist(&[("index.html", WASM)]);
        let html = pick(&files, "index.html", "br").unwrap();
        assert_eq!((html.body, html.encoding, html.varies), (WASM, None, false));
        assert_eq!(pick(&files, "gone.wasm", "br"), None);
    }
}
