//! HTTP request log (#850 spec §8): a DEBUG span per request (method, path +
//! query, user agent, client IP) and a DEBUG line per response (status,
//! latency in ms). The units' `RUST_LOG=info,tower_http=debug,sqlx=warn`
//! enables both — the span uses tower-http's own `make_span` target so that
//! filter covers it. Client IP follows the live WS rule (#793): X-Forwarded-For
//! counts only from a loopback peer (cloudflared); a LAN peer is the client
//! itself.
//!
//! Polls and assets that every open tab / stage display repeats every few
//! seconds ([`is_quiet_path`]) get NO span and NO response line, so the journal
//! keeps the requests a person made. A 5xx is never hidden: tower-http's
//! `on_failure` still logs it (WARN, without the request fields on a quiet
//! path). Client-controlled strings are recorded as `&str`, so they print
//! quoted and a hostile header cannot forge a field.

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header::USER_AGENT, Request, Response};
use tower_http::classify::{ServerErrorsAsFailures, SharedClassifier};
use tower_http::trace::{
    DefaultOnBodyChunk, DefaultOnFailure, DefaultOnResponse, OnResponse, TraceLayer,
};
use tower_http::LatencyUnit;
use tracing::{Level, Span};

/// The layer `build_router` installs: our span (none for a quiet path), no
/// "started" line, one DEBUG response line, no "end of stream" line, a WARN
/// line for a 5xx.
pub(super) type RequestTraceLayer = TraceLayer<
    SharedClassifier<ServerErrorsAsFailures>,
    fn(&Request<Body>) -> Span,
    (),
    RequestOnResponse,
    DefaultOnBodyChunk,
    (),
    DefaultOnFailure,
>;

/// What the request span carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RequestFields {
    pub(super) method: String,
    pub(super) uri: String,
    pub(super) user_agent: String,
    pub(super) client_ip: String,
}

/// tower-http's DEBUG response line (status, latency in ms), skipped when the
/// request span is disabled — a quiet path (`Span::none()`) has nothing to
/// attach the line to.
#[derive(Debug, Clone, Copy)]
pub(super) struct RequestOnResponse;

impl<B> OnResponse<B> for RequestOnResponse {
    fn on_response(self, response: &Response<B>, latency: Duration, span: &Span) {
        if span.is_disabled() {
            return;
        }
        DefaultOnResponse::new()
            .level(Level::DEBUG)
            .latency_unit(LatencyUnit::Millis)
            .on_response(response, latency, span);
    }
}

pub(super) fn request_trace_layer() -> RequestTraceLayer {
    TraceLayer::new_for_http()
        .make_span_with(request_span as fn(&Request<Body>) -> Span)
        .on_request(())
        .on_response(RequestOnResponse)
        .on_eos(())
        .on_failure(
            DefaultOnFailure::new()
                .level(Level::WARN)
                .latency_unit(LatencyUnit::Millis),
        )
}

/// Polls and assets repeated every few seconds by every open operator tab and
/// stage display: the resolume / AI status and `/healthz` polls, the NDI
/// clock and client-stats posts, and the WASM bundle files.
pub(super) fn is_quiet_path(path: &str) -> bool {
    const POLLS: [&str; 5] = [
        "/healthz",
        "/ai/status",
        "/integrations/resolume/status",
        "/ndi/time",
        "/ndi/client-stats",
    ];
    POLLS.contains(&path)
        || path.starts_with("/ui-pkg/")
        || path
            .strip_prefix("/ndi/sessions/")
            .and_then(|rest| rest.strip_suffix("/client-stats"))
            .is_some_and(|session_id| !session_id.is_empty() && !session_id.contains('/'))
}

