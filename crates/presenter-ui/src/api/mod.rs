pub mod ai;
pub mod bible;
pub mod interpreter_tips;
pub mod libraries;
pub mod listen;
pub mod ndi;
pub mod network;
pub mod playlists;
pub mod presentations;
pub mod sermons;
pub mod settings;
pub mod stage;
pub mod stream;
pub mod study;
pub mod sync;
pub mod timers;

use gloo_net::http::{Request, Response};
use serde::de::DeserializeOwned;
use wasm_bindgen::{JsCast, JsValue};

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

/// Start a JSON `PATCH` NOW, as a `keepalive` fetch (#871 review: the
/// nameplate saves).
///
/// The helpers above start their fetch on the first poll of the returned
/// future, and Chrome cancels a plain fetch when the page unloads, so a save
/// made as the page is reloaded or closed never reached the server. This one
/// is handed to the browser before the function returns, with `keepalive`
/// set the way `stage::ndi_video::dispatch_delete` sends its DELETE, so it
/// finishes after a reload or a closed tab too. Await its result with
/// [`finish_json`]. (Safari before 13 ignores `keepalive`.)
///
/// A fetch with no answer after `timeout_ms` is aborted, so its result is a
/// [`ApiError::Network`] instead of never coming. An `AbortController` and
/// a timer, not `AbortSignal.timeout()`: Safari before 16 (iPad Air 2,
/// iPad mini 4) lacks that, and the call would throw into the WASM.
pub fn start_patch_keepalive<B: serde::Serialize>(
    path: &str,
    body: &B,
    timeout_ms: u32,
) -> Result<js_sys::Promise, ApiError> {
    let json = serde_json::to_string(body).map_err(|e| ApiError::Serialize(e.into()))?;
    let headers = web_sys::Headers::new().map_err(js_error)?;
    headers
        .set("Content-Type", "application/json")
        .map_err(js_error)?;
    let abort = web_sys::AbortController::new().map_err(js_error)?;
    let init = web_sys::RequestInit::new();
    init.set_method("PATCH");
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(&json));
    init.set_signal(Some(&abort.signal()));
    let _ = js_sys::Reflect::set(&init, &"keepalive".into(), &JsValue::TRUE);
    let request =
        web_sys::Request::new_with_str_and_init(&api_url(path), &init).map_err(js_error)?;
    let window = web_sys::window().ok_or_else(|| js_error(JsValue::from_str("no window")))?;
    let fetch = window.fetch_with_request(&request);
    // Aborting a fetch that already answered does nothing.
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(timeout_ms).await;
        abort.abort();
    });
    Ok(fetch)
}

/// The JSON body of a fetch started by [`start_patch_keepalive`], or its
/// error: the same errors as [`patch_json`] (a refused fetch is
/// [`ApiError::Network`], a non-2xx [`ApiError::Status`]).
pub async fn finish_json<T: DeserializeOwned>(
    started: Result<js_sys::Promise, ApiError>,
) -> Result<T, ApiError> {
    let value = wasm_bindgen_futures::JsFuture::from(started?)
        .await
        .map_err(js_error)?;
    let response = Response::from(value.dyn_into::<web_sys::Response>().map_err(js_error)?);
    if !response.ok() {
        return Err(ApiError::Status(response.status(), response.status_text()));
    }
    response.json().await.map_err(ApiError::Deserialize)
}

/// A JavaScript failure of a fetch we started ourselves (a refused fetch is
/// a `TypeError: Failed to fetch`) as a network error.
fn js_error(value: JsValue) -> ApiError {
    let text = value
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .or_else(|| value.as_string())
        .unwrap_or_else(|| "fetch failed".to_string());
    ApiError::Network(gloo_net::Error::GlooError(text))
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
    let parts = error_parts(response).await;
    ApiError::Status(parts.code, parts.message.unwrap_or(parts.fallback))
}

/// Like [`status_with_detail`], but a body that IS the server's JSON error
/// body becomes [`ApiError::Refused`]: the text is then the server's own
/// (Slovak) message and may be shown as it is. Any other body (a proxy or
/// gateway page, an empty one) stays an [`ApiError::Status`] whose text is
/// not fit for the user.
async fn status_with_refusal(response: Response) -> ApiError {
    let parts = error_parts(response).await;
    match parts.message {
        Some(message) => ApiError::Refused(parts.code, message),
        None => ApiError::Status(parts.code, parts.fallback),
    }
}

