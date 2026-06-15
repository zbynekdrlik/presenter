//! Lite NDI stage page — plain-JS WHEP player (legacy fallback).
//!
//! Historically `/stage` 303-redirected here whenever the `ndi-fullscreen`
//! layout was active, to dodge a ~400ms/20s compositor hitch on the weak stage
//! TVs. That hitch was the **system WebView** (Fully Kiosk / libhwui), NOT the
//! WASM stage itself — the standalone `com.tcl.browser` (its own Viz
//! compositor) renders the standard WASM stage smoothly. So `/stage` now always
//! serves the standard WASM stage, which carries the approved overlay blocks.
//! This lite page stays reachable at `/stage/lite` only as a minimal fallback.

use axum::{
    http::header,
    response::{Html, IntoResponse, Response},
};
use tracing::instrument;

/// The self-contained lite player page, embedded at compile time.
const STAGE_LITE_HTML: &str = include_str!("../assets/stage_lite.html");

/// `GET /stage/lite` — serve the embedded plain-JS WHEP player (fallback only;
/// `/stage` serves the standard WASM stage). `Cache-Control: no-store` so an
/// always-on TV picks up a new page on every deploy without a hard-refresh.
#[instrument(skip_all)]
pub(super) async fn stage_lite_page() -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Html(STAGE_LITE_HTML)).into_response()
}

/// `GET /stage` — the stage display entry point. Always serves the standard
/// WASM stage app, which renders the active layout (worship, timer,
/// `ndi-fullscreen`, …) with the approved overlay. The earlier redirect to the
/// lite player is retired: the 20s hitch was the system WebView, fixed by
/// running the standard stage in the standalone com.tcl.browser.
#[instrument(skip_all)]
pub(super) async fn stage_shell() -> Response {
    super::wasm_ui::wasm_ui_shell().await
}
