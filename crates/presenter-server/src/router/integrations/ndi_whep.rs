//! WHEP HTTP shim — bridges browser WHEP HTTP operations
//! (POST/PATCH/DELETE) into the active source's `NdiPipeline` methods
//! (add_consumer / add_ice_candidate / remove_consumer) via
//! `NdiManager::whep_signaller_call`.

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
};
use presenter_ndi::manager::{WhepOp, WhepReply, SOURCE_NOT_ACTIVE_ERR};
use presenter_ndi::StreamProfile;
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;

/// Query parameters on the WHEP POST. `?profile=` is parsed but always resolves
/// to the single shared 720p H264 stream (see `StreamProfile::from_query`); the
/// value is accepted for backward-compat and never changes the codec.
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct WhepPostQuery {
    profile: Option<String>,
}

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
/// doesn't exist). "consumer cap reached" → 503 + Retry-After: 60 (browser
/// should back off, not hammer). Anything else (pipeline starting / stopped /
/// errored, signaller emit failures) → 503 so WHEP clients back off and retry.
fn map_signaller_error(err: anyhow::Error) -> AppError {
    let msg = err.to_string();
    if msg.contains(SOURCE_NOT_ACTIVE_ERR) {
        AppError::not_found("NDI source not active")
    } else if msg.contains("consumer cap reached") {
        AppError::service_unavailable_with_retry(format!("WHEP: {msg}"), 60)
    } else {
        AppError::service_unavailable(format!("WHEP: {msg}"))
    }
}

#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn post_whep_endpoint(
    Path(source_id): Path<String>,
    Query(query): Query<WhepPostQuery>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let reply = match manager
        .whep_signaller_call(
            &source_id,
            WhepOp::Post {
                id: None,
                body: body.to_vec(),
                profile: StreamProfile::from_query(query.profile.as_deref()),
            },
        )
        .await
    {
        Ok(reply) => reply,
        // #431: a configured-but-not-currently-producing source is a transient,
        // expected state — answer 204 No Content, NOT 404. The stage only POSTs
        // WHEP for a source the operator has activated (it never renders an
        // <NdiVideo> for an unknown source), so "source not active" here means
        // "configured, pipeline not producing yet" — the same reconnect-and-
        // wait semantics the client already handles for the placeholder. A 404
        // made the browser network layer log "Failed to load resource: 404" as
        // a console ERROR on every reconnect poll of the prod stage, violating
        // browser-console-zero-errors. Mirrors the idempotent-DELETE fix below.
        Err(err) if err.to_string().contains(SOURCE_NOT_ACTIVE_ERR) => WhepReply {
            status: 204,
            headers: Vec::new(),
            body: None,
        },
        Err(err) => return Err(map_signaller_error(err)),
    };
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
                // Session-scoped re-offer is unsupported (501) — the profile
                // is irrelevant on this path.
                profile: StreamProfile::Default,
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
    let reply = match manager
        .whep_signaller_call(&source_id, WhepOp::Delete { id: session_id })
        .await
    {
        Ok(reply) => reply,
        // Idempotent DELETE: a session (or its whole source) that is already
        // gone means the client's desired state holds — answer 204, not 404.
        // The stage UI dispatches teardown DELETEs from both on_cleanup and
        // pagehide, and after a server-side deactivate the session is gone
        // before the DELETE arrives; a 404 logged a browser console error
        // ("Failed to load resource") on every deactivate/navigation cycle.
        Err(err)
            if err.to_string().contains(SOURCE_NOT_ACTIVE_ERR)
                || err.to_string().contains("session not found") =>
        {
            WhepReply {
                status: 204,
                headers: Vec::new(),
                body: None,
            }
        }
        Err(err) => return Err(map_signaller_error(err)),
    };
    Ok(into_response(reply))
}

