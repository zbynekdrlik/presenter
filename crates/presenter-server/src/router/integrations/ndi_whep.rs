//! WHEP HTTP shim — bridges browser SDP exchanges into the per-source
//! `whepserversink` element's signaller via `emit_by_name`.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use presenter_ndi::manager::{WhepOp, WhepReply, SOURCE_NOT_ACTIVE_ERR};
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;

fn into_response(reply: WhepReply) -> Response {
    let mut builder = Response::builder().status(reply.status);
    for (name, value) in &reply.headers {
        builder = builder.header(name, value);
    }
    builder
        .body(axum::body::Body::from(reply.body.unwrap_or_default()))
        .expect("valid response")
}

/// Map a `whep_signaller_call` error string to the right HTTP status.
///
/// "source not active" → 404 (the WHEP spec calls for 404 when the resource
/// doesn't exist). Anything else (pipeline starting / stopped / errored,
/// signaller emit failures) → 503 so WHEP clients back off and retry.
fn map_signaller_error(err: anyhow::Error) -> AppError {
    let msg = err.to_string();
    if msg.contains(SOURCE_NOT_ACTIVE_ERR) {
        AppError::not_found("NDI source not active")
    } else {
        AppError::service_unavailable(format!("WHEP: {msg}"))
    }
}

#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn post_whep_endpoint(
    Path(source_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let reply = manager
        .whep_signaller_call(
            &source_id,
            WhepOp::Post {
                id: None,
                body: body.to_vec(),
            },
        )
        .await
        .map_err(map_signaller_error)?;
    Ok(into_response(reply))
}

#[instrument(skip_all, fields(source_id = %source_id, session_id = %session_id))]
pub(crate) async fn post_whep_session(
    Path((source_id, session_id)): Path<(String, String)>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let reply = manager
        .whep_signaller_call(
            &source_id,
            WhepOp::Post {
                id: Some(session_id),
                body: body.to_vec(),
            },
        )
        .await
        .map_err(map_signaller_error)?;
    Ok(into_response(reply))
}

#[instrument(skip_all, fields(source_id = %source_id, session_id = %session_id))]
pub(crate) async fn patch_whep_session(
    Path((source_id, session_id)): Path<(String, String)>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let hs: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|s| (k.as_str().to_string(), s.to_string()))
        })
        .collect();
    let reply = manager
        .whep_signaller_call(
            &source_id,
            WhepOp::Patch {
                id: session_id,
                body: body.to_vec(),
                headers: hs,
            },
        )
        .await
        .map_err(map_signaller_error)?;
    Ok(into_response(reply))
}

