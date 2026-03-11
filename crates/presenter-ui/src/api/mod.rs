pub mod bible;
pub mod libraries;
pub mod playlists;
pub mod presentations;
pub mod settings;
pub mod stage;
pub mod timers;

use gloo_net::http::Request;
use serde::de::DeserializeOwned;

/// Base URL for API requests (same origin).
fn api_url(path: &str) -> String {
    path.to_string()
}

/// Perform a GET request and deserialize the JSON response.
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

/// Perform a POST request with a JSON body and deserialize the response.
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

/// Perform a PUT request with a JSON body and deserialize the response.
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

/// Perform a DELETE request.
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

/// Perform a POST request with a JSON body, expecting no response body (204).
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

/// API error types.
#[derive(Debug)]
pub enum ApiError {
    Network(gloo_net::Error),
    Status(u16, String),
    Serialize(gloo_net::Error),
    Deserialize(gloo_net::Error),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(err) => write!(f, "Network error: {err}"),
            Self::Status(code, text) => write!(f, "HTTP {code}: {text}"),
            Self::Serialize(err) => write!(f, "Serialization error: {err}"),
            Self::Deserialize(err) => write!(f, "Deserialization error: {err}"),
        }
    }
}
