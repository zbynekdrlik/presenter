//! The reader of a request (#915), resolved ONCE per request by a middleware
//! and read by every handler that needs it (study bookmarks, the sermon author
//! label, `GET /bible/study/identity`).
//!
//! The rules are `crate::access`: a verified Cloudflare Access token, else a
//! remembered-device cookie, else nobody (the shared list). A token counts only
//! from a loopback peer (`cloudflared` on the same box) or with no peer at all
//! (tests) — the #793 `extract_client_ip` rule — because a LAN client could send
//! the header itself. The bare `Cf-Access-Authenticated-User-Email` header is
//! never read: nothing proves that Cloudflare set it.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::header::{CACHE_CONTROL, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;

use crate::access::{Identity, RequestFacts};
use crate::state::AppState;

/// The resolved reader, stored in the request extensions by the middleware.
#[derive(Clone, Debug, Default)]
pub(crate) struct RequestIdentity(pub(crate) Option<Identity>);

/// The reader's e-mail; `None` = nobody (also when the middleware did not run).
pub(crate) struct AccessEmail(pub(crate) Option<String>);

impl<S: Send + Sync> FromRequestParts<S> for AccessEmail {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<RequestIdentity>()
                .and_then(|identity| identity.0.as_ref())
                .map(|identity| identity.email.clone()),
        ))
    }
}

/// Install the identity middleware on every route of `router`.
pub(super) fn with_access_identity(router: Router<AppState>, state: &AppState) -> Router<AppState> {
    router.layer(middleware::from_fn_with_state(
        state.clone(),
        resolve_identity,
    ))
}

async fn resolve_identity(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    let from_tunnel = super::integrations::is_tunnel_peer(peer);
    let facts = RequestFacts {
        headers: request.headers(),
        from_tunnel,
        https: from_tunnel && forwarded_https(request.headers()),
        now: chrono::Utc::now().timestamp(),
    };
    let resolution = state.access_identity().resolve(facts).await;
    request
        .extensions_mut()
        .insert(RequestIdentity(resolution.identity));
    let mut response = next.run(request).await;
    if let Some(cookie) = resolution.set_cookie {
        attach_cookie(&mut response, &cookie);
    }
    response
}

/// Add the device cookie only to a response no shared cache keeps (an asset is
/// `public, immutable`: a cache could hand the cookie to another browser). The
/// next document or API answer carries it instead. A response without its own
/// `Cache-Control` is marked `private, no-store`.
fn attach_cookie(response: &mut Response, cookie: &str) {
    if !may_carry_cookie(response.status(), response.headers()) {
        tracing::debug!(target: "presenter::access", "device cookie held back from a cacheable response");
        return;
    }
    let Ok(value) = HeaderValue::from_str(cookie) else {
        tracing::warn!(target: "presenter::access", "device cookie not set: the value is no header value");
        return;
    };
    let headers = response.headers_mut();
    headers.append(SET_COOKIE, value);
    if !headers.contains_key(CACHE_CONTROL) {
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    }
}

fn may_carry_cookie(status: StatusCode, headers: &HeaderMap) -> bool {
    if status == StatusCode::SWITCHING_PROTOCOLS {
        return false;
    }
    match headers
        .get(CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
    {
        None => true,
        Some(value) => {
            let value = value.to_ascii_lowercase();
            value.contains("no-store") || value.contains("private")
        }
    }
}

/// The browser came over https: `cloudflared` forwards `X-Forwarded-Proto`.
fn forwarded_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|proto| proto.trim().eq_ignore_ascii_case("https"))
}

#[cfg(test)]
mod tests;
