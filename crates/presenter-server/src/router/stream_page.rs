//! Stream-graphics OUTPUT page shell (#709, epic #718).
//!
//! `GET /stream/{slug}` serves the standard embedded WASM app shell — the WASM
//! app (`presenter-ui`) routes by pathname to the transparent stream output
//! page, fetches the output definition over `/stream/api/*` (#707) and renders
//! it live from `/live/ws`. This route serves ONLY the page shell; the REST
//! config/activation API (`/stream/api/*`, #707) and the asset pipeline
//! (`/stream/assets/*`, #708) are mounted by their own routers.
//!
//! Route ordering: `/stream/api/*` and `/stream/assets/*` are deeper, static
//! prefixes and take precedence over the `{slug}` param (axum 0.8 / matchit 0.8
//! resolve a static segment before a param sibling); `api` and `assets` are also
//! reserved slugs (`presenter_core::RESERVED_STREAM_SLUGS`), so a real output
//! can never collide with them.

use axum::{extract::Path, response::Response};
use tracing::instrument;

/// `GET /stream/{slug}` — the OBS browser-source output page. Always serves the
/// standard WASM app shell; the slug is resolved client-side by the WASM app.
#[instrument(skip_all)]
pub(super) async fn stream_page(Path(_slug): Path<String>) -> Response {
    super::wasm_ui::wasm_ui_shell().await
}
