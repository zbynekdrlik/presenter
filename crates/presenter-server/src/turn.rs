//! Cloudflare Realtime TURN — server-side ICE-server minting (#502).
//!
//! WebRTC NDI preview was BLACK for any client that could not reach the
//! server's LAN host candidates directly: a remote operator, or an on-LAN
//! client whose packets to `10.77.x` are hijacked through a Tailscale subnet
//! route / DERP relay. With no STUN/TURN the stream had exactly one media path
//! (direct to `10.77.x`) and no fallback. The fix is a TURN relay candidate on
//! BOTH sides of the connection — the browser `RTCPeerConnection` AND the
//! server `webrtcbin` — so a fully-relayed path through Cloudflare exists when
//! the direct path is unreachable. The direct path still wins on the LAN
//! (`iceTransportPolicy` stays `all`), so there is no added latency where
//! direct works.
//!
//! Credentials are MINTED short-lived from Cloudflare server-side so the
//! long-lived TURN key never reaches the browser:
//!   POST {base}/{key_id}/credentials/generate-ice-servers
//!   Authorization: Bearer {api_token}   body: {"ttl": 86400}
//!   -> 201 { "iceServers": [ {urls:[stun…]}, {urls:[turn…], username, credential} ] }
//!
//! The minted `iceServers` array is handed verbatim to the browser via
//! `GET /ndi/ice-servers` (it is already RTCConfiguration-compatible), and a
//! single `turn://user:cred@host:port?transport=udp` URI is derived from the
//! same creds for the server `webrtcbin`'s `turn-server` property.
//!
//! Disabled gracefully: if either `PRESENTER_TURN_KEY_ID` or
//! `PRESENTER_TURN_KEY_API_TOKEN` is unset, the service mints nothing,
//! `ice_servers()` returns `[]`, `turn_uri()` returns `None`, and the on-LAN
//! direct path keeps working exactly as before. Secret values are NEVER logged.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Mutex;

/// Cloudflare Realtime TURN credentials endpoint base.
const CLOUDFLARE_TURN_BASE: &str = "https://rtc.live.cloudflare.com/v1/turn/keys";
/// Credential lifetime requested from Cloudflare (24h; max allowed is 48h).
const CRED_TTL_SECS: u64 = 86_400;
/// Re-mint once the cached creds are older than this (12h) — comfortably inside
/// the 24h TTL so a consumer never receives an already-expired credential.
const REFRESH_AFTER: Duration = Duration::from_secs(12 * 60 * 60);
/// After a FAILED mint, do not retry for this long — throttles a per-request
/// HTTP storm to Cloudflare when creds are misconfigured or Cloudflare is down.
const MIN_RETRY_AFTER: Duration = Duration::from_secs(60);
/// Bound on the Cloudflare mint HTTP call — caps how long the cache lock is held
/// when Cloudflare hangs (so concurrent WHEP POSTs can't wedge indefinitely).
const MINT_TIMEOUT: Duration = Duration::from_secs(10);

/// The long-lived Cloudflare TURN key (id + API token) read from env.
#[derive(Clone)]
struct TurnKey {
    key_id: String,
    api_token: String,
}

