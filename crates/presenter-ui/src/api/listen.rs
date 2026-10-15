//! Client for the listening API (#852 C1b): the microphones as a whole list,
//! the status and the recorder's per-day toggle. Writes use
//! `put_json_refusal`: a 422 whose body is the server's JSON error is an
//! `ApiError::Refused` carrying the Slovak rule.

use super::{get_json, put_json_refusal, ApiError};
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::recorder::{RecorderSetting, RecorderToggle};
use presenter_core::listen::status::ListenStatus;

pub async fn get_mics() -> Result<Vec<MicConfig>, ApiError> {
    get_json("/listen/mics").await
}

pub async fn put_mics(mics: &[MicConfig]) -> Result<Vec<MicConfig>, ApiError> {
    put_json_refusal("/listen/mics", &mics).await
}

pub async fn status() -> Result<ListenStatus, ApiError> {
    get_json("/listen/status").await
}

pub async fn set_recorder(enabled: bool) -> Result<RecorderSetting, ApiError> {
    put_json_refusal("/listen/recorder", &RecorderToggle { enabled }).await
}
