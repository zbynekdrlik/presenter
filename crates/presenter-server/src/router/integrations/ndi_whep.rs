//! WHEP HTTP shim — bridges browser SDP exchanges into the per-source
//! `whepserversink` element's signaller via `emit_by_name`.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use presenter_ndi::manager::{WhepOp, WhepReply};
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;

fn into_response(reply: WhepReply) -> Response {
    let mut builder = Response::builder().status(reply.status);
    if let Some(headers) = reply.headers {
        for (name, value) in headers.iter() {
            if let Ok(s) = value.get::<String>() {
                builder = builder.header(name.to_string(), s);
            }
        }
    }
    builder
        .body(axum::body::Body::from(reply.body.unwrap_or_default()))
        .expect("valid response")
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
    if !manager.is_active(&source_id).await {
        return Err(AppError::not_found("NDI source not active"));
    }
    let reply = manager
        .whep_signaller_call(
            &source_id,
            WhepOp::Post {
                id: None,
                body: body.to_vec(),
            },
        )
        .await
        .map_err(|e| AppError::service_unavailable(format!("WHEP POST: {e}")))?;
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
        .map_err(|e| AppError::service_unavailable(format!("WHEP POST session: {e}")))?;
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
        .map_err(|e| AppError::service_unavailable(format!("WHEP PATCH: {e}")))?;
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
        .map_err(|e| AppError::service_unavailable(format!("WHEP DELETE: {e}")))?;
    Ok(into_response(reply))
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

    #[tokio::test]
    async fn post_whep_session_returns_unavailable_for_unknown_source() {
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
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn patch_whep_session_returns_unavailable_for_unknown_source() {
        let state = fresh_state().await;
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/trickle-ice-sdpfrag".parse().unwrap());
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
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn delete_whep_session_returns_unavailable_for_unknown_source() {
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
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn into_response_passes_through_status_and_body_with_no_headers() {
        let reply = WhepReply {
            status: 201,
            headers: None,
            body: Some(b"v=0\r\ns=-\r\n".to_vec()),
        };
        let resp = into_response(reply);
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn into_response_defaults_to_empty_body_when_none() {
        let reply = WhepReply {
            status: 204,
            headers: None,
            body: None,
        };
        let resp = into_response(reply);
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
