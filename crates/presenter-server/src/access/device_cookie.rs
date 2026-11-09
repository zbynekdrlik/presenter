//! The remembered device (#915): `presenter_device=<payload>.<signature>`, set
//! after a verified Access login so the same browser is still known at church,
//! where the Access bypass sends no login.
//!
//! - payload = base64url(`v1|<issued unix seconds>|<e-mail>`), signature =
//!   base64url(HMAC-SHA256(site key, payload)). The e-mail is no secret; the
//!   signature stops anybody from naming another person.
//! - `HttpOnly; SameSite=Lax; Path=/; Max-Age` = one year, `Secure` when the
//!   browser came over https. The server also refuses a payload older than a
//!   year, whatever the browser kept.
//! - The site key = 32 random bytes in `access-device.key` (hex, 0600) next to
//!   the sqlite file. **Revoking every remembered device = deleting that file and
//!   restarting the service.** An in-memory database (tests) gets a key that lives
//!   as long as the process.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use axum::http::{header::COOKIE, HeaderMap};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::{normalize_email, LOG_TARGET};

pub(crate) const DEVICE_COOKIE: &str = "presenter_device";
const VERSION: &str = "v1";
pub(crate) const MAX_AGE_SECS: i64 = 365 * 24 * 60 * 60;
/// A verified login re-issues a cookie older than this (keeps it a year ahead).
pub(crate) const REFRESH_AFTER_SECS: i64 = 30 * 24 * 60 * 60;
/// A clock step between issuing and reading tolerated into the future.
const FUTURE_SLACK_SECS: i64 = 5 * 60;
const KEY_LEN: usize = 32;

type HmacSha256 = Hmac<Sha256>;

/// A cookie that verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeviceClaims {
    pub(crate) email: String,
    pub(crate) issued_at: i64,
}

impl DeviceClaims {
    /// A verified login for `email` re-issues this cookie: another person, or old.
    pub(crate) fn needs_refresh(&self, email: &str, now: i64) -> bool {
        self.email != email || now - self.issued_at >= REFRESH_AFTER_SECS
    }
}

/// What the request's device cookie says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CookieState {
    /// No `presenter_device` cookie.
    Absent,
    Valid(DeviceClaims),
    /// A cookie that does not verify (another key, older than a year, altered).
    Refused,
}

/// The `Set-Cookie` value that removes the device cookie.
pub(crate) fn removal(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{DEVICE_COOKIE}=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax{secure}")
}

pub(crate) struct DeviceKey {
    /// The keyed HMAC, built once; cloned per use.
    mac: HmacSha256,
    #[cfg(test)]
    key: [u8; KEY_LEN],
}

impl DeviceKey {
    /// `Err` never happens for HMAC (any key length is valid), but it is not
    /// swallowed either.
    pub(crate) fn from_bytes(key: [u8; KEY_LEN]) -> Result<Self, hmac::digest::InvalidLength> {
        Ok(Self {
            mac: <HmacSha256 as Mac>::new_from_slice(&key)?,
            #[cfg(test)]
            key,
        })
    }

    /// The key at `location` (created when missing), or a process-lifetime key
    /// without a location. `None` only when no random bytes could be had.
    pub(crate) fn open(location: Option<&Path>) -> Option<Self> {
        match location {
            Some(path) => match load_or_create(path) {
                Ok(key) => Some(key),
                Err(err) => {
                    tracing::warn!(
                        target: LOG_TARGET,
                        path = %path.display(),
                        error = %err,
                        "device key unusable: remembered devices last until the next restart"
                    );
                    Self::ephemeral()
                }
            },
            None => {
                tracing::debug!(target: LOG_TARGET, "no database file: the device key is not stored");
                Self::ephemeral()
            }
        }
    }

    fn ephemeral() -> Option<Self> {
        match random_key().and_then(|key| Self::from_bytes(key).map_err(std::io::Error::other)) {
            Ok(key) => Some(key),
            Err(err) => {
                tracing::warn!(
                    target: LOG_TARGET,
                    error = %err,
                    "no random bytes for a device key: devices are not remembered"
                );
                None
            }
        }
    }

