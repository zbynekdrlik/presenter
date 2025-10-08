use axum::extract::State;
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
pub(super) async fn operator_ui(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_operator_ui(&state).await?;
    Ok(html)
}

#[instrument(skip_all)]
pub(super) async fn settings_ui(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_settings_ui(&state).await?;
    Ok(html)
}

pub(super) async fn tablet_ui(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, AppError> {
    let html = ui::render_tablet_ui(&state).await?;
    Ok(html)
}
