//! HTTP request log (#850 spec §8): a DEBUG span per request (method, path +
//! query, user agent, client IP) and a DEBUG line per response (status,
//! latency in ms). The units' `RUST_LOG=info,tower_http=debug,sqlx=warn`
//! enables both — the span uses tower-http's own `make_span` target so that
//! filter covers it. Client IP follows the live WS rule (#793): X-Forwarded-For
//! counts only from a loopback peer (cloudflared); a LAN peer is the client
//! itself.
//!
//! Polls and assets that every open tab / stage display repeats every few
//! seconds ([`is_quiet_request`]) get NO span and NO response line, so the
//! journal keeps the requests a person made. A 5xx is never hidden:
//! tower-http's `on_failure` still logs it (WARN, without the request fields on
//! a quiet path). Client-controlled strings are recorded as `&str`, so they
//! print quoted and a hostile header cannot forge a field.
//!
//! One poll cannot be told apart before it is answered: a stage display whose
//! NDI broadcaster is silent POSTs `/ndi/whep/{id}` every 5 s and gets a 204
//! ([`is_silent_whep_poll`]). Its span exists (the status is unknown when the
//! span opens) but its response line is skipped; a 201 session start and every
//! error stay logged. [`with_request_log`] installs the layer pair.

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header::USER_AGENT, Method, Request, Response, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::Router;
use tower_http::classify::{ServerErrorsAsFailures, SharedClassifier};
use tower_http::trace::{
    DefaultOnBodyChunk, DefaultOnFailure, DefaultOnResponse, OnResponse, TraceLayer,
};
use tower_http::LatencyUnit;
use tracing::{Level, Span};

/// The trace layer: our span (none for a quiet path), no "started" line, one
/// DEBUG response line (none for a silent WHEP poll), no "end of stream" line,
/// a WARN line for a 5xx.
type RequestTraceLayer = TraceLayer<
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

/// The request a response answers. `OnResponse` sees only the response, so
/// [`tag_request_line`] (installed INSIDE the trace layer) stamps it on the
/// response extensions.
#[derive(Debug, Clone)]
struct RequestLine {
    method: Method,
    uri: Uri,
}

/// Stamp the request's method and URI on its response (both clone without a
/// heap copy), for [`RequestOnResponse`] to read.
async fn tag_request_line(request: Request<Body>, next: Next) -> Response<Body> {
    let line = RequestLine {
        method: request.method().clone(),
        uri: request.uri().clone(),
    };
    let mut response = next.run(request).await;
    response.extensions_mut().insert(line);
    response
}

/// tower-http's DEBUG response line (status, latency in ms), skipped when the
/// request span is disabled — a quiet path (`Span::none()`) has nothing to
/// attach the line to — and for a silent WHEP poll ([`is_silent_whep_poll`]).
#[derive(Debug, Clone, Copy)]
pub(super) struct RequestOnResponse;

impl<B> OnResponse<B> for RequestOnResponse {
    fn on_response(self, response: &Response<B>, latency: Duration, span: &Span) {
        if span.is_disabled() || answers_a_silent_whep_poll(response) {
            return;
        }
        DefaultOnResponse::new()
            .level(Level::DEBUG)
            .latency_unit(LatencyUnit::Millis)
            .on_response(response, latency, span);
    }
}

/// Install the request log on `router`: the trace layer OUTSIDE, the tagging
/// middleware inside it (axum applies the layer added last first, so the tag is
/// already on the response when the trace layer logs it).
pub(super) fn with_request_log<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .layer(middleware::from_fn(tag_request_line))
        .layer(request_trace_layer())
}