/// Test-only: simulate an `ndisrc` "Internal data stream error" by
/// forcing the source's pipeline into `Errored` state. The supervisor
/// task (still alive, still watching the state channel) reacts as it
/// would for a real fault: rebuilds the pipeline via
/// `NdiManager::rebuild_pipeline`. The browser-side `Watchdog` sees the
/// resulting WebRTC stall, dispatches a fresh WHEP POST, and the new
/// pipeline accepts it — end-to-end recovery in 3-5s.
///
/// Exposed ONLY when compiled with the `test-helpers` cargo feature;
/// production binaries (built without the feature) do not contain this
/// route. The Playwright recovery test calls it to make the recovery
/// assertion deterministic.
///
/// Note: `simulate_pipeline_error` acquires the active mutex once. There
/// is no two-acquire TOCTOU like the previous `stop_pipeline`
/// implementation, and the source remains active — what we're injecting
/// is a fault that the supervisor recovers from, not a deactivation.
#[cfg(feature = "test-helpers")]
#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn kill_pipeline_for_test(
    Path(source_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    if !manager
        .simulate_pipeline_error(&source_id, "simulated ndisrc crash")
        .await
    {
        return Err(AppError::not_found("NDI source not active"));
    }
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

    /// Pre-#431 this asserted a 404/503 contract for a not-active POST. #431
    /// deliberately changed the not-producing POST to 204 (see
    /// `post_whep_endpoint_returns_204_for_configured_not_producing_source`),
    /// because the stage only POSTs for configured sources and a 404 logged a
    /// browser console error. Kept as a regression guard that the POST handler
    /// NEVER returns 404 for the not-active case — only 204 (libndi) or 503
    /// (no libndi).
    #[tokio::test]
    async fn post_whep_endpoint_never_404_for_inactive_source() {
        let state = fresh_state().await;
        let result = post_whep_endpoint(
            Path("00000000-0000-0000-0000-000000000000".to_string()),
            Query(WhepPostQuery::default()),
            State(state),
            empty_body(),
        )
        .await;
        let status = match result {
            // With libndi: manager exists but the source isn't active → 204.
            Ok(resp) => resp.status(),
            // Without libndi: ndi_manager() is None → 503.
            Err(err) => err.into_response().status(),
        };
        assert!(
            matches!(
                status,
                StatusCode::NO_CONTENT | StatusCode::SERVICE_UNAVAILABLE
            ),
            "POST for a not-active source must be 204 or 503, never 404 (#431), got {status}"
        );
    }

    /// `?profile=compat` must not change the contract for a not-active source:
    /// the query is parsed but always resolves to the single shared 720p H264
    /// stream (see `StreamProfile::from_query`), so it still yields the same
    /// 204/503 path as a bare-profile POST (#431).
    #[tokio::test]
    async fn post_whep_endpoint_with_compat_profile_keeps_inactive_source_contract() {
        let state = fresh_state().await;
        let result = post_whep_endpoint(
            Path("00000000-0000-0000-0000-000000000000".to_string()),
            Query(WhepPostQuery {
                profile: Some("compat".to_string()),
            }),
            State(state),
            empty_body(),
        )
        .await;
        let status = match result {
            Ok(resp) => resp.status(),
            Err(err) => err.into_response().status(),
        };
        assert!(
            matches!(
                status,
                StatusCode::NO_CONTENT | StatusCode::SERVICE_UNAVAILABLE
            ),
            "compat-profile POST for a not-active source must be 204 or 503, never 404 (#431), got {status}"
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

    /// DELETE on a WHEP session must be IDEMPOTENT: a session (or its whole
    /// source) that is already gone means the client's desired state holds,
    /// so the reply is 204 — NOT 404. A 404 here made every stage-display
    /// teardown after a server-side deactivate log a browser console error
    /// ("Failed to load resource: 404") from both the on_cleanup and the
    /// pagehide DELETE dispatches.
    ///
    /// With libndi: manager exists, source not active → 204 (idempotent).
    /// Without libndi: ndi_manager() is None → Err 503 (different failure).
    #[tokio::test]
    async fn delete_whep_session_is_idempotent_for_unknown_source() {
        let state = fresh_state().await;
        let result = delete_whep_session(
            Path((
                "00000000-0000-0000-0000-000000000000".to_string(),
                "session-id".to_string(),
            )),
            State(state),
        )
        .await;
        match result {
            Ok(resp) => assert_eq!(
                resp.status(),
                StatusCode::NO_CONTENT,
                "DELETE on an already-gone session must be 204 (idempotent)"
            ),
            Err(err) => assert_eq!(
                err.into_response().status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "only the no-libndi (manager missing) branch may error"
            ),
        }
    }

    /// #431: a WHEP POST for a source that is configured (the stage only
    /// renders `<NdiVideo>` for an operator-activated source) but not currently
    /// producing a pipeline must return 204 No Content — NOT 404. This is a
    /// transient, expected state (the NDI sender went quiet / the pipeline
    /// hasn't started yet); 404 made the browser log a "Failed to load
    /// resource: 404" console error on the prod stage on every reconnect poll,
    /// violating browser-console-zero-errors. 404 is reserved for a genuinely
    /// unknown source, which the stage never POSTs for. Mirrors the DELETE
    /// idempotency fix below.
    ///
    /// With libndi: manager exists, source not active → 204.
    /// Without libndi: ndi_manager() is None → Err 503 (different failure).
    #[tokio::test]
    async fn post_whep_endpoint_returns_204_for_configured_not_producing_source() {
        let state = fresh_state().await;
        let result = post_whep_endpoint(
            Path("00000000-0000-0000-0000-000000000000".to_string()),
            Query(WhepPostQuery::default()),
            State(state),
            empty_body(),
        )
        .await;
        match result {
            Ok(resp) => assert_eq!(
                resp.status(),
                StatusCode::NO_CONTENT,
                "POST on a configured-but-not-producing source must be 204, not 404 (#431)"
            ),
            Err(err) => assert_eq!(
                err.into_response().status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "only the no-libndi (manager missing) branch may error"
            ),
        }
    }

    /// `?profile=compat` must follow the same 204 contract for a
    /// configured-but-not-producing source (the profile query is a no-op
    /// server-side; it must not change the not-producing status code).
    #[tokio::test]
    async fn post_whep_endpoint_with_compat_profile_returns_204_for_not_producing_source() {
        let state = fresh_state().await;
        let result = post_whep_endpoint(
            Path("00000000-0000-0000-0000-000000000000".to_string()),
            Query(WhepPostQuery {
                profile: Some("compat".to_string()),
            }),
            State(state),
            empty_body(),
        )
        .await;
        match result {
            Ok(resp) => assert_eq!(
                resp.status(),
                StatusCode::NO_CONTENT,
                "compat-profile POST on a not-producing source must also be 204 (#431)"
            ),
            Err(err) => assert_eq!(
                err.into_response().status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "only the no-libndi (manager missing) branch may error"
            ),
        }
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
    fn map_signaller_error_consumer_cap_emits_503_with_retry_after() {
        let err = anyhow::anyhow!("WHEP consumer cap reached (8 per source) — try again later");
        let app_err = map_signaller_error(err);
        let resp = app_err.into_response();
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "consumer cap must map to 503"
        );
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok());
        assert_eq!(
            retry_after,
            Some("60"),
            "Retry-After header must be 60 seconds"
        );
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
