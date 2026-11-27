//! The guests' own hostname (#999, ROZHODNUTÉ 27.11): a request whose `Host`
//! is `PRESENTER_GUEST_HOST` gets ONLY the guest surface
//! ([`listen_guest::guest_routes`](super::listen_guest::guest_routes)); every
//! other path is a 404, because that hostname is a public tunnel with no
//! login in front of it. Every other host gets the normal router, which never
//! serves the guest page at `/`. Without a guest hostname nothing changes.
//!
//! The match reads the `Host` header (cloudflared passes the tunnel
//! hostname), lowercase without its port; `X-Forwarded-Host` is never read —
//! a client could set it to reach the normal router through the guest tunnel.

use axum::extract::Request;
use axum::http::header::HOST;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tower::ServiceExt;

use crate::listen::translate::config::bare_host;
use crate::listen::translate::TARGET;
use crate::state::AppState;

/// Whether `request` came through the guest hostname.
pub(super) fn is_guest_request(request: &Request, guest_host: &str) -> bool {
    let host = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or_else(|| request.uri().host().map(str::to_string));
    host.is_some_and(|host| bare_host(&host) == guest_host)
}

/// Puts the guest dispatch in front of `router` (no-op without a guest host).
pub(super) fn with_guest_host(router: Router<AppState>, state: &AppState) -> Router<AppState> {
    let Some(guest_host) = state.guest_host() else {
        tracing::info!(target: TARGET, "no guest hostname (PRESENTER_GUEST_HOST unset): the guest page is not served");
        return router;
    };
    tracing::info!(target: TARGET, guest_host = guest_host.as_str(), "guest hostname: only the guest surface is served there");
    let guest = super::listen_guest::guest_routes().with_state(state.clone());
    router.layer(middleware::from_fn(move |request: Request, next: Next| {
        let guest = guest.clone();
        let guest_host = guest_host.clone();
        async move { dispatch(request, next, guest, &guest_host).await }
    }))
}

async fn dispatch(request: Request, next: Next, guest: Router, guest_host: &str) -> Response {
    if !is_guest_request(&request, guest_host) {
        return next.run(request).await;
    }
    match guest.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
    .into_response()
}

#[cfg(test)]
mod tests;
