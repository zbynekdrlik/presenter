use super::{get_json, post_json, ApiError};
use presenter_core::{TimerCommand, TimersOverview};

pub async fn get_timers() -> Result<TimersOverview, ApiError> {
    get_json("/timers/overview").await
}

pub async fn send_command(command: &TimerCommand) -> Result<TimersOverview, ApiError> {
    post_json("/timers/command", command).await
}
