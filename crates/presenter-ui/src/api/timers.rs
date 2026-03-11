use super::{get_json, post_no_content, ApiError};
use presenter_core::TimersOverview;
use serde::Serialize;

/// Fetch timers overview.
pub async fn get_timers() -> Result<TimersOverview, ApiError> {
    get_json("/timers").await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerCommandRequest {
    pub command: String,
}

/// Send a timer command (start, stop, reset, etc.).
pub async fn send_command(timer_type: &str, command: &str) -> Result<(), ApiError> {
    post_no_content(
        &format!("/timers/{timer_type}"),
        &TimerCommandRequest {
            command: command.to_string(),
        },
    )
    .await
}
