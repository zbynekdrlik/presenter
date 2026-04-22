use axum::{extract::State, http::StatusCode, Json};
use tracing::instrument;

use super::AppError;
use crate::state::{ApiStageState, AppState};

#[instrument(skip_all)]
pub(super) async fn update_api_stage(
    State(state): State<AppState>,
    Json(payload): Json<ApiStageState>,
) -> Result<StatusCode, AppError> {
    state
        .update_api_stage(payload)
        .await
        .map_err(AppError::bad_request)?;
    Ok(StatusCode::NO_CONTENT)
}
