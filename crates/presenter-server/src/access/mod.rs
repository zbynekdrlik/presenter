//! Who is reading (#915): the e-mail of a person Cloudflare Access verified, and
//! the device that person used before.
//!
//! - **A verified login** = a `Cf-Access-Jwt-Assertion` token that `jwt` verifies
//!   (RS256 with the team's published keys, the site's AUD, the team issuer). The
//!   bare `Cf-Access-Authenticated-User-Email` header is never read: nothing
//!   proves it came from Cloudflare.
//! - **A remembered device** = the `presenter_device` cookie the server hands out
//!   after a verified login (`device_cookie`). At church the domain is reached
//!   through the Access BYPASS for the church IP (no login, no token), so this
//!   cookie is how the server still knows the person there.
//! - **Neither** (the LAN address, a device that never logged in) = nobody, and
//!   every reader gets the shared list, as before #915.
//!
//! The e-mail is a convenience scope (whose bookmarks, the sermon author label),
//! never a permission. Logs: target `presenter::access`; the cookie value and the
//! token are never logged.

mod device_cookie;
mod jwt;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::path::PathBuf;

use axum::http::HeaderMap;

pub(crate) use device_cookie::DeviceKey;
pub(crate) use jwt::{JwtConfig, JwtVerifier};

const LOG_TARGET: &str = "presenter::access";
const JWT_HEADER: &str = "cf-access-jwt-assertion";
const MAX_EMAIL_LEN: usize = 254;
/// The key file's name next to the site's sqlite file.
const DEVICE_KEY_FILE_NAME: &str = "access-device.key";

pub(crate) const TEAM_DOMAIN_ENV: &str = "PRESENTER_ACCESS_TEAM_DOMAIN";
pub(crate) const AUD_ENV: &str = "PRESENTER_ACCESS_AUD";
pub(crate) const CERTS_URL_ENV: &str = "PRESENTER_ACCESS_CERTS_URL";
pub(crate) const DEVICE_KEY_FILE_ENV: &str = "PRESENTER_ACCESS_DEVICE_KEY_FILE";

/// Where an identity came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentitySource {
    Jwt,
    DeviceCookie,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub(crate) email: String,
    pub(crate) source: IdentitySource,
}

/// What the server knows about one request.
#[derive(Debug, Default)]
pub(crate) struct Resolution {
    pub(crate) identity: Option<Identity>,
    /// A `Set-Cookie` value to add to the response (a new or refreshed device).
    pub(crate) set_cookie: Option<String>,
}

/// The facts of a request the resolution needs, read by the router glue.
#[derive(Clone, Copy)]
pub(crate) struct RequestFacts<'a> {
    pub(crate) headers: &'a HeaderMap,
    /// The socket peer is loopback (`cloudflared`) or unknown (tests): only then
    /// can a token have come from Cloudflare.
    pub(crate) from_tunnel: bool,
    /// The browser reached the site over https (`X-Forwarded-Proto: https`).
    pub(crate) https: bool,
    /// Unix seconds.
    pub(crate) now: i64,
}

/// The identity service, one per site process, shared by every `AppState` clone.
pub(crate) struct AccessIdentity {
    jwt: Option<JwtVerifier>,
    device: Option<DeviceKey>,
}

impl AccessIdentity {
    pub(crate) fn new(jwt: Option<JwtVerifier>, device: Option<DeviceKey>) -> Self {
        Self { jwt, device }
    }