    /// The `Set-Cookie` value remembering `email` from `now`.
    pub(crate) fn issue(&self, email: &str, now: i64, secure: bool) -> String {
        let payload = URL_SAFE_NO_PAD.encode(format!("{VERSION}|{now}|{email}"));
        let signature = URL_SAFE_NO_PAD.encode(self.sign(payload.as_bytes()));
        let secure = if secure { "; Secure" } else { "" };
        format!(
            "{DEVICE_COOKIE}={payload}.{signature}; Max-Age={MAX_AGE_SECS}; Path=/; HttpOnly; SameSite=Lax{secure}"
        )
    }

    /// The first `presenter_device` cookie of the request that verifies and is
    /// not older than a year.
    pub(crate) fn read(&self, headers: &HeaderMap, now: i64) -> CookieState {
        let mut state = CookieState::Absent;
        for value in cookie_values(headers) {
            match self.verify(value, now) {
                Some(claims) => return CookieState::Valid(claims),
                None => state = CookieState::Refused,
            }
        }
        state
    }

    fn verify(&self, value: &str, now: i64) -> Option<DeviceClaims> {
        let (payload, signature) = value.split_once('.')?;
        let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
        let mut mac = self.mac();
        mac.update(payload.as_bytes());
        mac.verify_slice(&signature).ok()?;
        let decoded = String::from_utf8(URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
        let mut parts = decoded.splitn(3, '|');
        if parts.next()? != VERSION {
            return None;
        }
        let issued_at: i64 = parts.next()?.parse().ok()?;
        let email = normalize_email(parts.next()?)?;
        let age = now - issued_at;
        (-FUTURE_SLACK_SECS..=MAX_AGE_SECS)
            .contains(&age)
            .then_some(DeviceClaims { email, issued_at })
    }

    fn sign(&self, data: &[u8]) -> Vec<u8> {
        let mut mac = self.mac();
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    }

    fn mac(&self) -> HmacSha256 {
        self.mac.clone()
    }
}

/// Every `presenter_device=` value of the request's `Cookie` headers.
fn cookie_values(headers: &HeaderMap) -> impl Iterator<Item = &str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(name, _)| *name == DEVICE_COOKIE)
        .map(|(_, value)| value.trim())
}

fn load_or_create(path: &Path) -> std::io::Result<DeviceKey> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_key(&text).and_then(key_from_bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => create(path),
        Err(err) => Err(err),
    }
}

fn create(path: &Path) -> std::io::Result<DeviceKey> {
    let key = random_key()?;
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path);
    match file {
        Ok(mut file) => {
            file.write_all(hex.as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            tracing::info!(
                target: LOG_TARGET,
                path = %path.display(),
                "new device key created: no device was remembered before"
            );
            key_from_bytes(key)
        }
        // Another process created it first: use that one.
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            parse_key(&std::fs::read_to_string(path)?).and_then(key_from_bytes)
        }
        Err(err) => Err(err),
    }
}

fn key_from_bytes(key: [u8; KEY_LEN]) -> std::io::Result<DeviceKey> {
    DeviceKey::from_bytes(key).map_err(std::io::Error::other)
}

fn parse_key(text: &str) -> std::io::Result<[u8; KEY_LEN]> {
    let text = text.trim();
    let invalid = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not 64 hex characters");
    if text.len() != KEY_LEN * 2 || !text.is_ascii() {
        return Err(invalid());
    }
    let mut key = [0u8; KEY_LEN];
    for (slot, pair) in key.iter_mut().zip(text.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| invalid())?;
        *slot = u8::from_str_radix(pair, 16).map_err(|_| invalid())?;
    }
    Ok(key)
}

