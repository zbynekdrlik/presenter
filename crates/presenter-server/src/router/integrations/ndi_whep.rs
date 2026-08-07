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
use presenter_ndi::manager::{NdiSessionError, WhepOp, WhepReply};
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

/// Map a `whep_signaller_call` error to the right HTTP status via the TYPED
/// `NdiSessionError` — never a string match on `Display` text (#589: that
/// silently broke the moment `.context(...)` was added anywhere upstream in
/// `presenter-ndi`, mirroring the persistence-layer `RepositoryError` fix in
/// #584/#586/#587 across a different crate boundary).
///
/// `SourceNotActive` → 404 (the WHEP spec calls for 404 when the resource
/// doesn't exist). `ConsumerCapReached` → 503 + Retry-After: 60 (browser
/// should back off, not hammer). Anything else (pipeline starting / stopped /
/// errored, signaller emit failures, `SessionNotFound`) → 503 so WHEP clients
/// back off and retry.
fn map_signaller_error(err: anyhow::Error) -> AppError {
    match err.downcast_ref::<NdiSessionError>() {
        Some(NdiSessionError::SourceNotActive) => AppError::not_found("NDI source not active"),
        Some(NdiSessionError::ConsumerCapReached { .. }) => {
            AppError::service_unavailable_with_retry(format!("WHEP: {err}"), 60)
        }
        _ => AppError::service_unavailable(format!("WHEP: {err}")),
    }
}

/// #431: map a `whep_signaller_call` POST error to its WHEP response.
///
/// A configured-but-not-currently-producing source (`NdiSessionError::SourceNotActive`)
/// is a transient, EXPECTED state — answer 204 No Content, NOT 404. The stage
/// only POSTs WHEP for a source the operator has activated (it never renders an
/// `<NdiVideo>` for an unknown source), so "source not active" here means
/// "configured, pipeline not producing yet" — the same reconnect-and-wait
/// semantics the client already handles for the placeholder. A 404 made the
/// browser network layer log "Failed to load resource: 404" as a console ERROR
/// on every reconnect poll of the prod stage, violating
/// browser-console-zero-errors. Any OTHER error maps via `map_signaller_error`
/// (404 for unknown on the session-scoped paths, 503 otherwise).
///
/// Pure + directly unit-tested (no live NdiManager needed) so the
/// `SourceNotActive` → 204 vs the fall-through → error distinction is
/// exercised even on a host without libndi — which is required to KILL the
/// mutation-testing mutants on this match guard.
fn map_post_whep_error(err: anyhow::Error) -> Result<WhepReply, AppError> {
    match err.downcast_ref::<NdiSessionError>() {
        Some(NdiSessionError::SourceNotActive) => Ok(WhepReply {
            status: 204,
            headers: Vec::new(),
            body: None,
        }),
        _ => Err(map_signaller_error(err)),
    }
}

/// Idempotent DELETE: a session (or its whole source) that is already gone
/// means the client's desired state holds — answer 204, not 404. The stage UI
/// dispatches teardown DELETEs from both on_cleanup and pagehide, and after a
/// server-side deactivate the session is gone before the DELETE arrives; a 404
/// logged a browser console error ("Failed to load resource") on every
/// deactivate/navigation cycle.
///
/// Pure + directly unit-tested (no live NdiManager needed) so the idempotency
/// guard is exercised on every host — killing the cargo-mutants mutants that
/// replace the match arms with `true`/`false` (which the handler-level test
/// can't catch on a libndi-less CI runner, where the manager-missing 503
/// short-circuits before the guard is reached). Same extraction shape as
/// `map_post_whep_error` above (#431) and #616 Gap B.
fn map_delete_whep_error(err: anyhow::Error) -> Result<WhepReply, AppError> {
    match err.downcast_ref::<NdiSessionError>() {
        Some(NdiSessionError::SourceNotActive) | Some(NdiSessionError::SessionNotFound { .. }) => {
            Ok(WhepReply {
                status: 204,
                headers: Vec::new(),
                body: None,
            })
        }
        _ => Err(map_signaller_error(err)),
    }
}

