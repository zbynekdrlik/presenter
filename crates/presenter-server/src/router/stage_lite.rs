//! Lite NDI stage page — plain-JS WHEP player, manual diagnostic only.
//!
//! Born as the #379 experiment (do the 1GB Vestel TVs stall because of the
//! WASM app?). The 2026-06-12 A/B answered NO — the lite page stalled the
//! same — so the page is kept ONLY as a manual diagnostic player at
//! `/stage/lite`. It mirrors the WASM client's WHEP connect + watchdog
//! semantics in a single self-contained HTML page (no WASM, no frameworks).

use axum::{
    http::header,
    response::{Html, IntoResponse, Response},
};
use tracing::instrument;

/// The self-contained lite player page, embedded at compile time (same
/// convention as `settings_script.js` / the tablet service worker).
const STAGE_LITE_HTML: &str = include_str!("../assets/stage_lite.html");

/// `GET /stage/lite` — serve the embedded plain-JS WHEP player.
///
/// `Cache-Control: no-store` for the same reason as the WASM shell: an
/// always-on stage TV must pick up a new page on every deploy without a
/// manual hard-refresh.
#[instrument(skip_all)]
pub(super) async fn stage_lite_page() -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Html(STAGE_LITE_HTML)).into_response()
}

/// `GET /stage` — the stage display entry point: ALWAYS the full WASM page.
///
/// The #379 lite-player auto-redirect for the `ndi-fullscreen` layout is
/// RETIRED: the 2026-06-12 A/B proved the WASM page was NOT the weak-TV
/// bottleneck (identical ledger results on both pages), and the redirect
/// silently removed the stage overlay blocks (clock, song number, status) —
/// a UX regression the user never approved. Every layout serves the WASM
/// shell; `/stage/lite` stays available as a manual diagnostic player only.
#[instrument(skip_all)]
pub(super) async fn stage_shell() -> Response {
    super::wasm_ui::wasm_ui_shell().await
}