fn request_trace_layer() -> RequestTraceLayer {
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

/// A stage display's WHEP offer while its broadcaster is silent: `POST
/// /ndi/whep/{source_id}` answered 204 No Content. The reconnect loop repeats it
/// every 5 s per display (~720 lines/h each). A 201 session start, an error and
/// every other WHEP request (the session-scoped POST / PATCH / DELETE) are not.
pub(super) fn is_silent_whep_poll(method: &Method, path: &str, status: StatusCode) -> bool {
    *method == Method::POST
        && status == StatusCode::NO_CONTENT
        && path
            .strip_prefix("/ndi/whep/")
            .is_some_and(|source_id| !source_id.is_empty() && !source_id.contains('/'))
}

fn answers_a_silent_whep_poll<B>(response: &Response<B>) -> bool {
    response
        .extensions()
        .get::<RequestLine>()
        .is_some_and(|line| is_silent_whep_poll(&line.method, line.uri.path(), response.status()))
}

/// Polls and assets repeated every few seconds by every open operator tab and
/// stage display, whatever the method: the resolume / AI status and `/healthz`
/// polls, the NDI clock and client-stats posts, and the WASM bundle files.
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

/// Quiet by path ([`is_quiet_path`]), or a timer-polled GET ([`is_quiet_get_path`]).
pub(super) fn is_quiet_request(method: &Method, path: &str) -> bool {
    is_quiet_path(path) || (*method == Method::GET && is_quiet_get_path(path))
}

/// The list / status GETs a page re-fetches on a timer: the tablet's 10 s
/// presentation list (`pages/tablet.rs`) and the Settings tab's 5 s pollers (the
/// Resolume and Android lists in `list_card.rs`, AbleSet + OSC status in
/// `ableton.rs`, NDI source status in `video_sources.rs`) and the listening
/// microphones' status (`/listen/status`, #852 C1a). Only the GET is
/// quiet: the same paths take a person's POST / PUT / DELETE (create, edit,
/// remove), and those stay logged.
pub(super) fn is_quiet_get_path(path: &str) -> bool {
    const POLLED_GETS: [&str; 7] = [
        "/bible/presentations",
        "/integrations/resolume/hosts",
        "/integrations/android-stage/displays",
        "/integrations/ableset/status",
        "/integrations/osc/status",
        "/integrations/video-sources/status",
        "/listen/status",
    ];
    POLLED_GETS.contains(&path)
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
    if is_quiet_request(request.method(), request.uri().path()) {
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
    use crate::state::AppState;
    use crate::tracing_capture::{quoted, Capture};
    use axum::routing::{delete, get, post};
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
        call_with(app, Method::GET, uri).await
    }

    /// `method` `uri` through `app`, like [`call`].
    async fn call_with(app: &Router, method: Method, uri: &str) -> StatusCode {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
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
        // WHEP stays a logged REQUEST (span + response line); only the 204 answer to the
        // offer POST is dropped, at response time (`is_silent_whep_poll`, tested below).
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

    #[test]
    fn the_timer_polled_gets_are_quiet_but_a_persons_writes_are_not() {
        for path in [
            "/bible/presentations",
            "/integrations/resolume/hosts",
            "/integrations/android-stage/displays",
            "/integrations/ableset/status",
            "/integrations/osc/status",
            "/integrations/video-sources/status",
            "/listen/status",
        ] {
            assert!(is_quiet_request(&Method::GET, path), "GET {path} is polled");
            assert!(!is_quiet_path(path), "{path} is quiet by GET only");
            for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
                assert!(
                    !is_quiet_request(&method, path),
                    "{method} {path} is a person's write and stays logged"
                );
            }
        }
    }

    #[test]
    fn a_get_nobody_polls_stays_logged_and_the_quiet_paths_ignore_the_method() {
        for path in [
            "/bible/presentations/abc",
            "/bible/presentations/",
            "/integrations/video-sources",
            "/integrations/resolume/hosts/abc",
            "/integrations/android-stage/displays/abc",
            "/integrations/ableset/settings",
            "/integrations/osc/status/extra",
            "/settings/features",
            "/listen/mics",
        ] {
            assert!(
                !is_quiet_request(&Method::GET, path),
                "GET {path} must be logged"
            );
        }
        assert!(is_quiet_request(&Method::POST, "/ndi/client-stats"));
        assert!(is_quiet_request(&Method::GET, "/healthz"));
        assert!(is_quiet_request(&Method::GET, "/ui-pkg/presenter_ui.js"));
    }

    #[test]
    fn only_a_204_to_the_whep_offer_post_is_a_silent_poll() {
        let (offer, no_content) = (&Method::POST, StatusCode::NO_CONTENT);
        assert!(is_silent_whep_poll(
            offer,
            "/ndi/whep/STREAM-SNV",
            no_content
        ));
        // A session start and every error stay logged.
        for status in [
            StatusCode::CREATED,
            StatusCode::OK,
            StatusCode::NOT_FOUND,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert!(
                !is_silent_whep_poll(offer, "/ndi/whep/STREAM-SNV", status),
                "POST whep answered {status} must be logged"
            );
        }
        // Another method, another path, or a session-scoped WHEP path is not the poll.
        assert!(!is_silent_whep_poll(
            &Method::GET,
            "/ndi/whep/STREAM-SNV",
            no_content
        ));
        assert!(!is_silent_whep_poll(offer, "/ndi/status", no_content));
        for path in [
            "/ndi/whep/",
            "/ndi/whep",
            "/ndi/whep/a/b",
            "/ndi/whep/a/b/c",
        ] {
            assert!(
                !is_silent_whep_poll(offer, path, no_content),
                "POST {path} {no_content} must be logged"
            );
        }
        for method in [Method::DELETE, Method::PATCH, Method::PUT] {
            assert!(
                !is_silent_whep_poll(&method, "/ndi/whep/STREAM-SNV", no_content),
                "{method} whep answered 204 must be logged"
            );
            assert!(
                !is_silent_whep_poll(&method, "/ndi/whep/STREAM-SNV/6f1a9c52", no_content),
                "{method} whep session answered 204 must be logged"
            );
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

        // The tablet's 10 s list refresh is a quiet GET too (#850 O1 review).
        let list = call(&app, "/bible/presentations").await;
        assert_eq!(list, StatusCode::OK);
        assert!(capture.spans(MAKE_SPAN).is_empty(), "no span for the poll");
        assert!(capture.events(ON_RESPONSE).is_empty());

        // The capture is live: the next, non-quiet request IS logged.
        call(&app, "/bible/active-slide").await;
        assert_eq!(capture.spans(MAKE_SPAN).len(), 1);
        assert_eq!(capture.events(ON_RESPONSE).len(), 1);
    }

    /// A router answering the WHEP shapes with fixed statuses, behind the real
    /// request log.
    fn whep_app() -> Router {
        with_request_log(
            Router::new()
                .route("/ndi/whep/idle", post(|| async { StatusCode::NO_CONTENT }))
                .route("/ndi/whep/live", post(|| async { StatusCode::CREATED }))
                .route(
                    "/ndi/whep/busy",
                    post(|| async { StatusCode::SERVICE_UNAVAILABLE }),
                )
                .route(
                    "/ndi/whep/live/s1",
                    delete(|| async { StatusCode::NO_CONTENT }),
                )
                .route("/ndi/other", post(|| async { StatusCode::NO_CONTENT })),
        )
    }

    #[tokio::test]
    async fn a_silent_whep_poll_logs_no_response_line_and_every_other_answer_does() {
        let capture = Capture::install();
        let app = whep_app();

        // The silent broadcaster's 5 s reconnect poll: answered 204, no line.
        let idle = call_with(&app, Method::POST, "/ndi/whep/idle").await;
        assert_eq!(idle, StatusCode::NO_CONTENT);
        assert!(
            capture.events(ON_RESPONSE).is_empty(),
            "a silent WHEP poll must not log a response line: {:?}",
            capture.events(ON_RESPONSE)
        );

        // Everything else about WHEP stays visible: the session start, the 503
        // back-pressure (response line AND WARN), the teardown DELETE answered
        // 204, and a 204 from another path.
        for (method, uri) in [
            (Method::POST, "/ndi/whep/live"),
            (Method::POST, "/ndi/whep/busy"),
            (Method::DELETE, "/ndi/whep/live/s1"),
            (Method::POST, "/ndi/other"),
        ] {
            call_with(&app, method, uri).await;
        }
        let responses = capture.events(ON_RESPONSE);
        let statuses: Vec<Option<&str>> =
            responses.iter().map(|line| line.field("status")).collect();
        assert_eq!(
            statuses,
            vec![Some("201"), Some("503"), Some("204"), Some("204")],
            "every non-silent answer keeps its response line: {responses:?}"
        );
        let failures = capture.events(ON_FAILURE);
        assert_eq!(failures.len(), 1, "the 503 stays a WARN: {failures:?}");
        assert_eq!(failures[0].level, Level::WARN);
    }

    #[tokio::test]
    async fn a_5xx_stays_visible_as_a_warning_even_on_a_quiet_path() {
        let capture = Capture::install();
        let app = with_request_log(
            Router::new()
                .route(
                    "/ndi/time",
                    get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
                )
                .route("/boom", get(|| async { StatusCode::SERVICE_UNAVAILABLE })),
        );

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