impl TurnKey {
    /// Read `PRESENTER_TURN_KEY_ID` + `PRESENTER_TURN_KEY_API_TOKEN`. Returns
    /// `None` (TURN disabled) if EITHER is unset or blank.
    fn from_env() -> Option<Self> {
        let key_id = std::env::var("PRESENTER_TURN_KEY_ID")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        let api_token = std::env::var("PRESENTER_TURN_KEY_API_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        Some(Self { key_id, api_token })
    }
}

/// Parsed result of one Cloudflare mint: the browser-ready `iceServers` array
/// plus the single `turn://…` URI for the server `webrtcbin`.
#[derive(Clone, Default)]
struct ParsedIce {
    /// The `iceServers` JSON array, RTCConfiguration-compatible (handed to the
    /// browser verbatim).
    ice_servers: Value,
    /// `turn://user:cred@host:port?transport=udp` for `webrtcbin.turn-server`.
    turn_uri: Option<String>,
}

struct CacheEntry {
    parsed: ParsedIce,
    minted_at: Instant,
}

/// Cache + retry-throttle state behind one lock. `entry` is the last
/// successfully-minted creds (kept across a later mint failure — stale-but-valid
/// beats none); `last_attempt` throttles re-minting after a failure so a
/// misconfigured key / Cloudflare outage can't trigger a per-request HTTP storm.
#[derive(Default)]
struct CacheState {
    entry: Option<CacheEntry>,
    last_attempt: Option<Instant>,
}

/// Shared, cloneable TURN service. Mints on demand and caches for ~12h.
#[derive(Clone)]
pub struct TurnService {
    inner: Arc<Inner>,
}

struct Inner {
    key: Option<TurnKey>,
    client: reqwest::Client,
    cache: Mutex<CacheState>,
    /// Base URL of the Cloudflare keys endpoint (overridable in tests).
    base_url: String,
}

impl TurnService {
    /// Build from the process environment. When the key vars are unset the
    /// service is permanently disabled (graceful no-op, on-LAN unaffected).
    pub fn from_env() -> Self {
        let key = TurnKey::from_env();
        if key.is_some() {
            tracing::info!("TURN: Cloudflare Realtime TURN enabled (creds minted on demand)");
        } else {
            tracing::info!(
                "TURN: disabled (PRESENTER_TURN_KEY_ID / PRESENTER_TURN_KEY_API_TOKEN unset) — \
                 WebRTC uses LAN host candidates only"
            );
        }
        // Bounded-timeout client so a hung Cloudflare endpoint can't pin the
        // cache lock (and thus block WHEP POSTs) indefinitely.
        let client = reqwest::Client::builder()
            .timeout(MINT_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            inner: Arc::new(Inner {
                key,
                client,
                cache: Mutex::new(CacheState::default()),
                base_url: CLOUDFLARE_TURN_BASE.to_string(),
            }),
        }
    }

    /// The browser-facing `iceServers` array. Returns `[]` when disabled or
    /// when minting has so far failed and nothing is cached.
    pub async fn ice_servers(&self) -> Value {
        match self.fresh_parsed().await {
            Some(parsed) => parsed.ice_servers,
            None => json!([]),
        }
    }

    /// The `turn://…` URI for the server `webrtcbin`. `None` when disabled or
    /// no usable TURN URL could be derived.
    pub async fn turn_uri(&self) -> Option<String> {
        self.fresh_parsed().await.and_then(|p| p.turn_uri)
    }

    /// Return a fresh (cache-or-mint) `ParsedIce`. Mints under the cache lock
    /// (so concurrent callers don't cause a mint storm), but bounded two ways:
    /// the `MINT_TIMEOUT` client caps how long the lock is held when Cloudflare
    /// hangs, and a failed mint is throttled to once per `MIN_RETRY_AFTER` so a
    /// misconfigured key / outage can't fire an HTTP request per WHEP connect.
    /// On mint failure: log + keep any existing cache (stale-but-valid beats
    /// none); if nothing is cached, returns `None` → callers degrade to LAN-only.
    async fn fresh_parsed(&self) -> Option<ParsedIce> {
        if self.inner.key.is_none() {
            return None;
        }
        let mut st = self.inner.cache.lock().await;
        let fresh = st
            .entry
            .as_ref()
            .is_some_and(|e| e.minted_at.elapsed() < REFRESH_AFTER);
        if !fresh {
            let may_retry = st
                .last_attempt
                .map_or(true, |t| t.elapsed() >= MIN_RETRY_AFTER);
            if may_retry {
                st.last_attempt = Some(Instant::now());
                match self.mint().await {
                    Ok(body) => {
                        st.entry = Some(CacheEntry {
                            parsed: parse_cloudflare_ice(&body),
                            minted_at: Instant::now(),
                        });
                    }
                    Err(e) => {
                        // Never log the token (api_token is not in the error) nor
                        // the key_id (mint() strips the URL from reqwest errors).
                        tracing::warn!(error = %e, "TURN: minting ICE servers failed — \
                            keeping any cached creds; retry throttled ~60s; WebRTC \
                            falls back to LAN-only meanwhile");
                    }
                }
            }
        }
        st.entry.as_ref().map(|e| e.parsed.clone())
    }

