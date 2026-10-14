//! Client for the šepkár tips API (#852 spec §9.4). Writes use
//! `post_json_refusal`: a 422 / 429 / 404 / 409 whose body is the server's
//! JSON error body is an `ApiError::Refused` carrying its Slovak `message`;
//! any other non-2xx (a proxy page) stays an `ApiError::Status`.
use super::{get_json, post_json_refusal, ApiError};
use presenter_core::interpreter_tips::{
    CreateTipResponse, DismissAllResponse, NewTipRequest, TipDto, TipsList,
};

/// `scope=active` (the interpreter) or `scope=today` (the šepkár page).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipsScope {
    Active,
    Today,
}

pub async fn list(scope: TipsScope) -> Result<TipsList, ApiError> {
    let scope = match scope {
        TipsScope::Active => "active",
        TipsScope::Today => "today",
    };
    get_json(&format!("/interpreter/tips?scope={scope}")).await
}

pub async fn create(request: &NewTipRequest) -> Result<CreateTipResponse, ApiError> {
    post_json_refusal("/interpreter/tips", request).await
}

pub async fn dismiss(id: i64) -> Result<TipDto, ApiError> {
    post_json_refusal(&format!("/interpreter/tips/{id}/dismiss"), &()).await
}

pub async fn restore(id: i64) -> Result<TipDto, ApiError> {
    post_json_refusal(&format!("/interpreter/tips/{id}/restore"), &()).await
}

pub async fn dismiss_all() -> Result<DismissAllResponse, ApiError> {
    post_json_refusal("/interpreter/tips/dismiss-all", &()).await
}
