pub mod ai;
pub mod bible;
pub mod libraries;
pub mod playlists;
pub mod presentations;
pub mod settings;
pub mod stage;
pub mod timers;

use gloo_net::http::Request;
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
