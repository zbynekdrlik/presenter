//! Stage display entry point.
//!
//! `/stage` always serves the standard WASM stage, which carries the approved
//! overlay blocks (worship, timer, `ndi-fullscreen`, …). An experimental
//! plain-JS "lite" player and a layout-conditional redirect were tried for the
//! weak stage TVs but retired: the ~400ms/20s compositor hitch was the system
//! WebView (Fully Kiosk / libhwui), NOT the WASM stage — the standalone
//! `com.tcl.browser` (its own Viz compositor) renders the standard WASM stage
//! smoothly.

use axum::response::Response;
use tracing::instrument;

/// `GET /stage` — the stage display entry point. Always serves the standard
/// WASM stage app, which renders the active layout (worship, timer,
/// `ndi-fullscreen`, …) with the approved overlay.
#[instrument(skip_all)]
pub(super) async fn stage_shell() -> Response {
    super::wasm_ui::wasm_ui_shell().await
}
