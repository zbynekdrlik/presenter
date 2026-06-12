//! Lite NDI stage page — plain-JS WHEP player for weak displays.
//!
//! The 1GB Vestel stage TVs stall repeatedly on the full WASM stage page
//! regardless of codec, while VDO.Ninja (a small plain-JS page) has played
//! on the SAME TVs for years. This module serves a single self-contained
//! HTML page (no WASM, no frameworks) that mirrors the WASM client's WHEP
//! connect + watchdog semantics, to test whether dropping the WASM app
//! frees enough RAM/CPU for sustained decode.

use axum::{
    extract::State,
    http::header,
    response::{Html, IntoResponse, Redirect, Response},
};
use tracing::instrument;

use crate::state::AppState;

/// The self-contained lite player page, embedded at compile time (same
/// convention as `settings_script.js` / the tablet service worker).
const STAGE_LITE_HTML: &str = include_str!("../assets/stage_lite.html");

/// Stage layout code that routes `/stage` to the lite player.
const NDI_FULLSCREEN_LAYOUT_CODE: &str = "ndi-fullscreen";

/// `GET /stage/lite` — serve the embedded plain-JS WHEP player.
///
/// `Cache-Control: no-store` for the same reason as the WASM shell: an
/// always-on stage TV must pick up a new page on every deploy without a
/// manual hard-refresh.
#[instrument(skip_all)]
pub(super) async fn stage_lite_page() -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Html(STAGE_LITE_HTML)).into_response()
}

/// `GET /stage` — the stage display entry point.
///
/// EXPERIMENT (see issue #379): while the `ndi-fullscreen` layout is active,
/// EVERY stage display is 303-redirected to the lite plain-JS player at
/// `/stage/lite` instead of the WASM app, so the weak TVs get the
/// minimal-RAM page whenever the NDI layout is live. Any other layout
/// (worship, timer, …) serves the normal WASM shell unchanged.
// TODO(#379): decide the permanent UX — overlays on the lite page vs
// per-device routing — then either fold the lite page into the layout
// system or remove this redirect.
#[instrument(skip_all)]
pub(super) async fn stage_shell(State(state): State<AppState>) -> Response {
    if state.stage_layout_code().await == NDI_FULLSCREEN_LAYOUT_CODE {
        return Redirect::to("/stage/lite").into_response();
    }
    super::wasm_ui::wasm_ui_shell().await
}
