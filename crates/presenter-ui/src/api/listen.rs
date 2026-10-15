//! Client for the listening API (#852 C1b): the microphones as a whole list.
//! The write uses `put_json_refusal`: a 422 whose body is the server's JSON
//! error is an `ApiError::Refused` carrying the Slovak rule.

use super::{get_json, put_json_refusal, ApiError};
use presenter_core::listen::mics::MicConfig;

pub async fn get_mics() -> Result<Vec<MicConfig>, ApiError> {
    get_json("/listen/mics").await
}

pub async fn put_mics(mics: &[MicConfig]) -> Result<Vec<MicConfig>, ApiError> {
    put_json_refusal("/listen/mics", &mics).await
}