#[instrument(skip_all, fields(source_id = %source_id, session_id = %session_id))]
pub(crate) async fn delete_whep_session(
    Path((source_id, session_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let reply = manager
        .whep_signaller_call(&source_id, WhepOp::Delete { id: session_id })
        .await
        .map_err(map_signaller_error)?;
    Ok(into_response(reply))
}

/// Test-only: force the per-source GStreamer pipeline to stop, simulating
/// an `ndisrc` crash. The PipelineSupervisor's recovery path should then
/// rebuild it autonomously. Exposed ONLY when compiled with the
/// `test-helpers` cargo feature; production binaries (built without the
/// feature) do not contain this route. The Playwright recovery test calls
/// it to make the recovery assertion deterministic.
///
/// Note: the `is_active` → `stop_pipeline` sequence is two separate mutex
/// acquisitions, so a concurrent stop could race the `is_active` check.
/// Harmless in the test context (the worst case is returning 204 for an
/// already-stopped pipeline, and `stop_pipeline` is a no-op on missing keys).
#[cfg(feature = "test-helpers")]
#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn kill_pipeline_for_test(
    Path(source_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    if !manager.is_active(&source_id).await {
        return Err(AppError::not_found("NDI source not active"));
    }
    manager.stop_pipeline(&source_id).await;
    Ok(Response::builder()
        .status(204)
        .body(axum::body::Body::empty())
        .expect("valid response"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    /// Build a fresh in-memory AppState that may or may not have a real NDI
    /// manager attached depending on whether libndi is loadable on the host.
    async fn fresh_state() -> AppState {
        AppState::in_memory().await.expect("in-memory AppState")
    }

    fn empty_body() -> Bytes {
        Bytes::new()
    }

    #[tokio::test]
    async fn post_whep_endpoint_returns_not_found_or_unavailable_for_inactive_source() {
        let state = fresh_state().await;
        let result = post_whep_endpoint(
            Path("00000000-0000-0000-0000-000000000000".to_string()),
            State(state),
            empty_body(),
        )
        .await;
        let Err(err) = result else {
            panic!("expected Err for inactive source");
        };
        // With libndi: manager exists but the source isn't active → 404.
        // Without libndi: ndi_manager() is None → 503.
        let resp = err.into_response();
        assert!(
            matches!(
                resp.status(),
                StatusCode::NOT_FOUND | StatusCode::SERVICE_UNAVAILABLE
            ),
            "expected 404 or 503, got {}",
            resp.status()
        );
    }

    /// Both 404 (manager present, source not active) and 503 (no manager) are
    /// expected for an unknown source — same logic as post_whep_endpoint.
    fn assert_not_found_or_unavailable(resp_status: StatusCode) {
        assert!(
            matches!(
                resp_status,
                StatusCode::NOT_FOUND | StatusCode::SERVICE_UNAVAILABLE
            ),
            "expected 404 or 503, got {resp_status}"
        );
    }

    #[tokio::test]
    async fn post_whep_session_returns_not_found_or_unavailable_for_unknown_source() {
        let state = fresh_state().await;
        let result = post_whep_session(
            Path((
                "00000000-0000-0000-0000-000000000000".to_string(),
                "session-id".to_string(),
            )),
            State(state),
            empty_body(),
        )
        .await;
        let Err(err) = result else {
            panic!("expected Err for unknown source");
        };
        assert_not_found_or_unavailable(err.into_response().status());
    }

    #[tokio::test]
    async fn patch_whep_session_returns_not_found_or_unavailable_for_unknown_source() {
        let state = fresh_state().await;
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            "application/trickle-ice-sdpfrag".parse().unwrap(),
        );
        let result = patch_whep_session(
            Path((
                "00000000-0000-0000-0000-000000000000".to_string(),
                "session-id".to_string(),
            )),
            State(state),
            headers,
            empty_body(),
        )
        .await;
        let Err(err) = result else {
            panic!("expected Err for unknown source");
        };
        assert_not_found_or_unavailable(err.into_response().status());
    }

    #[tokio::test]
    async fn delete_whep_session_returns_not_found_or_unavailable_for_unknown_source() {
        let state = fresh_state().await;
        let result = delete_whep_session(
            Path((
                "00000000-0000-0000-0000-000000000000".to_string(),
                "session-id".to_string(),
            )),
            State(state),
        )
        .await;
        let Err(err) = result else {
            panic!("expected Err for unknown source");
        };
        assert_not_found_or_unavailable(err.into_response().status());
    }

    #[test]
    fn into_response_passes_status_headers_and_body() {
        let reply = WhepReply {
            status: 201,
            headers: vec![("location".to_string(), "/whep/abc".to_string())],
            body: Some(b"v=0\r\ns=-\r\n".to_vec()),
        };
        let resp = into_response(reply);
        assert_eq!(resp.status(), StatusCode::CREATED);
        let location = resp.headers().get("location").and_then(|v| v.to_str().ok());
        assert_eq!(location, Some("/whep/abc"));
    }

    #[test]
    fn into_response_defaults_to_empty_body_when_none() {
        let reply = WhepReply {
            status: 204,
            headers: Vec::new(),
            body: None,
        };
        let resp = into_response(reply);
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[cfg(feature = "test-helpers")]
    #[tokio::test]
    async fn kill_pipeline_for_test_returns_404_or_503_for_unknown_source() {
        let state = fresh_state().await;
        let result = kill_pipeline_for_test(
            axum::extract::Path("unknown".to_string()),
            axum::extract::State(state),
        )
        .await;
        assert!(result.is_err(), "expected error for unknown source");
        let err = result.unwrap_err();
        // With libndi: manager exists but the source isn't active → 404.
        // Without libndi: ndi_manager() is None → 503.
        assert_not_found_or_unavailable(err.into_response().status());
    }
}
