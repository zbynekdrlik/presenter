//! HTTP request log (#850 spec §8): a DEBUG span per request (method, path +
//! query, user agent, client IP) and a DEBUG line per response (status,
//! latency in ms). The units' `RUST_LOG=info,tower_http=debug` enables both —
//! the span uses tower-http's own `make_span` target so that filter covers it.
//! Client IP follows the live WS rule (#793): X-Forwarded-For counts only from
//! a loopback peer (cloudflared); a LAN peer is the client itself.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header::USER_AGENT, Request};
use tower_http::classify::{ServerErrorsAsFailures, SharedClassifier};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tower_http::LatencyUnit;
use tracing::{Level, Span};

/// The layer `build_router` installs: our span, no "started" line, one
/// DEBUG response line, tower-http's default ERROR line for a 5xx.
pub(super) type RequestTraceLayer =
    TraceLayer<SharedClassifier<ServerErrorsAsFailures>, fn(&Request<Body>) -> Span, ()>;

/// What the request span carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RequestFields {
    pub(super) method: String,
    pub(super) uri: String,
    pub(super) user_agent: String,
    pub(super) client_ip: String,
}

pub(super) fn request_trace_layer() -> RequestTraceLayer {
    TraceLayer::new_for_http()
        .make_span_with(request_span as fn(&Request<Body>) -> Span)
        .on_request(())
        .on_response(
            DefaultOnResponse::new()
                .level(Level::DEBUG)
                .latency_unit(LatencyUnit::Millis),
        )
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
    let fields = request_fields(request);
    tracing::span!(
        target: "tower_http::trace::make_span",
        Level::DEBUG,
        "request",
        method = %fields.method,
        uri = %fields.uri,
        user_agent = %fields.user_agent,
        client_ip = %fields.client_ip
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
