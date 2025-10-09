use axum::{extract::State, Json};
use serde_json::Value;
use tracing::instrument;

use super::AppError;
use crate::state::AppState;
use anyhow::Error as AnyhowError;
use presenter_core::TimersOverview;

#[instrument(skip_all)]
pub(super) async fn get_timers_overview(
    State(state): State<AppState>,
) -> Result<Json<TimersOverview>, AppError> {
    let overview = state.timers_overview().await?;
    Ok(Json(overview))
}

#[instrument(skip_all)]
pub(super) async fn execute_timer_command(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<Json<TimersOverview>, AppError> {
    let command: presenter_core::TimerCommand =
        serde_json::from_value(payload).map_err(AnyhowError::new)?;
    match state.execute_timer_command(command).await {
        Ok(overview) => Ok(Json(overview)),
        Err(err) => {
            if let Some(timer_err) = err.downcast_ref::<presenter_core::timer::TimerError>() {
                return Err(AppError::bad_request_message(timer_err.to_string()));
            }
            Err(err.into())
        }
    }
}