    /// From the process environment: the team + AUD of this site's Access app
    /// (missing → no verified logins, one WARN) and the device key next to the
    /// sqlite file (an in-memory database → an ephemeral key).
    pub(crate) fn from_env() -> Self {
        let jwt = match JwtConfig::parse(
            env_value(TEAM_DOMAIN_ENV).as_deref(),
            env_value(AUD_ENV).as_deref(),
            env_value(CERTS_URL_ENV).as_deref(),
        ) {
            Ok(config) => {
                tracing::info!(
                    target: LOG_TARGET,
                    team = config.team_domain(),
                    audiences = config.audiences().len(),
                    "Cloudflare Access logins are verified"
                );
                Some(JwtVerifier::new(config))
            }
            Err(reason) => {
                tracing::warn!(
                    target: LOG_TARGET,
                    reason,
                    "Cloudflare Access is not configured: no reader is identified by a login \
                     (set {TEAM_DOMAIN_ENV} and {AUD_ENV})"
                );
                None
            }
        };
        let location = device_key_location(
            std::env::var_os(DEVICE_KEY_FILE_ENV),
            &std::env::var("PRESENTER_DB_URL").unwrap_or_default(),
        );
        Self::new(jwt, DeviceKey::open(location.as_deref()))
    }

    /// Who sent this request, and a device cookie to hand out when a verified
    /// login has no current one.
    pub(crate) async fn resolve(&self, facts: RequestFacts<'_>) -> Resolution {
        let device = self
            .device
            .as_ref()
            .and_then(|key| key.read(facts.headers, facts.now));
        if let Some(email) = self.verified_login(facts).await {
            let set_cookie = self.cookie_for(&email, device.as_ref(), facts);
            tracing::debug!(target: LOG_TARGET, source = "jwt", email = %email, "reader identified");
            return Resolution {
                identity: Some(Identity {
                    email,
                    source: IdentitySource::Jwt,
                }),
                set_cookie,
            };
        }
        match device {
            Some(device) => {
                tracing::debug!(
                    target: LOG_TARGET,
                    source = "device-cookie",
                    email = %device.email,
                    "reader identified"
                );
                Resolution {
                    identity: Some(Identity {
                        email: device.email,
                        source: IdentitySource::DeviceCookie,
                    }),
                    set_cookie: None,
                }
            }
            None => {
                tracing::debug!(target: LOG_TARGET, source = "none", "reader not identified");
                Resolution::default()
            }
        }
    }

    /// The e-mail of a token that verifies; a token that does not is logged.
    async fn verified_login(&self, facts: RequestFacts<'_>) -> Option<String> {
        let token = facts.headers.get(JWT_HEADER)?.to_str().ok()?;
        if !facts.from_tunnel {
            tracing::debug!(target: LOG_TARGET, "Access token from a LAN peer ignored");
            return None;
        }
        let Some(verifier) = &self.jwt else {
            tracing::debug!(target: LOG_TARGET, "Access token ignored: Access is not configured");
            return None;
        };
        match verifier.verify(token).await {
            Ok(email) => Some(email),
            Err(err) => {
                tracing::warn!(target: LOG_TARGET, reason = %err, "Access token refused");
                None
            }
        }
    }

    /// A `Set-Cookie` for a verified login whose device has no current cookie
    /// for this e-mail (or one older than the refresh age).
    fn cookie_for(
        &self,
        email: &str,
        current: Option<&device_cookie::DeviceClaims>,
        facts: RequestFacts<'_>,
    ) -> Option<String> {
        let key = self.device.as_ref()?;
        if current.is_some_and(|claims| !claims.needs_refresh(email, facts.now)) {
            return None;
        }
        tracing::info!(
            target: LOG_TARGET,
            email = %email,
            secure = facts.https,
            refresh = current.is_some(),
            "device remembered for the reader"
        );
        Some(key.issue(email, facts.now, facts.https))
    }
}

/// The trimmed, lower-case e-mail; `None` when the value is no e-mail (no `@`,
/// longer than 254 characters).
pub(crate) fn normalize_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_lowercase();
    (email.contains('@') && email.chars().count() <= MAX_EMAIL_LEN).then_some(email)
}

/// The device key file: `explicit` when set, else `access-device.key` next to
/// the sqlite file of `db_url`; `None` for an in-memory or empty url.
pub(crate) fn device_key_location(explicit: Option<OsString>, db_url: &str) -> Option<PathBuf> {
    if let Some(path) = explicit.filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let db_file = crate::state::stream_assets::sqlite_file_path(db_url)?;
    let parent = db_file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    Some(parent.join(DEVICE_KEY_FILE_NAME))
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
