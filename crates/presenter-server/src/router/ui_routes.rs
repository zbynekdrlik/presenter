use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use tracing::instrument;

use super::AppError;
use crate::{state::AppState, ui};

#[instrument(skip_all)]
pub(super) async fn home(
    State(_state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_home_ui().await?;
    Ok(html)
}

/// `/overlays/timer` → `302 /stream/timer` (#785). The timer overlay is now a
/// seeded stream-graphics output (`slug=timer`) rendered by the WASM output page,
/// so the retired SSR page is gone; this redirect keeps every existing OBS
/// browser source (`/overlays/timer`) working. A plain `302 Found` with a
/// `Location` header — a GET redirect OBS/CEF and every browser follow.
#[instrument(skip_all)]
pub(super) async fn timer_overlay_redirect() -> impl IntoResponse {
    (StatusCode::FOUND, [(header::LOCATION, "/stream/timer")])
}