    /// POST to Cloudflare and return the raw JSON response body. Errors carry
    /// the HTTP status + truncated body for diagnostics — never the API token,
    /// and never the request URL (which embeds the secret `key_id`): a reqwest
    /// transport error's `Display` includes the URL, so `without_url()` strips it
    /// before the error can reach the WARN log.
    async fn mint(&self) -> anyhow::Result<Value> {
        let key = self
            .inner
            .key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("TURN disabled"))?;
        let url = format!(
            "{}/{}/credentials/generate-ice-servers",
            self.inner.base_url, key.key_id
        );
        let resp = self
            .inner
            .client
            .post(&url)
            .bearer_auth(&key.api_token)
            .json(&json!({ "ttl": CRED_TTL_SECS }))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Cloudflare TURN request failed: {}", e.without_url()))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let snippet: String = body.chars().take(300).collect();
            anyhow::bail!("Cloudflare TURN mint returned HTTP {status}: {snippet}");
        }
        let value: Value = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("Cloudflare TURN response not JSON: {e}"))?;
        let server_count = value
            .get("iceServers")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        tracing::info!(
            ice_server_count = server_count,
            "TURN: minted Cloudflare ICE servers"
        );
        Ok(value)
    }
}

/// Parse a Cloudflare `generate-ice-servers` response into the browser
/// `iceServers` array + the derived server `turn://` URI. Pure + tested.
fn parse_cloudflare_ice(body: &Value) -> ParsedIce {
    let ice_servers = body.get("iceServers").cloned().unwrap_or_else(|| json!([]));
    let servers = ice_servers.as_array();
    // Find the entry carrying TURN urls + username/credential.
    let turn_uri = servers.and_then(|arr| {
        arr.iter().find_map(|entry| {
            let username = entry.get("username").and_then(|v| v.as_str())?;
            let credential = entry.get("credential").and_then(|v| v.as_str())?;
            let urls = collect_urls(entry.get("urls"));
            build_turn_uri(&urls, username, credential)
        })
    });
    ParsedIce {
        ice_servers,
        turn_uri,
    }
}

