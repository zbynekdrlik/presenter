//! Verification of the Cloudflare Access token (`Cf-Access-Jwt-Assertion`,
//! #915), as Cloudflare documents it: RS256 signed with one of the team's keys
//! published at `https://<team>/cdn-cgi/access/certs`, `aud` = the Access
//! application's AUD tag, `iss` = `https://<team>`, `exp` / `nbf` in force. The
//! person is the token's `email` claim (a service token has none).
//!
//! The keys are cached by `kid`. An unknown `kid` or a cache older than an hour
//! fetches them again, but at most once a minute, and only ONE request fetches:
//! the others go on with what is cached (a request never waits on a second fetch
//! while Cloudflare is slow; it is identified by its device cookie or not at all).

use std::collections::HashMap;
use std::sync::Mutex;
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
    cache: Mutex<KeyCache>,
    fetching: tokio::sync::Mutex<()>,
}

impl JwtVerifier {
    pub(crate) fn new(config: JwtConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent(concat!("presenter/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        let url = config.certs_url.clone();
        Self::with_source(config, KeySource::Remote { client, url })
    }

    /// A verifier that trusts exactly `keys` (tests).
    #[cfg(test)]
    pub(crate) fn with_keys(config: JwtConfig, keys: &JwkSet) -> Self {
        let verifier = Self::with_source(config, KeySource::Fixed);
        verifier.store(keys);
        verifier
    }

    fn with_source(config: JwtConfig, source: KeySource) -> Self {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&config.audiences);
        validation.set_issuer(&[config.issuer()]);
        validation.set_required_spec_claims(&["exp", "aud", "iss"]);
        validation.validate_nbf = true;
        Self {
            validation,
            source,
            cache: Mutex::new(KeyCache::default()),
            fetching: tokio::sync::Mutex::new(()),
        }
    }

    /// The verified, normalised e-mail of `token`.
    pub(crate) async fn verify(&self, token: &str) -> Result<String, TokenRefusal> {
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

    async fn key_for(&self, kid: &str) -> Result<DecodingKey, TokenRefusal> {
        let (cached, wants_fetch) = {
            let cache = self.lock_cache();
            let now = Instant::now();
            let fresh = cache
                .fetched_at
                .is_some_and(|at| now.duration_since(at) < KEYS_TTL);
            let cached = cache.keys.get(kid).cloned();
            let may_fetch = cache
                .last_attempt
                .is_none_or(|at| now.duration_since(at) >= MIN_REFETCH);
            (cached.clone(), may_fetch && !(fresh && cached.is_some()))
        };
        if wants_fetch {
            if let Ok(_turn) = self.fetching.try_lock() {
                self.fetch().await;
                return self.cached_key(kid);
            }
        }
        match cached {
            Some(key) => Ok(key),
            None => self.cached_key(kid),
        }
    }

    fn cached_key(&self, kid: &str) -> Result<DecodingKey, TokenRefusal> {
        let cache = self.lock_cache();
        match cache.keys.get(kid) {
            Some(key) => Ok(key.clone()),
            None if cache.keys.is_empty() => Err(TokenRefusal::KeysUnavailable),
            None => Err(TokenRefusal::UnknownKey),
        }
    }

    async fn fetch(&self) {
        self.lock_cache().last_attempt = Some(Instant::now());
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