#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn post_whep_endpoint(
    Path(source_id): Path<String>,
    Query(query): Query<WhepPostQuery>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Response, AppError> {
    // #502: mint (or read cached) the server-side TURN relay URI so the
    // consumer webrtcbin gathers a relay candidate. `None` when TURN is
    // unconfigured → today's LAN-only behavior.
    let turn_server = state.turn().turn_uri().await;
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
                turn_server,
            },
        )
        .await
    {
        Ok(reply) => reply,
        Err(err) => map_post_whep_error(err)?,
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
                // and TURN URI are irrelevant on this path.
                profile: StreamProfile::Default,
                turn_server: None,
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
        // Extracted into `map_delete_whep_error` (#616 Gap B) so the guard is
        // directly unit-testable on CI runners without libndi (the handler
        // short-circuits to 503 before reaching the guard when the manager is
        // missing, mirroring #431's extraction of `map_post_whep_error`).
        Err(err) => map_delete_whep_error(err)?,
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
    use crate::state::ndi_control::{FakeNdiControl, NdiManagerHandle, WhepOutcome};
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

    /// #431 + mutation-kill: `map_post_whep_error` must return a 204 reply for a
    /// `NdiSessionError::SourceNotActive` (configured-but-not-producing) and an
    /// Err for anything else. Tested directly (no NdiManager) so the
    /// downcast-match guard is exercised on every host — killing the
    /// cargo-mutants mutants that replace it with `true`/`false` (which the
    /// handler-level tests couldn't catch on a libndi-less CI runner, where
    /// the manager-missing 503 short-circuits before the guard).
    #[test]
    fn map_post_whep_error_returns_204_for_source_not_active() {
        // Guard MUST be true for a not-active error → 204 reply, NOT an Err.
        // Kills the "guard with false" mutant.
        match map_post_whep_error(NdiSessionError::SourceNotActive.into()) {
            Ok(reply) => {
                assert_eq!(
                    reply.status, 204,
                    "configured-but-not-producing POST must be 204 (#431)"
                );
                assert!(reply.body.is_none(), "204 reply carries no body");
            }
            Err(err) => panic!(
                "not-active POST error must map to a 204 reply, got HTTP {}",
                err.into_response().status()
            ),
        }
    }

    #[test]
    fn map_post_whep_error_passes_through_non_not_active_errors() {
        // Guard MUST be false for a non-not-active error → it maps to an
        // AppError (here: 503), NOT a 204. Kills the "guard with true" mutant.
        match map_post_whep_error(anyhow::anyhow!("pipeline errored: ndisrc crash")) {
            Ok(reply) => panic!(
                "a non-not-active error must NOT become a 204 reply, got status {}",
                reply.status
            ),
            Err(err) => assert_eq!(
                err.into_response().status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "a generic pipeline error maps to 503, not 204"
            ),
        }

        // A consumer-cap error also passes through (503 + Retry-After), never 204.
        match map_post_whep_error(NdiSessionError::ConsumerCapReached { max: 8 }.into()) {
            Ok(reply) => panic!(
                "consumer-cap must not become a 204 reply, got status {}",
                reply.status
            ),
            Err(err) => assert_eq!(
                err.into_response().status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "consumer cap maps to 503, not 204"
            ),
        }
    }

    #[test]
    fn map_signaller_error_consumer_cap_emits_503_with_retry_after() {
        let err: anyhow::Error = NdiSessionError::ConsumerCapReached { max: 8 }.into();
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

    /// #616 Gap B: `map_delete_whep_error` must return 204 for a
    /// `SourceNotActive` — the DELETE is idempotent, a source that's already
    /// gone means the client's desired state holds. The handler-level test
    /// can't exercise this guard on a CI runner without libndi (the manager
    /// is `None` → 503 short-circuit), so the extracted pure function is
    /// tested directly. Kills the "guard with false" mutant.
    #[test]
    fn map_delete_whep_error_returns_204_for_source_not_active() {
        match map_delete_whep_error(NdiSessionError::SourceNotActive.into()) {
            Ok(reply) => {
                assert_eq!(
                    reply.status, 204,
                    "DELETE of an inactive source must be 204 (idempotent)"
                );
                assert!(reply.body.is_none(), "204 reply carries no body");
            }
            Err(err) => panic!(
                "not-active DELETE error must map to a 204 reply, got HTTP {}",
                err.into_response().status()
            ),
        }
    }

    /// #616 Gap B: `SessionNotFound` is the other half of the idempotency
    /// guard — a session that's already gone means the client's desired state
    /// holds. Deliberately preserved even though production code rarely
    /// reaches it (see the issue note about not "cleaning up" this branch).
    #[test]
    fn map_delete_whep_error_returns_204_for_session_not_found() {
        match map_delete_whep_error(
            NdiSessionError::SessionNotFound {
                session_id: "test-session".to_string(),
            }
            .into(),
        ) {
            Ok(reply) => {
                assert_eq!(
                    reply.status, 204,
                    "DELETE of a non-existent session must be 204 (idempotent)"
                );
            }
            Err(err) => panic!(
                "session-not-found DELETE error must map to a 204 reply, got HTTP {}",
                err.into_response().status()
            ),
        }
    }

    /// #616 Gap B: a non-idempotent error must NOT become 204 — it maps to
    /// `map_signaller_error` (503 for pipeline errors, 404 for unknown
    /// sources via the generic path). Kills the "guard with true" mutant.
    #[test]
    fn map_delete_whep_error_passes_through_non_idempotent_errors() {
        match map_delete_whep_error(anyhow::anyhow!("pipeline errored: ndisrc crash")) {
            Ok(reply) => panic!(
                "a non-idempotent error must NOT become a 204 reply, got status {}",
                reply.status
            ),
            Err(err) => assert_eq!(
                err.into_response().status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "a generic pipeline error maps to 503, not 204"
            ),
        }
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

    /// #589: `map_signaller_error`'s SOURCE_NOT_ACTIVE_ERR → 404 branch had NO
    /// unit test at all — only the consumer-cap branch was covered. Worse,
    /// the branch decides status by `err.to_string().contains(...)`, which
    /// breaks the moment upstream code wraps the error with `.context(...)`
    /// (a `.context()` call replaces the anyhow `Display` text with the
    /// context message — the original "source not active" text is no longer
    /// visible to a string match, only to a downcast that walks the chain).
    /// This test simulates exactly that: a genuinely "source not active"
    /// error that picked up an unrelated context wrapper somewhere upstream.
    /// It must still map to 404.
    #[test]
    fn map_signaller_error_source_not_active_survives_context_wrapping() {
        let err = anyhow::Error::from(NdiSessionError::SourceNotActive)
            .context("signaller call failed while forwarding to the pipeline");
        let app_err = map_signaller_error(err);
        assert_eq!(
            app_err.into_response().status(),
            StatusCode::NOT_FOUND,
            "a source-not-active error must map to 404 even wrapped in extra context (#589)"
        );
    }

    /// #630 (Gap A from #616): the two existing `map_signaller_error` tests
    /// above call the pure function DIRECTLY — they never prove it is still
    /// wired via `.map_err(map_signaller_error)?` at `post_whep_session`'s
    /// real call site. On a libndi-free CI runner `ndi_manager()` is always
    /// `None`, so that line is never reached by any existing handler-level
    /// test (the "manager missing" 503 short-circuits first) — if the wiring
    /// were dropped (e.g. swapped for a bare `?` that loses the typed
    /// downcast), every existing test would stay green. `set_ndi_handle`
    /// injects a `Fake` manager so the call site is reached deterministically,
    /// regardless of libndi.
    #[tokio::test]
    async fn post_whep_session_wires_map_signaller_error_at_its_call_site() {
        let mut state = fresh_state().await;
        state.set_ndi_handle(NdiManagerHandle::Fake(FakeNdiControl::with_whep_outcome(
            WhepOutcome::ConsumerCapReached { max: 8 },
        )));
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
            panic!("expected an Err mapped via map_signaller_error");
        };
        let resp = err.into_response();
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "ConsumerCapReached must map to 503 via map_signaller_error"
        );
        assert_eq!(
            resp.headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok()),
            Some("60"),
            "map_signaller_error must set Retry-After: 60 for the consumer cap"
        );
    }

    /// Same call-site-wiring gap as above, for `patch_whep_session`'s
    /// `.map_err(map_signaller_error)?`. Uses `SourceNotActive` to also pin
    /// that the session-scoped PATCH path has NO special "not active" → 204
    /// branch (unlike `post_whep_endpoint`/`delete_whep_session`) — it always
    /// falls through to `map_signaller_error`'s 404, per the doc comment on
    /// `map_signaller_error` ("404 for unknown on the session-scoped paths").
    #[tokio::test]
    async fn patch_whep_session_wires_map_signaller_error_at_its_call_site() {
        let mut state = fresh_state().await;
        state.set_ndi_handle(NdiManagerHandle::Fake(FakeNdiControl::with_whep_outcome(
            WhepOutcome::SourceNotActive,
        )));
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
            panic!("expected an Err mapped via map_signaller_error");
        };
        assert_eq!(
            err.into_response().status(),
            StatusCode::NOT_FOUND,
            "SourceNotActive must map to 404 via map_signaller_error on the session-scoped PATCH path"
        );
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
