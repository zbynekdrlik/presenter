//! Client for the listening API (#852 C1b): the microphones as a whole list,
//! the status and the recorder's per-day toggle; C3-9: the guest transcription
//! setting and today's session choice. Writes use
//! `put_json_refusal`: a 422 whose body is the server's JSON error is an
//! `ApiError::Refused` carrying the Slovak rule.

use super::{get_json, put_json_refusal, ApiError};
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::recorder::{RecorderSetting, RecorderToggle};
use presenter_core::listen::status::ListenStatus;
use presenter_core::listen::transcription::{
    SessionMode, TranscriptionSetting, TranscriptionStatus,
};
use serde::Serialize;

/// `PUT /listen/session` body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionChoice {
    mode: SessionMode,
}

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

/// The paid switch, guest mic and daily cap (422 = the Slovak rule).
pub async fn put_transcription(
    setting: &TranscriptionSetting,
) -> Result<TranscriptionStatus, ApiError> {
    put_json_refusal("/listen/transcription", setting).await
}

/// Today's choice: start now, stop, or follow the Kázeň layout.
pub async fn put_session(mode: SessionMode) -> Result<TranscriptionStatus, ApiError> {
    put_json_refusal("/listen/session", &SessionChoice { mode }).await
}