pub(super) fn request_fields<B>(request: &Request<B>) -> RequestFields {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    let uri = request.uri();
    RequestFields {
        method: request.method().to_string(),
        uri: uri
            .path_and_query()
            .map_or_else(|| uri.path().to_string(), ToString::to_string),
        user_agent: request
            .headers()
            .get(USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("-")
            .to_string(),
        client_ip: super::integrations::extract_client_ip(request.headers(), peer),
    }
}

fn request_span(request: &Request<Body>) -> Span {
    if is_quiet_path(request.uri().path()) {
        return Span::none();
    }
    let fields = request_fields(request);
    tracing::span!(
        target: "tower_http::trace::make_span",
        Level::DEBUG,
        "request",
        method = %fields.method,
        uri = fields.uri.as_str(),
        user_agent = fields.user_agent.as_str(),
        client_ip = fields.client_ip.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::build_router;
    use crate::router::tracing_capture::{quoted, Capture};
    use crate::state::AppState;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    const MAKE_SPAN: &str = "tower_http::trace::make_span";
    const ON_RESPONSE: &str = "tower_http::trace::on_response";
    const ON_EOS: &str = "tower_http::trace::on_eos";
    const ON_FAILURE: &str = "tower_http::trace::on_failure";

    fn request(uri: &str, peer: Option<&str>, forwarded: Option<&str>) -> Request<()> {
        let mut builder = Request::builder()
            .uri(uri)
            .header(USER_AGENT, "Mozilla/5.0 (iPhone)");
        if let Some(value) = forwarded {
            builder = builder.header("x-forwarded-for", value);
        }
        let mut request = builder.body(()).expect("request");
        if let Some(addr) = peer {
            let addr: SocketAddr = addr.parse().expect("socket address");
            request.extensions_mut().insert(ConnectInfo(addr));
        }
        request
    }

    /// GET `uri` through `app` as a phone behind the tunnel, drain the body (the
    /// end-of-stream callback fires there) and return the status.
    async fn call(app: &Router, uri: &str) -> StatusCode {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(USER_AGENT, "Mozilla/5.0 (iPhone)")
                    .header("x-forwarded-for", "203.0.113.9")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        status
    }

    #[test]
    fn fields_keep_the_query_and_the_user_agent() {
        let request = request(
            "/bible/books?translation=slk-seb",
            Some("192.0.2.7:51000"),
            None,
        );
        assert_eq!(
            request_fields(&request),
            RequestFields {
                method: "GET".to_string(),
                uri: "/bible/books?translation=slk-seb".to_string(),
                user_agent: "Mozilla/5.0 (iPhone)".to_string(),
                client_ip: "192.0.2.7".to_string(),
            }
        );
    }

    #[test]
    fn forwarded_for_counts_only_from_a_loopback_peer() {
        let tunnel = request("/healthz", Some("127.0.0.1:40000"), Some("203.0.113.9"));
        assert_eq!(request_fields(&tunnel).client_ip, "203.0.113.9");
        let lan = request("/healthz", Some("192.0.2.7:40000"), Some("203.0.113.9"));
        assert_eq!(request_fields(&lan).client_ip, "192.0.2.7");
    }

    #[test]
    fn a_bare_request_logs_dashes_and_anonymous() {
        let bare = Request::builder().uri("/").body(()).expect("request");
        let fields = request_fields(&bare);
        assert_eq!(fields.user_agent, "-");
        assert_eq!(fields.client_ip, "anonymous");
    }

    #[test]
    fn the_polls_and_the_assets_are_quiet() {
        for path in [
            "/healthz",
            "/ai/status",
            "/integrations/resolume/status",
            "/ndi/time",
            "/ndi/client-stats",
            "/ndi/sessions/6f1a9c52-0d4e/client-stats",
            "/ui-pkg/presenter_ui.js",
            "/ui-pkg/snippets/nested/app_bg.wasm",
        ] {
            assert!(is_quiet_path(path), "{path} must be quiet");
        }
    }

    #[test]
    fn every_other_path_stays_logged() {
        for path in [
            "/",
            "/ndi/status",
            "/ndi/whep/abc",
            "/ndi/sessions/6f1a9c52-0d4e/whep",
            "/ndi/sessions//client-stats",
            "/ndi/sessions/a/b/client-stats",
            "/ndi/client-stats/extra",
            "/healthz/extra",
            "/ui-pkg",
            "/ai/chat",
            "/bible/trigger-slide",
            "/integrations/resolume/hosts",
        ] {
            assert!(!is_quiet_path(path), "{path} must be logged");
        }
    }

    #[tokio::test]
    async fn a_request_opens_the_request_span_with_quoted_client_fields() {
        let capture = Capture::install();
        let app = build_router(AppState::in_memory().await.expect("state"));

        let status = call(&app, "/bible/active-slide?probe=1").await;

        assert_eq!(status, StatusCode::OK);
        let spans = capture.spans(MAKE_SPAN);
        assert_eq!(spans.len(), 1, "one request span per request: {spans:?}");
        let span = &spans[0];
        assert_eq!(span.level, Level::DEBUG);
        assert_eq!(span.name, "request");
        assert_eq!(span.field("method"), Some("GET"));
        assert_eq!(
            span.field("uri"),
            Some(quoted("/bible/active-slide?probe=1").as_str())
        );
        assert_eq!(
            span.field("user_agent"),
            Some(quoted("Mozilla/5.0 (iPhone)").as_str())
        );
        assert_eq!(
            span.field("client_ip"),
            Some(quoted("203.0.113.9").as_str())
        );
    }

    #[tokio::test]
    async fn a_response_logs_one_debug_line_and_no_end_of_stream_line() {
        let capture = Capture::install();
        let app = build_router(AppState::in_memory().await.expect("state"));

        call(&app, "/bible/active-slide").await;

        let responses = capture.events(ON_RESPONSE);
        assert_eq!(responses.len(), 1, "one response line: {responses:?}");
        assert_eq!(responses[0].level, Level::DEBUG);
        assert_eq!(responses[0].field("status"), Some("200"));
        assert!(
            capture.events(ON_EOS).is_empty(),
            "a drained body must not log a second line"
        );
    }

    #[tokio::test]
    async fn a_quiet_poll_leaves_no_span_and_no_response_line() {
        let capture = Capture::install();
        let app = build_router(AppState::in_memory().await.expect("state"));

        let quiet = call(&app, "/healthz").await;
        assert_eq!(quiet, StatusCode::OK);
        assert!(capture.spans(MAKE_SPAN).is_empty(), "no span for /healthz");
        assert!(capture.events(ON_RESPONSE).is_empty());
        assert!(capture.events(ON_EOS).is_empty());

        // The capture is live: the next, non-quiet request IS logged.
        call(&app, "/bible/active-slide").await;
        assert_eq!(capture.spans(MAKE_SPAN).len(), 1);
        assert_eq!(capture.events(ON_RESPONSE).len(), 1);
    }

    #[tokio::test]
    async fn a_5xx_stays_visible_as_a_warning_even_on_a_quiet_path() {
        let capture = Capture::install();
        let app = Router::new()
            .route(
                "/ndi/time",
                get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
            )
            .route("/boom", get(|| async { StatusCode::SERVICE_UNAVAILABLE }))
            .layer(request_trace_layer());

        assert_eq!(
            call(&app, "/ndi/time").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(call(&app, "/boom").await, StatusCode::SERVICE_UNAVAILABLE);

        let failures = capture.events(ON_FAILURE);
        assert_eq!(failures.len(), 2, "both 5xx are logged: {failures:?}");
        assert!(
            failures.iter().all(|line| line.level == Level::WARN),
            "routine 503 back-pressure is a WARN, never an ERROR: {failures:?}"
        );
        // Only the non-quiet request has a span and a DEBUG response line.
        assert_eq!(capture.spans(MAKE_SPAN).len(), 1);
        assert_eq!(capture.events(ON_RESPONSE).len(), 1);
    }
}
