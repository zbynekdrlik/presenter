use axum::{extract::State, response::Redirect};
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

#[instrument(skip_all)]
pub(super) async fn timer_overlay(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_timer_overlay(&state).await?;
    Ok(html)
}

#[instrument(skip_all)]
pub(super) async fn settings_ui(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_settings_ui(&state).await?;
    Ok(html)
}

#[instrument(skip_all)]
pub(super) async fn stage_settings_ui() -> Redirect {
    Redirect::permanent("/ui/stage-design")
}

#[instrument(skip_all)]
pub(super) async fn stage_design_ui(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_stage_design_ui(&state).await?;
    Ok(html)
}