/// What a non-2xx response said.
struct ErrorParts {
    code: u16,
    /// The server's own `message`; `None` when the body is not its JSON error
    /// body (or its message is empty).
    message: Option<String>,
    /// The raw body, or the status text when the body is empty or unreadable.
    fallback: String,
}

async fn error_parts(response: Response) -> ErrorParts {
    let code = response.status();
    let status_text = response.status_text();
    match response.text().await {
        Ok(text) => ErrorParts {
            code,
            message: error_message_of(&text),
            fallback: if text.trim().is_empty() {
                status_text
            } else {
                text
            },
        },
        Err(_) => ErrorParts {
            code,
            message: None,
            fallback: status_text,
        },
    }
}

/// The `message` of the server's JSON error body, `None` for anything else
/// (an HTML gateway page, an empty body, JSON without a message).
fn error_message_of(body: &str) -> Option<String> {
    serde_json::from_str::<ErrorDetail>(body)
        .ok()
        .map(|detail| detail.message)
        .filter(|message| !message.is_empty())
}

/// `post_json`, but a non-2xx whose body is the server's JSON error body is an
/// [`ApiError::Refused`] carrying that message (see [`status_with_refusal`]).
pub async fn post_json_refusal<B: serde::Serialize, T: DeserializeOwned>(
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
        return Err(status_with_refusal(response).await);
    }
    response.json().await.map_err(ApiError::Deserialize)
}

/// `put_json`, but a non-2xx whose body is the server's JSON error body is an
/// [`ApiError::Refused`] carrying that message (see [`status_with_refusal`]).
pub async fn put_json_refusal<B: serde::Serialize, T: DeserializeOwned>(
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
        return Err(status_with_refusal(response).await);
    }
    response.json().await.map_err(ApiError::Deserialize)
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

/// PATCH with the [`post_json_refusal`] error shape.
pub async fn patch_json_refusal<B: serde::Serialize, T: DeserializeOwned>(
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
        return Err(status_with_refusal(response).await);
    }
    response.json().await.map_err(ApiError::Deserialize)
}

/// DELETE answering JSON, with the [`post_json_refusal`] error shape.
pub async fn delete_json_refusal<T: DeserializeOwned>(path: &str) -> Result<T, ApiError> {
    let response = Request::delete(&api_url(path))
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(status_with_refusal(response).await);
    }
    response.json().await.map_err(ApiError::Deserialize)
}

/// DELETE with no answer body, with the [`post_json_refusal`] error shape.
pub async fn delete_refusal(path: &str) -> Result<(), ApiError> {
    let response = Request::delete(&api_url(path))
        .send()
        .await
        .map_err(ApiError::Network)?;
    if !response.ok() {
        return Err(status_with_refusal(response).await);
    }
    Ok(())
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
    /// A non-2xx whose body was the server's own JSON error body: the text is
    /// the server's message (only [`post_json_refusal`] / [`put_json_refusal`]
    /// produce it).
    Refused(u16, String),
    Serialize(gloo_net::Error),
    Deserialize(gloo_net::Error),
    NotFound(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(err) => write!(f, "Network error: {err}"),
            Self::Status(code, text) | Self::Refused(code, text) => {
                write!(f, "HTTP {code}: {text}")
            }
            Self::Serialize(err) => write!(f, "Serialization error: {err}"),
            Self::Deserialize(err) => write!(f, "Deserialization error: {err}"),
            Self::NotFound(msg) => write!(f, "Not found: {msg}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_servers_json_error_body_gives_a_message() {
        assert_eq!(
            error_message_of(r#"{"message":"Poznámka môže mať najviac 200 znakov."}"#),
            Some("Poznámka môže mať najviac 200 znakov.".to_string())
        );
        assert_eq!(
            error_message_of("<html><body>429 Too Many Requests</body></html>"),
            None,
            "a proxy page is not the server's message"
        );
        assert_eq!(error_message_of(""), None);
        assert_eq!(error_message_of(r#"{"message":""}"#), None);
        assert_eq!(error_message_of(r#"{"error":"nope"}"#), None);
        assert_eq!(error_message_of(r#""just a string""#), None);
    }
}
