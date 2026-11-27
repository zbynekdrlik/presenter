//! The guests' own hostname (#999, ROZHODNUTÉ 27.11): a request whose `Host`
//! is `PRESENTER_GUEST_HOST` gets ONLY the guest surface
//! ([`listen_guest::guest_routes`](super::listen_guest::guest_routes)); every
//! other path is a 404, because that hostname is a public tunnel with no
//! login in front of it. Every other host gets the normal router, which never
//! serves the guest page at `/`. Without a guest hostname nothing changes.
//!
//! The match reads the `Host` header (cloudflared passes the tunnel
//! hostname) AND the URI authority (HTTP/2 `:authority`, an absolute-form
//! request): either one naming the guest host makes it a guest request — the
//! safe direction. Lowercase, without the port. `X-Forwarded-Host` is never
//! read — a client could set it to reach the normal router through the guest
//! tunnel.
//!
//! What this cannot see: a tunnel ingress that REWRITES the Host header
//! (`originRequest.httpHostHeader`), or a box whose `PRESENTER_GUEST_HOST`
//! went missing while the tunnel still points at it — both would hand the
//! public hostname the whole app. The deploy checks the boundary after every
//! start (`deploy-site.yml` "Verify deployment": `/ui/operator`, `/listen/status` …
//! through that Host must be 404).

use axum::extract::Request;
use axum::http::header::HOST;
use axum::Router;
use tower::ServiceExt;

use crate::listen::translate::config::bare_host;
use crate::listen::translate::TARGET;
use crate::state::AppState;

/// Whether `request` came through the guest hostname.
pub(super) fn is_guest_request(request: &Request, guest_host: &str) -> bool {
    let header = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| bare_host(host) == guest_host);
    let authority = request
        .uri()
        .host()
        .is_some_and(|host| bare_host(host) == guest_host);
    header || authority
}

/// Chooses the router BEFORE any routing: the guest router for the guest
/// hostname, `app` for every other host (`app` unchanged without a guest
/// host). Never a `Router::layer` — that runs after the outer router matched,
/// and its path parameters would reach the guest router's extractors.
pub(super) fn with_guest_host(app: Router, state: &AppState) -> Router {
    let Some(guest_host) = state.guest_host() else {
        tracing::info!(target: TARGET, "no guest hostname (PRESENTER_GUEST_HOST unset): the guest page is not served");
        return app;
    };
    tracing::info!(target: TARGET, guest_host = guest_host.as_str(), "guest hostname: only the guest surface is served there");
    let guest = super::request_log::with_request_log(super::listen_guest::guest_routes())
        .with_state(state.clone());
    let dispatch = tower::service_fn(move |request: Request| {
        let target = if is_guest_request(&request, &guest_host) {
            guest.clone()
        } else {
            app.clone()
        };
        async move { target.oneshot(request).await }
    });
    Router::new().fallback_service(dispatch)
}

#[cfg(test)]
mod tests;
