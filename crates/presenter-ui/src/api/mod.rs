pub mod ai;
pub mod bible;
pub mod libraries;
pub mod ndi;
pub mod network;
pub mod playlists;
pub mod presentations;
pub mod settings;
pub mod stage;
pub mod stream;
pub mod sync;
pub mod timers;

use gloo_net::http::{Request, Response};
use serde::de::DeserializeOwned;

fn api_url(path: &str) -> String {
    path.to_string()
}

pub async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T, ApiError> {
    let response = Request::get(&api_url(path))
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    response.json().await.map_err(ApiError::Deserialize)
}

pub async fn post_json<B: serde::Serialize, T: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let response = Request::post(&api_url(path))
        .json(body)
        .map_err(ApiError::Serialize)?
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    response.json().await.map_err(ApiError::Deserialize)
}

pub async fn put_json<B: serde::Serialize, T: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let response = Request::put(&api_url(path))
        .json(body)
        .map_err(ApiError::Serialize)?
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    response.json().await.map_err(ApiError::Deserialize)
}

pub async fn patch_json<B: serde::Serialize, T: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let response = Request::patch(&api_url(path))
        .json(body)
        .map_err(ApiError::Serialize)?
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    response.json().await.map_err(ApiError::Deserialize)
}

pub async fn put_no_content<B: serde::Serialize>(path: &str, body: &B) -> Result<(), ApiError> {
    let response = Request::put(&api_url(path))
        .json(body)
        .map_err(ApiError::Serialize)?
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    Ok(())
}

pub async fn patch_no_content<B: serde::Serialize>(path: &str, body: &B) -> Result<(), ApiError> {
    let response = Request::patch(&api_url(path))
        .json(body)
        .map_err(ApiError::Serialize)?
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    Ok(())
}

pub async fn delete(path: &str) -> Result<(), ApiError> {
    let response = Request::delete(&api_url(path))
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    Ok(())
}

pub async fn delete_json<T: DeserializeOwned>(path: &str) -> Result<T, ApiError> {
    let response = Request::delete(&api_url(path))
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    response.json().await.map_err(ApiError::Deserialize)
}

pub async fn post_no_content<B: serde::Serialize>(path: &str, body: &B) -> Result<(), ApiError> {
    let response = Request::post(&api_url(path))
        .json(body)
        .map_err(ApiError::Serialize)?
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    Ok(())
}

pub async fn post_form_data<T: DeserializeOwned>(
    path: &str,
    form_data: &web_sys::FormData,
) -> Result<T, ApiError> {
    let response = Request::post(&api_url(path))
        .body(form_data.clone())
        .map_err(ApiError::Serialize)?
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    response.json().await.map_err(ApiError::Deserialize)
}

/// The server's JSON error body (`router.rs` `ErrorBody { message }`). Read by
/// the `*_detail` helpers below so a 422/409 message reaches the UI instead of
/// the bare HTTP status text.
#[derive(serde::Deserialize)]
struct ErrorDetail {
    #[serde(default)]
    message: String,
}

/// Build an [`ApiError::Status`] carrying the server's error-body `message`
/// (falling back to the raw body, then the status text) for a non-2xx
/// response. Used by the `*_detail` helpers so an inline error can show the
/// real refusal reason (e.g. a props-validation 422 or a referenced-asset 409)
/// rather than "Unprocessable Entity".
async fn status_with_detail(response: Response) -> ApiError {
    let code = response.status();
    let fallback = response.status_text();
    match response.text().await {
        Ok(text) => {
            let detail = serde_json::from_str::<ErrorDetail>(&text)
                .ok()
                .map(|d| d.message)
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| {
                    if text.trim().is_empty() {
                        fallback
                    } else {
                        text
                    }
                });
            ApiError::Status(code, detail)
        }
        Err(_) => ApiError::Status(code, fallback),
    }
}

/// `post_json`, but a non-2xx carries the server error-body message (see
/// [`status_with_detail`]).
pub async fn post_json_detail<B: serde::Serialize, T: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let response = Request::post(&api_url(path))
        .json(body)
        .map_err(ApiError::Serialize)?
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(status_with_detail(response).await);
    }
    response.json().await.map_err(ApiError::Deserialize)
}

/// `patch_json`, but a non-2xx carries the server error-body message.
pub async fn patch_json_detail<B: serde::Serialize, T: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, ApiError> {
    let response = Request::patch(&api_url(path))
        .json(body)
        .map_err(ApiError::Serialize)?
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(status_with_detail(response).await);
    }
    response.json().await.map_err(ApiError::Deserialize)
}

/// `delete`, but a non-2xx carries the server error-body message (the guarded
/// asset-delete 409 names the referencing scenes).
pub async fn delete_detail(path: &str) -> Result<(), ApiError> {
    let response = Request::delete(&api_url(path))
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(status_with_detail(response).await);
    }
    Ok(())
}

/// Response from the /healthz endpoint.
#[derive(serde::Deserialize)]
pub struct HealthzResponse {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub channel: String,
}

#[derive(Debug)]
pub enum ApiError {
    Network(gloo_net::Error),
    Status(u16, String),
    Serialize(gloo_net::Error),
    Deserialize(gloo_net::Error),
    NotFound(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(err) => write!(f, "Network error: {err}"),
            Self::Status(code, text) => write!(f, "HTTP {code}: {text}"),
            Self::Serialize(err) => write!(f, "Serialization error: {err}"),
            Self::Deserialize(err) => write!(f, "Deserialization error: {err}"),
            Self::NotFound(msg) => write!(f, "Not found: {msg}"),
        }
    }
}
