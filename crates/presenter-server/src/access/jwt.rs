//! Verification of the Cloudflare Access token (`Cf-Access-Jwt-Assertion`,
//! #915), as Cloudflare documents it: RS256 signed with one of the team's keys
//! published at `https://<team>/cdn-cgi/access/certs`, `aud` = the Access
//! application's AUD tag, `iss` = `https://<team>`, `exp` / `nbf` in force. The
//! person is the token's `email` claim (a service token has none).
//!
//! The keys are cached by `kid`, and only ONE fetch runs at a time:
//! - a token whose key is cached never waits: a cache older than an hour is
//!   refreshed by a background task;
//! - a token whose key is NOT cached (first start, Cloudflare rotated its key)
//!   waits for the fetch in flight, or starts one, so a first login is never
//!   mistaken for nobody while the keys load. A fetch that brings no key for the
//!   `kid` is not repeated within a minute.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use super::{normalize_email, LOG_TARGET};

/// Keys older than this are fetched again (Cloudflare rotates them every six
/// weeks and publishes the next key ahead of time).
const KEYS_TTL: Duration = Duration::from_secs(60 * 60);
/// No two fetches closer than this, whatever the tokens ask for.
const MIN_REFETCH: Duration = Duration::from_secs(60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// When the cached keys are fetched again.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeyTiming {
    /// Keys older than this are refreshed in the background.
    pub(crate) ttl: Duration,
    /// No two fetches closer than this.
    pub(crate) min_refetch: Duration,
}

impl Default for KeyTiming {
    fn default() -> Self {
        Self {
            ttl: KEYS_TTL,
            min_refetch: MIN_REFETCH,
        }
    }
}

/// One site's Access application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JwtConfig {
    team_domain: String,
    audiences: Vec<String>,
    certs_url: String,
}

impl JwtConfig {
    /// `team` = the team domain (`newlevelchurch.cloudflareaccess.com`), `aud` =
    /// one or more AUD tags separated by commas, `certs_url` = an override of the
    /// key URL (tests). `Err` names what is missing or malformed.
    pub(crate) fn parse(
        team: Option<&str>,
        aud: Option<&str>,
        certs_url: Option<&str>,
    ) -> Result<Self, &'static str> {
        let team = team.map(str::trim).filter(|team| !team.is_empty());
        let team = team.ok_or("no team domain")?.to_ascii_lowercase();
        let team_ok = team
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            && team.contains('.');
        if !team_ok {
            return Err("the team domain is no host name");
        }
        let audiences: Vec<String> = aud
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_string)
            .collect();
        if audiences.is_empty() {
            return Err("no application AUD");
        }
        if audiences
            .iter()
            .any(|tag| !tag.chars().all(|c| c.is_ascii_alphanumeric()))
        {
            return Err("an application AUD is not alphanumeric");
        }
        let certs_url = certs_url
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://{team}/cdn-cgi/access/certs"));
        Ok(Self {
            team_domain: team,
            audiences,
            certs_url,
        })
    }

    pub(crate) fn team_domain(&self) -> &str {
        &self.team_domain
    }

    pub(crate) fn audiences(&self) -> &[String] {
        &self.audiences
    }

    fn issuer(&self) -> String {
        format!("https://{}", self.team_domain)
    }
}

/// Why a token was refused (logged at WARN, never answered to the client).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum TokenRefusal {
    #[error("the token has no readable header")]
    Malformed,
    #[error("the token is not RS256 with a key id")]
    NotRs256,
    #[error("no published key has the token's key id")]
    UnknownKey,
    #[error("the published keys could not be read")]
    KeysUnavailable,
    #[error("the token did not verify: {0}")]
    Invalid(String),
    #[error("the token names no e-mail")]
    NoEmail,
}

#[derive(Deserialize)]
struct Claims {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Default)]
struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
    /// When the last fetch ENDED (success or failure); a cancelled fetch
    /// leaves it untouched, so the next request simply fetches.
    last_attempt: Option<Instant>,
}

enum KeySource {
    Remote {
        client: reqwest::Client,
        url: String,
    },
    /// Tests: the keys given at construction, never fetched.
    #[cfg(test)]
    Fixed,
}

pub(crate) struct JwtVerifier {
    validation: Validation,
    source: KeySource,
    timing: KeyTiming,
    cache: Mutex<KeyCache>,
    /// Held for the whole of a fetch: one fetch at a time.
    fetching: Arc<tokio::sync::Mutex<()>>,
}