fn random_key() -> std::io::Result<[u8; KEY_LEN]> {
    let mut key = [0u8; KEY_LEN];
    getrandom::getrandom(&mut key).map_err(|err| std::io::Error::other(err.to_string()))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn key() -> DeviceKey {
        DeviceKey::from_bytes([7; KEY_LEN]).expect("key")
    }

    fn with_cookie(set_cookie: &str) -> HeaderMap {
        let pair = set_cookie.split(';').next().expect("pair");
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&format!("other=1; {pair}")).expect("cookie"),
        );
        headers
    }

    #[test]
    fn an_issued_cookie_reads_back_its_email_and_flags() {
        let cookie = key().issue("preacher@example.org", 1_000_000, true);
        assert!(cookie.starts_with("presenter_device="), "{cookie}");
        assert!(cookie.contains("; HttpOnly"), "{cookie}");
        assert!(cookie.contains("; SameSite=Lax"), "{cookie}");
        assert!(cookie.contains("; Path=/"), "{cookie}");
        assert!(cookie.contains("; Max-Age=31536000"), "{cookie}");
        assert!(cookie.ends_with("; Secure"), "{cookie}");
        assert!(!key().issue("a@b.c", 1, false).contains("Secure"));
        let claims = key().read(&with_cookie(&cookie), 1_000_000 + 60);
        assert_eq!(
            claims,
            CookieState::Valid(DeviceClaims {
                email: "preacher@example.org".into(),
                issued_at: 1_000_000
            })
        );
    }

    #[test]
    fn another_key_a_changed_payload_or_an_old_cookie_is_refused() {
        let cookie = key().issue("preacher@example.org", 1_000_000, false);
        let other = DeviceKey::from_bytes([8; KEY_LEN]).expect("key");
        assert_eq!(
            other.read(&with_cookie(&cookie), 1_000_000),
            CookieState::Refused
        );

        let value = cookie.split(';').next().expect("pair");
        let (_, signature) = value.split_once('.').expect("dot");
        let forged_payload = URL_SAFE_NO_PAD.encode("v1|1000000|boss@example.org");
        let forged = format!("{DEVICE_COOKIE}={forged_payload}.{signature}");
        assert_eq!(
            key().read(&with_cookie(&forged), 1_000_000),
            CookieState::Refused
        );

        assert_eq!(
            key().read(&with_cookie(&cookie), 1_000_000 + MAX_AGE_SECS + 1),
            CookieState::Refused
        );
        assert!(matches!(
            key().read(&with_cookie(&cookie), 1_000_000 + MAX_AGE_SECS),
            CookieState::Valid(_)
        ));
        assert_eq!(
            key().read(&with_cookie(&cookie), 1_000_000 - 3_600),
            CookieState::Refused
        );
        assert_eq!(
            key().read(&HeaderMap::new(), 1_000_000),
            CookieState::Absent
        );
        assert_eq!(
            key().read(&with_cookie("presenter_device=x.y"), 1),
            CookieState::Refused
        );
        let removal = removal(true);
        assert!(
            removal.starts_with("presenter_device=; Max-Age=0; Path=/; HttpOnly"),
            "{removal}"
        );
    }

    #[test]
    fn a_login_refreshes_a_cookie_of_another_person_or_after_thirty_days() {
        let claims = DeviceClaims {
            email: "preacher@example.org".into(),
            issued_at: 0,
        };
        assert!(!claims.needs_refresh("preacher@example.org", REFRESH_AFTER_SECS - 1));
        assert!(claims.needs_refresh("preacher@example.org", REFRESH_AFTER_SECS));
        assert!(claims.needs_refresh("other@example.org", 1));
    }

    #[test]
    fn the_key_file_is_created_once_0600_and_read_back() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("access-device.key");
        let first = load_or_create(&path).expect("create");
        let mode = std::fs::metadata(&path).expect("meta").permissions();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(mode.mode() & 0o777, 0o600);
        let second = load_or_create(&path).expect("read");
        assert_eq!(first.key, second.key);
        let cookie = first.issue("a@b.org", 10, false);
        assert!(matches!(
            second.read(&with_cookie(&cookie), 10),
            CookieState::Valid(_)
        ));

        std::fs::write(&path, "not a key").expect("write");
        assert!(load_or_create(&path).is_err());
        assert!(
            DeviceKey::open(Some(&path)).is_some(),
            "falls back to a process key"
        );
    }
}
