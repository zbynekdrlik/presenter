//! Lite NDI stage page — plain-JS WHEP player for weak displays.
//!
//! The 1GB Vestel stage TVs hitch ~400ms every ~20s on the full WASM stage
//! page: the WASM page's periodic render work (the status bar's `autofit_effect`
//! synchronous reflow + Leptos churn) stalls the WebView compositor. This module
//! serves a single self-contained HTML page (no WASM, no frameworks) that
//! mirrors the WASM client's WHEP connect + watchdog semantics AND replicates
//! the NDI-fullscreen overlay (clock / song number / connection / version) as
//! lightweight FIXED-size plain DOM — no autofit, no per-tick reflow — so it is
//! proven smooth on the same TVs while still showing the overlay info.

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
/// While the `ndi-fullscreen` layout is active, EVERY stage display is
/// 303-redirected to the lite plain-JS player at `/stage/lite` instead of the
/// WASM app, so the weak TVs get the smooth minimal-RAM page whenever the NDI
/// layout is live. The retirement reason (the lite page dropped the stage
/// overlay blocks) is gone: the lite page now renders the clock, song number,
/// connection status and version as lightweight fixed-size DOM. Any other
/// layout (worship, timer, …) serves the normal WASM shell unchanged.
#[instrument(skip_all)]
pub(super) async fn stage_shell(State(state): State<AppState>) -> Response {
    if state.stage_layout_code().await == NDI_FULLSCREEN_LAYOUT_CODE {
        return Redirect::to("/stage/lite").into_response();
    }
    super::wasm_ui::wasm_ui_shell().await
}