impl JwtVerifier {
    /// `Err` when no HTTP client could be built (no TLS backend).
    pub(crate) fn new(config: JwtConfig, timing: KeyTiming) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent(concat!("presenter/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let url = config.certs_url.clone();
        Ok(Self::with_source(
            config,
            KeySource::Remote { client, url },
            timing,
        ))
    }

    /// A verifier that trusts exactly `keys` (tests).
    #[cfg(test)]
    pub(crate) fn with_keys(config: JwtConfig, keys: &JwkSet) -> Self {
        let verifier = Self::with_source(config, KeySource::Fixed, KeyTiming::default());
        verifier.store(keys);
        verifier
    }

    fn with_source(config: JwtConfig, source: KeySource, timing: KeyTiming) -> Self {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&config.audiences);
        validation.set_issuer(&[config.issuer()]);
        validation.set_required_spec_claims(&["exp", "aud", "iss"]);
        validation.validate_nbf = true;
        Self {
            validation,
            source,
            timing,
            cache: Mutex::new(KeyCache::default()),
            fetching: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// The verified, normalised e-mail of `token`.
    pub(crate) async fn verify(self: &Arc<Self>, token: &str) -> Result<String, TokenRefusal> {
        let header = decode_header(token).map_err(|_| TokenRefusal::Malformed)?;
        let kid = match (header.alg, header.kid) {
            (Algorithm::RS256, Some(kid)) => kid,
            _ => return Err(TokenRefusal::NotRs256),
        };
        let key = self.key_for(&kid).await?;
        let data = decode::<Claims>(token, &key, &self.validation)
            .map_err(|err| TokenRefusal::Invalid(format!("{:?}", err.kind())))?;
        data.claims
            .email
            .as_deref()
            .and_then(normalize_email)
            .ok_or(TokenRefusal::NoEmail)
    }

    async fn key_for(self: &Arc<Self>, kid: &str) -> Result<DecodingKey, TokenRefusal> {
        let (cached, stale) = {
            let cache = self.lock_cache();
            let stale = cache
                .fetched_at
                .is_none_or(|at| at.elapsed() >= self.timing.ttl);
            (cache.keys.get(kid).cloned(), stale)
        };
        if let Some(key) = cached {
            if stale {
                self.refresh_in_background();
            }
            return Ok(key);
        }
        // Not cached: wait for a fetch in flight (it may bring the key), then
        // fetch ourselves unless one just ended.
        let _turn = self.fetching.lock().await;
        if let Ok(key) = self.cached_key(kid) {
            return Ok(key);
        }
        if self.may_fetch() {
            self.fetch().await;
        }
        self.cached_key(kid)
    }

    /// Refresh stale keys without making the request wait. Nothing when a
    /// fetch is running or one ended less than `min_refetch` ago.
    fn refresh_in_background(self: &Arc<Self>) {
        let Ok(turn) = Arc::clone(&self.fetching).try_lock_owned() else {
            return;
        };
        if !self.may_fetch() {
            return;
        }
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let _turn = turn;
            me.fetch().await;
        });
    }

    fn may_fetch(&self) -> bool {
        self.lock_cache()
            .last_attempt
            .is_none_or(|at| at.elapsed() >= self.timing.min_refetch)
    }

    fn cached_key(&self, kid: &str) -> Result<DecodingKey, TokenRefusal> {
        let cache = self.lock_cache();
        match cache.keys.get(kid) {
            Some(key) => Ok(key.clone()),
            None if cache.keys.is_empty() => Err(TokenRefusal::KeysUnavailable),
            None => Err(TokenRefusal::UnknownKey),
        }
    }

    /// One fetch; the caller holds `fetching`.
    async fn fetch(&self) {
        let (client, url) = match &self.source {
            KeySource::Remote { client, url } => (client, url),
            #[cfg(test)]
            KeySource::Fixed => return,
        };
        let started = Instant::now();
        let keys = async {
            client
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .json::<JwkSet>()
                .await
        }
        .await;
        match keys {
            Ok(keys) => {
                let stored = self.store(&keys);
                tracing::info!(
                    target: LOG_TARGET,
                    url = %url,
                    keys = stored,
                    ms = started.elapsed().as_millis() as u64,
                    "Access signing keys fetched"
                );
            }
            Err(err) => tracing::warn!(
                target: LOG_TARGET,
                url = %url,
                error = %err,
                kept = self.lock_cache().keys.len(),
                "Access signing keys could not be fetched; the cached ones stay"
            ),
        }
        self.lock_cache().last_attempt = Some(Instant::now());
    }

    /// Replace the cache with the RSA keys of `keys`; the number stored.
    fn store(&self, keys: &JwkSet) -> usize {
        let parsed: HashMap<String, DecodingKey> = keys
            .keys
            .iter()
            .filter_map(|jwk| {
                let kid = jwk.common.key_id.clone()?;
                match DecodingKey::from_jwk(jwk) {
                    Ok(key) => Some((kid, key)),
                    Err(err) => {
                        tracing::warn!(target: LOG_TARGET, kid = %kid, error = %err, "Access key skipped");
                        None
                    }
                }
            })
            .collect();
        let count = parsed.len();
        if count > 0 {
            let mut cache = self.lock_cache();
            cache.keys = parsed;
            cache.fetched_at = Some(Instant::now());
        }
        count
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, KeyCache> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