/// Normalise an `urls` field (string or array of strings) into a `Vec<String>`.
fn collect_urls(urls: Option<&Value>) -> Vec<String> {
    match urls {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Build the `webrtcbin`-format TURN URI from Cloudflare's `turn:` URLs + creds.
///
/// `webrtcbin.turn-server` wants `turn(s)://username:password@host:port` with
/// the userinfo percent-encoded. Cloudflare's URLs are `turn:host:port?...`;
/// we prefer a UDP transport (lowest latency for the server's egress) and fall
/// back to any plain `turn:` URL. Returns `None` if no `turn:` URL is present.
fn build_turn_uri(turn_urls: &[String], username: &str, credential: &str) -> Option<String> {
    let chosen = turn_urls
        .iter()
        .find(|u| u.starts_with("turn:") && u.contains("transport=udp"))
        .or_else(|| turn_urls.iter().find(|u| u.starts_with("turn:")))?;
    // Strip the `turn:` scheme — keep `host:port?transport=udp`.
    let rest = chosen.strip_prefix("turn:")?;
    let user = percent_encode_userinfo(username);
    let cred = percent_encode_userinfo(credential);
    Some(format!("turn://{user}:{cred}@{rest}"))
}

/// Percent-encode a URI userinfo component (RFC 3986 unreserved chars pass
/// through; everything else is `%XX`). Cloudflare creds are hex, so this is
/// effectively a no-op for them, but it keeps the URI well-formed for any
/// credential containing `:`, `@`, `/`, `?`, etc.
fn percent_encode_userinfo(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A representative Cloudflare `generate-ice-servers` 201 body.
    fn sample_body() -> Value {
        json!({
            "iceServers": [
                { "urls": ["stun:stun.cloudflare.com:3478"] },
                {
                    "urls": [
                        "turn:turn.cloudflare.com:3478?transport=udp",
                        "turn:turn.cloudflare.com:3478?transport=tcp",
                        "turns:turn.cloudflare.com:5349?transport=tcp"
                    ],
                    "username": "aabbccdd11223344",
                    "credential": "deadbeefcafef00d"
                }
            ]
        })
    }

    #[test]
    fn build_turn_uri_prefers_udp_and_encodes_userinfo() {
        let urls = vec![
            "turn:turn.cloudflare.com:3478?transport=tcp".to_string(),
            "turn:turn.cloudflare.com:3478?transport=udp".to_string(),
            "turns:turn.cloudflare.com:5349?transport=tcp".to_string(),
        ];
        let uri = build_turn_uri(&urls, "user@id", "p:w/d").expect("turn uri");
        // UDP chosen; userinfo special chars percent-encoded.
        assert_eq!(
            uri,
            "turn://user%40id:p%3Aw%2Fd@turn.cloudflare.com:3478?transport=udp"
        );
    }

    #[test]
    fn build_turn_uri_hex_creds_unchanged() {
        let urls = vec!["turn:turn.cloudflare.com:3478?transport=udp".to_string()];
        let uri = build_turn_uri(&urls, "aabbccdd", "deadbeef").expect("turn uri");
        assert_eq!(
            uri,
            "turn://aabbccdd:deadbeef@turn.cloudflare.com:3478?transport=udp"
        );
    }

    #[test]
    fn build_turn_uri_falls_back_to_any_turn_url() {
        let urls = vec!["turn:relay.example.com:3478?transport=tcp".to_string()];
        let uri = build_turn_uri(&urls, "u", "c").expect("turn uri");
        assert_eq!(uri, "turn://u:c@relay.example.com:3478?transport=tcp");
    }

    #[test]
    fn build_turn_uri_none_when_no_turn_url() {
        // Only a STUN url present.
        let urls = vec!["stun:stun.example.com:3478".to_string()];
        assert!(build_turn_uri(&urls, "u", "c").is_none());
    }

    #[test]
    fn percent_encode_leaves_unreserved() {
        assert_eq!(percent_encode_userinfo("abcXYZ012-._~"), "abcXYZ012-._~");
    }

    #[test]
    fn parse_cloudflare_ice_extracts_servers_and_turn_uri() {
        let parsed = parse_cloudflare_ice(&sample_body());
        // The iceServers array is preserved verbatim for the browser.
        assert_eq!(parsed.ice_servers, sample_body()["iceServers"]);
        // The derived webrtcbin URI uses the UDP turn URL + the creds.
        assert_eq!(
            parsed.turn_uri.as_deref(),
            Some("turn://aabbccdd11223344:deadbeefcafef00d@turn.cloudflare.com:3478?transport=udp")
        );
    }

    #[test]
    fn parse_cloudflare_ice_missing_field_is_empty() {
        let parsed = parse_cloudflare_ice(&json!({}));
        assert_eq!(parsed.ice_servers, json!([]));
        assert!(parsed.turn_uri.is_none());
    }

    #[test]
    fn collect_urls_handles_string_and_array() {
        assert_eq!(
            collect_urls(Some(&json!("turn:h:3478?transport=udp"))),
            vec!["turn:h:3478?transport=udp".to_string()]
        );
        assert_eq!(
            collect_urls(Some(&json!(["a", "b"]))),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(collect_urls(None).is_empty());
        assert!(collect_urls(Some(&json!(5))).is_empty());
    }

    #[tokio::test]
    async fn disabled_service_returns_empty_and_no_uri() {
        // Force-disabled regardless of ambient env.
        let svc = TurnService {
            inner: Arc::new(Inner {
                key: None,
                client: reqwest::Client::new(),
                cache: Mutex::new(CacheState::default()),
                base_url: CLOUDFLARE_TURN_BASE.to_string(),
            }),
        };
        assert_eq!(svc.ice_servers().await, json!([]));
        assert!(svc.turn_uri().await.is_none());
    }

    #[tokio::test]
    async fn cached_service_returns_parsed_servers_without_network() {
        // Seed the cache as if a (mocked) Cloudflare mint already succeeded;
        // ice_servers()/turn_uri() must read it back without any HTTP call.
        let parsed = parse_cloudflare_ice(&sample_body());
        let svc = TurnService {
            inner: Arc::new(Inner {
                key: Some(TurnKey {
                    key_id: "test-key".to_string(),
                    api_token: "test-token".to_string(),
                }),
                client: reqwest::Client::new(),
                cache: Mutex::new(CacheState {
                    entry: Some(CacheEntry {
                        parsed,
                        minted_at: Instant::now(),
                    }),
                    last_attempt: None,
                }),
                base_url: "http://127.0.0.1:1/unused".to_string(),
            }),
        };
        assert_eq!(svc.ice_servers().await, sample_body()["iceServers"]);
        assert_eq!(
            svc.turn_uri().await.as_deref(),
            Some("turn://aabbccdd11223344:deadbeefcafef00d@turn.cloudflare.com:3478?transport=udp")
        );
    }
}
