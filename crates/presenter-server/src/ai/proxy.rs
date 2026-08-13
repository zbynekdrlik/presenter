//! Manages CLIProxyAPI as a child process within Presenter.
//!
//! CLIProxyAPI is a Go binary that provides an OpenAI-compatible API by
//! authenticating with Claude via OAuth. Presenter bundles it, manages
//! its lifecycle, and handles Claude OAuth login via CLIProxyAPI's
//! native `-claude-login` flow with callback URL forwarding.

use crate::ai::proxy_output_relay::{redact_proxy_output_line, spawn_proxy_output_relay};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, ChildStdout, Command};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Last-reported overall Claude auth state, process-wide (#622 post-merge
/// review finding 3b): `None` = never reported yet, `Some(true)` = last
/// scan was authenticated, `Some(false)` = last scan was NOT authenticated.
/// Used so the "token is EXPIRED" log only WARNs on the TRANSITION into
/// not-authenticated — a steady-state dead login, polled every 5s by the
/// status chip, used to re-warn on every single scan (~12x/minute for the
/// exact same already-known problem). One process runs one `ProxyManager`
/// in practice, so a process-global is the right scope here.
static LAST_REPORTED_AUTH: Mutex<Option<bool>> = Mutex::new(None);

/// Holds a login child process and its stdout handle.
/// The stdout must be kept alive to prevent SIGPIPE from killing the process.
struct LoginProcess {
    child: Child,
    _stdout: ChildStdout,
}

/// Default port for the embedded CLIProxyAPI instance.
const DEFAULT_PROXY_PORT: u16 = 18787;

/// OAuth callback port used by CLIProxyAPI for Claude login.
const OAUTH_CALLBACK_PORT: u16 = 54545;

/// Name of the CLIProxyAPI binary.
const PROXY_BINARY_NAME: &str = "cli-proxy-api";

/// Freshness classification of an on-disk Claude OAuth token (#438).
enum TokenValidity {
    /// Token's `expired` timestamp is in the future — usable. Carries the raw
    /// RFC3339 string so #599 can surface it to the UI.
    Fresh { expired: String },
    /// Token's `expired` timestamp is in the past — dead, re-login required.
    Expired { expired: String },
    /// File unreadable or has no parseable `expired` field — fail-open as valid.
    Unknown,
}

/// Result of scanning on-disk Claude auth state (#599): whether Presenter
/// treats the account as authenticated, and — when derivable — the RFC3339
/// expiry of the token that "wins" the scan. Surfaced in the UI so an
/// operator can renew a login BEFORE it dies mid-event instead of only
/// discovering it dead when an AI request silently fails (2026-07-26).
struct AuthScan {
    authenticated: bool,
    /// The latest-expiring FRESH token's expiry when any token is fresh;
    /// otherwise the most-recently-expired token's expiry (so the UI can say
    /// "expired at X"); `None` for API-key auth, no tokens on disk, or tokens
    /// with no parseable expiry at all.
    expires_at: Option<String>,
}

/// Accumulated state from walking the auth directory's `claude-*` token
/// files: whether any token is fresh, the latest-expiring fresh/expired
/// timestamps seen so far, and every expired token (for the post-scan log).
struct AuthDirScan {
    authenticated: bool,
    fresh_max: Option<(chrono::DateTime<chrono::FixedOffset>, String)>,
    expired_max: Option<(chrono::DateTime<chrono::FixedOffset>, String)>,
    expired_tokens: Vec<(String, String)>,
}

/// State of the managed CLIProxyAPI process.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    pub running: bool,
    pub port: u16,
    pub api_url: String,
    pub binary_found: bool,
    pub claude_authenticated: bool,
    /// RFC3339 expiry of the token backing `claude_authenticated`, when
    /// derivable from on-disk state (#599). `None` for API-key auth, no
    /// tokens on disk, or tokens with no parseable expiry.
    pub token_expires_at: Option<String>,
}

/// Configuration for the embedded proxy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    #[serde(default = "default_proxy_port")]
    pub port: u16,
    #[serde(default = "default_true")]
    pub auto_start: bool,
}

fn default_proxy_port() -> u16 {
    DEFAULT_PROXY_PORT
}

fn default_true() -> bool {
    true
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PROXY_PORT,
            auto_start: true,
        }
    }
}

/// Manages the CLIProxyAPI child process and Claude OAuth.
pub struct ProxyManager {
    child: Arc<RwLock<Option<Child>>>,
    login_child: Arc<RwLock<Option<LoginProcess>>>,
    config: Arc<RwLock<ProxyConfig>>,
    deploy_dir: PathBuf,
}

impl ProxyManager {
    pub fn new(deploy_dir: PathBuf) -> Self {
        Self {
            child: Arc::new(RwLock::new(None)),
            login_child: Arc::new(RwLock::new(None)),
            config: Arc::new(RwLock::new(ProxyConfig::default())),
            deploy_dir,
        }
    }

    async fn binary_path(&self) -> Option<PathBuf> {
        let deploy_path = self.deploy_dir.join(PROXY_BINARY_NAME);
        if deploy_path.exists() {
            return Some(deploy_path);
        }

        let cwd_path = PathBuf::from(PROXY_BINARY_NAME);
        if cwd_path.exists() {
            return Some(cwd_path);
        }

        // `Command` is `tokio::process::Command` (see imports); `.output().await`
        // offloads the `which` lookup instead of blocking the runtime thread.
        if let Ok(output) = Command::new("which").arg(PROXY_BINARY_NAME).output().await {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }

        None
    }

    fn auth_dir(&self) -> PathBuf {
        self.deploy_dir.join(".cli-proxy-api")
    }

    fn config_path(&self) -> PathBuf {
        self.deploy_dir.join("cli-proxy-api-config.yaml")
    }

    /// Check if Claude credentials exist AND are still valid, and — when
    /// derivable (#599) — the expiry of the token backing that verdict.
    ///
    /// An explicit `claude-api-key` in the config counts as authenticated.
    /// Otherwise, OAuth token files (`claude-*.json`) are validated: a token
    /// whose `expired` timestamp is in the past does NOT count (it can no
    /// longer mint completions — #438). At least one fresh token → authenticated.
    ///
    /// This is an offline freshness check only (no live network probe per the
    /// #438 MVP scope). A token file with no parseable `expired` field is
    /// treated as valid (fail-open) so we never regress a working install on a
    /// format we don't recognise — only a *provably* expired token is rejected.
    ///
    /// Single pass so `status()` never has to read the same token files twice.
    async fn scan_claude_auth(&self) -> AuthScan {
        if let Ok(content) = tokio::fs::read_to_string(self.config_path()).await {
            if content.contains("claude-api-key:") {
                return AuthScan {
                    authenticated: true,
                    expires_at: None,
                };
            }
        }

        let scan = Self::scan_auth_dir(&self.auth_dir()).await;

        // #622 post-merge review finding 2: gate the fallback on the
        // AGGREGATE `authenticated` verdict, not merely on whether a fresh
        // token happens to exist. Before this, `fresh_max.or_else(expired_max)`
        // let an EXPIRED token's past timestamp leak through as "validity"
        // whenever `authenticated` was true ONLY via a fail-open `Unknown`
        // token (no fresh token at all) — the UI would show "Prihlásenie
        // platí do <a date in the past>". Authenticated must only ever
        // surface a FRESH timestamp (`None` when no fresh token backs it);
        // not-authenticated keeps showing the newest expired timestamp so
        // the "vypršalo X" banner subtext still works.
        let expires_at = if scan.authenticated {
            scan.fresh_max.map(|(_, s)| s)
        } else {
            scan.expired_max.map(|(_, s)| s)
        };

        Self::report_auth_transition(scan.authenticated, &scan.expired_tokens);

        AuthScan {
            authenticated: scan.authenticated,
            expires_at,
        }
    }

    /// Walk `auth_dir`, classifying every `claude-*` token file, and
    /// aggregate the authenticated verdict plus the fresh/expired timestamps
    /// and expired-token list `scan_claude_auth` needs afterward. A
    /// non-existent (or unreadable) `auth_dir` yields the same all-`None`,
    /// not-authenticated result the caller previously got from its own early
    /// return.
    async fn scan_auth_dir(auth_dir: &Path) -> AuthDirScan {
        let mut scan = AuthDirScan {
            authenticated: false,
            fresh_max: None,
            expired_max: None,
            // Collected, not logged immediately — the WARN-vs-DEBUG decision
            // (finding 3b) can only be made once the aggregate `authenticated`
            // verdict is known, after the loop.
            expired_tokens: Vec::new(),
        };

        let Ok(mut entries) = tokio::fs::read_dir(auth_dir).await else {
            return scan;
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.contains("claude") {
                continue;
            }
            // Only regular files are token files. A directory whose name happens
            // to contain "claude" (e.g. a cache/log subdir) must NOT be read as
            // a token: read_to_string would EISDIR → Unknown → fail-open, which
            // would mask a genuinely-expired-token state and defeat #438.
            match entry.file_type().await {
                Ok(ft) if ft.is_file() => {}
                Ok(_) => continue,
                Err(e) => {
                    warn!(entry = %name, ?e, "could not stat auth-dir entry; skipping");
                    continue;
                }
            }
            match Self::token_validity(&entry.path()).await {
                TokenValidity::Fresh { expired } => {
                    scan.authenticated = true;
                    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&expired) {
                        let is_newer = scan
                            .fresh_max
                            .as_ref()
                            .map_or(true, |(cur, _)| parsed > *cur);
                        if is_newer {
                            scan.fresh_max = Some((parsed, expired));
                        }
                    }
                }
                TokenValidity::Unknown => {
                    // Unparseable expiry — fail-open, this token counts as valid.
                    warn!(token = %name, "Claude token has no parseable expiry; treating as valid");
                    scan.authenticated = true;
                }
                TokenValidity::Expired { expired } => {
                    // AI auth is dead unless a fresh (or unknown, fail-open)
                    // token is found elsewhere in the scan. Logged after the
                    // loop, once, at WARN or DEBUG depending on whether this
                    // is a new transition (finding 3b).
                    scan.expired_tokens.push((name.clone(), expired.clone()));
                    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&expired) {
                        let is_newer = scan
                            .expired_max
                            .as_ref()
                            .map_or(true, |(cur, _)| parsed > *cur);
                        if is_newer {
                            scan.expired_max = Some((parsed, expired));
                        }
                    }
                }
            }
        }

        scan
    }

    /// #622 post-merge review finding 3b: WARN only on the TRANSITION into
    /// not-authenticated (this scan found expired token(s) and last time we
    /// were authenticated, or this is the very first scan); repeat scans of
    /// an already-known-dead login log at DEBUG instead. A recovered login
    /// resets the tracked state so the NEXT time it dies we warn again
    /// rather than staying silent forever.
    fn report_auth_transition(authenticated: bool, expired_tokens: &[(String, String)]) {
        if authenticated {
            if let Ok(mut last) = LAST_REPORTED_AUTH.lock() {
                *last = Some(true);
            }
        } else if !expired_tokens.is_empty() {
            let transitioned = match LAST_REPORTED_AUTH.lock() {
                Ok(mut last) => {
                    let changed = *last != Some(false);
                    *last = Some(false);
                    changed
                }
                Err(_) => true,
            };
            for (name, expired) in expired_tokens {
                if transitioned {
                    warn!(
                        token = %name,
                        expired = %expired,
                        "Claude OAuth token is EXPIRED — reporting not authenticated; re-login required"
                    );
                } else {
                    debug!(
                        token = %name,
                        expired = %expired,
                        "Claude OAuth token still expired — not authenticated (already reported)"
                    );
                }
            }
        }
    }

    /// Inspect a `claude-*.json` OAuth token file and classify its freshness
    /// from the on-disk `expired` RFC3339 timestamp.
    async fn token_validity(path: &Path) -> TokenValidity {
        let Ok(content) = tokio::fs::read_to_string(path).await else {
            return TokenValidity::Unknown;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
            return TokenValidity::Unknown;
        };
        let Some(expired_str) = json.get("expired").and_then(|v| v.as_str()) else {
            return TokenValidity::Unknown;
        };
        match chrono::DateTime::parse_from_rfc3339(expired_str) {
            Ok(expired_at) => {
                if expired_at <= chrono::Utc::now() {
                    TokenValidity::Expired {
                        expired: expired_str.to_string(),
                    }
                } else {
                    TokenValidity::Fresh {
                        expired: expired_str.to_string(),
                    }
                }
            }
            Err(_) => TokenValidity::Unknown,
        }
    }

    /// Write the config YAML file for CLIProxyAPI.
    async fn write_config_with_key(&self, api_key: Option<&str>) -> anyhow::Result<()> {
        let config = self.config.read().await;
        let auth_dir = self.auth_dir();

        let key_section = if let Some(key) = api_key {
            format!(
                r#"
claude-api-key:
  - api-key: "{key}"
"#
            )
        } else {
            String::new()
        };

        let config_content = format!(
            r#"# Auto-generated by Presenter - do not edit
host: "127.0.0.1"
port: {port}
auth-dir: "{auth_dir}"
debug: false
logging-to-file: false
request-retry: 2
{key_section}"#,
            port = config.port,
            auth_dir = auth_dir.display(),
        );

        tokio::fs::create_dir_all(&auth_dir).await?;
        tokio::fs::write(self.config_path(), config_content).await?;
        Ok(())
    }

    /// Start the CLIProxyAPI process.
    pub async fn start(&self) -> anyhow::Result<()> {
        {
            let guard = self.child.read().await;
            if guard.is_some() {
                return Ok(());
            }
        }

        let binary = self
            .binary_path()
            .await
            .ok_or_else(|| anyhow::anyhow!("CLIProxyAPI binary not found"))?;

        let existing_key = self.read_existing_api_key().await;
        self.write_config_with_key(existing_key.as_deref()).await?;

        let config = self.config.read().await;
        info!(binary = %binary.display(), port = config.port, "starting CLIProxyAPI");

        let mut child = Command::new(&binary)
            .arg("-config")
            .arg(self.config_path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        // #675-adjacent observability gap: relay the child's own
        // stdout/stderr into Presenter's tracing output (redacted — see
        // `spawn_proxy_output_relay`'s doc comment) instead of discarding
        // it, so a failed OAuth refresh leaves an actual WHY in the
        // journal, not just Presenter's own periodic WHETHER
        // (`scan_claude_auth`). Each stream is taken and handed to its own
        // relay task; a `None` here (already-taken, or `Stdio::piped()`
        // failed to give a handle) is simply skipped rather than treated
        // as fatal — losing the diagnostic relay must never prevent the
        // proxy itself from starting.
        if let Some(stdout) = child.stdout.take() {
            spawn_proxy_output_relay(stdout, "stdout");
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_proxy_output_relay(stderr, "stderr");
        }

        {
            let mut guard = self.child.write().await;
            *guard = Some(child);
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        let port = config.port;
        drop(config);
        let url = format!("http://127.0.0.1:{port}/v1/models");
        match reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                info!(port, "CLIProxyAPI started successfully");
            }
            Ok(resp) => {
                warn!(port, status = %resp.status(), "CLIProxyAPI responded with non-success");
            }
            Err(e) => {
                warn!(?e, "CLIProxyAPI may not have started correctly");
            }
        }
        Ok(())
    }

    /// Stop the CLIProxyAPI process.
    pub async fn stop(&self) -> anyhow::Result<()> {
        let mut guard = self.child.write().await;
        if let Some(mut child) = guard.take() {
            info!("stopping CLIProxyAPI");
            child.kill().await.ok();
            child.wait().await.ok();
        }
        Ok(())
    }

    /// The bundled proxy's CONFIGURED port — always available (static
    /// config, defaulting to [`DEFAULT_PROXY_PORT`]) regardless of whether
    /// the process is currently running. Deliberately cheap (one `RwLock`
    /// read, no `is_running()`/`binary_path()` filesystem work): used by
    /// `router::ai::is_bundled_proxy_address` to classify a stored `api_url`
    /// as the bundled proxy's own address WITHOUT paying for a full
    /// [`status()`](Self::status) call just to read one field (#683).
    pub async fn configured_port(&self) -> u16 {
        self.config.read().await.port
    }

    /// Get current status.
    pub async fn status(&self) -> ProxyStatus {
        let config = self.config.read().await;
        let port = config.port;
        drop(config);

        let auth_scan = self.scan_claude_auth().await;

        ProxyStatus {
            running: self.is_running().await,
            port,
            api_url: format!("http://127.0.0.1:{port}/v1"),
            binary_found: self.binary_path().await.is_some(),
            claude_authenticated: auth_scan.authenticated,
            token_expires_at: auth_scan.expires_at,
        }
    }

    async fn is_running(&self) -> bool {
        let mut guard = self.child.write().await;
        if let Some(ref mut child) = *guard {
            match child.try_wait() {
                Ok(None) => true,
                _ => {
                    *guard = None;
                    false
                }
            }
        } else {
            false
        }
    }

    // ── Claude Login via CLIProxyAPI ──

    /// Start the Claude OAuth login flow.
    ///
    /// Spawns `cli-proxy-api -claude-login -no-browser` and reads the auth URL
    /// from stdout. The login process listens on port 54545 for the OAuth
    /// callback. After the user authorizes, the browser redirects to
    /// `localhost:54545/callback?code=...` which fails (since the server is
    /// remote). The user copies the full URL from the browser error page and
    /// pastes it into Presenter, which forwards it via `complete_login()`.
    pub async fn claude_login(&self) -> anyhow::Result<String> {
        // Kill any previous login process
        {
            let mut guard = self.login_child.write().await;
            if let Some(mut proc) = guard.take() {
                proc.child.kill().await.ok();
                proc.child.wait().await.ok();
            }
        }

        let binary = self
            .binary_path()
            .await
            .ok_or_else(|| anyhow::anyhow!("CLIProxyAPI binary not found"))?;

        let existing_key = self.read_existing_api_key().await;
        self.write_config_with_key(existing_key.as_deref()).await?;

        info!(binary = %binary.display(), "starting CLIProxyAPI claude-login");

        let mut child = Command::new(&binary)
            .arg("-claude-login")
            .arg("-no-browser")
            .arg("-config")
            .arg(self.config_path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to capture login process stdout"))?;

        let mut reader = tokio::io::BufReader::new(stdout);
        let mut auth_url: Option<String> = None;
        let mut line_buf = String::new();

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            line_buf.clear();
            match tokio::time::timeout(remaining, reader.read_line(&mut line_buf)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(_)) => {
                    let line = line_buf.trim_end();
                    // Same credential-path caution as `start()`'s relay
                    // (see `redact_proxy_output_line`'s doc comment) — the
                    // URL-extraction below still reads the UNREDACTED
                    // `line`, only the logged copy is filtered.
                    info!(line = %redact_proxy_output_line(line), "claude-login stdout");
                    if let Some(url_start) = line.find("https://") {
                        let url = line[url_start..].split_whitespace().next().unwrap_or("");
                        if !url.is_empty() {
                            auth_url = Some(url.to_string());
                            break;
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!(?e, "error reading claude-login stdout");
                    break;
                }
                Err(_) => break,
            }
        }

        let url = auth_url.ok_or_else(|| {
            anyhow::anyhow!("claude-login did not produce an auth URL within 10s")
        })?;

        // Keep both child AND stdout handle alive — dropping stdout closes the
        // pipe and kills the process via SIGPIPE.
        let stdout_handle = reader.into_inner();
        {
            let mut guard = self.login_child.write().await;
            *guard = Some(LoginProcess {
                child,
                _stdout: stdout_handle,
            });
        }

        info!("claude-login URL obtained, waiting for user to paste callback URL");
        Ok(url)
    }

    /// Complete the OAuth login by forwarding the callback URL to CLIProxyAPI.
    ///
    /// After the user authorizes, the browser redirects to
    /// `localhost:54545/callback?code=XXX&state=YYY`. Since the server is
    /// remote, this fails in the browser. The user copies the full URL and
    /// pastes it here. Presenter forwards it to CLIProxyAPI's callback
    /// endpoint on localhost.
    pub async fn complete_login(&self, callback_url: &str) -> anyhow::Result<()> {
        let query = if let Some(pos) = callback_url.find('?') {
            &callback_url[pos..]
        } else {
            anyhow::bail!("callback URL must contain query parameters (?code=...&state=...)");
        };

        let target = format!("http://127.0.0.1:{OAUTH_CALLBACK_PORT}/callback{query}");
        info!("forwarding OAuth callback to CLIProxyAPI");

        // Don't follow redirects — CLIProxyAPI returns 302 on success
        let resp = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?
            .get(&target)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OAuth callback failed (HTTP {status}): {body}");
        }

        // Give CLIProxyAPI a moment to save tokens
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Kill the login process — it's done
        {
            let mut guard = self.login_child.write().await;
            if let Some(mut proc) = guard.take() {
                proc.child.kill().await.ok();
                proc.child.wait().await.ok();
            }
        }

        // Restart the main proxy to pick up new credentials
        if self.is_running().await {
            info!("restarting CLIProxyAPI to use new credentials");
            self.stop().await?;
            self.start().await?;
        }

        info!("Claude OAuth login completed");
        Ok(())
    }

    /// Read existing API key from config file (if any).
    async fn read_existing_api_key(&self) -> Option<String> {
        let content = tokio::fs::read_to_string(self.config_path()).await.ok()?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("- api-key:") || trimmed.starts_with("api-key:") {
                let value = trimmed.split(':').nth(1)?.trim().trim_matches('"').trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
        None
    }

    /// Auto-start if configured and binary exists.
    pub async fn auto_start(&self) {
        let config = self.config.read().await;
        let should_start = config.auto_start;
        drop(config);

        if should_start && self.binary_path().await.is_some() {
            if let Err(e) = self.start().await {
                warn!(?e, "failed to auto-start CLIProxyAPI");
            }
        }
    }
}

/// Determine the deploy directory (where presenter-server lives).
pub fn detect_deploy_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    /// Write a `claude-<email>.json` OAuth token file into the manager's auth
    /// dir, with the given `expired` RFC3339 timestamp (mirrors CLIProxyAPI's
    /// on-disk token format).
    async fn write_token(mgr: &ProxyManager, email: &str, expired: &str) {
        let auth_dir = mgr.auth_dir();
        tokio::fs::create_dir_all(&auth_dir).await.unwrap();
        let body = format!(
            r#"{{"access_token":"a","refresh_token":"r","id_token":"i","email":"{email}","expired":"{expired}","last_refresh":"2026-06-01T00:00:00+02:00","type":"claude","disabled":false}}"#
        );
        tokio::fs::write(auth_dir.join(format!("claude-{email}.json")), body)
            .await
            .unwrap();
    }

    /// Regression for #438: an EXPIRED OAuth token must report NOT authenticated.
    /// Before the fix, `scan_claude_auth()` only checked file existence,
    /// so a dead/expired token reported `claudeAuthenticated:true` (masked the
    /// 2026-06-20 PP outage). No network — pure file + timestamp check.
    #[tokio::test]
    async fn expired_token_is_not_authenticated() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let past = (Utc::now() - Duration::hours(2)).to_rfc3339();
        write_token(&mgr, "expired@example.com", &past).await;
        assert!(
            !mgr.scan_claude_auth().await.authenticated,
            "an expired token must not count as authenticated"
        );
    }

    /// A fresh (not-yet-expired) token must still report authenticated.
    #[tokio::test]
    async fn fresh_token_is_authenticated() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let future = (Utc::now() + Duration::hours(8)).to_rfc3339();
        write_token(&mgr, "fresh@example.com", &future).await;
        assert!(
            mgr.scan_claude_auth().await.authenticated,
            "a fresh token must count as authenticated"
        );
    }

    /// One expired + one fresh token: at least one valid → authenticated.
    #[tokio::test]
    async fn one_fresh_among_expired_is_authenticated() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let past = (Utc::now() - Duration::hours(2)).to_rfc3339();
        let future = (Utc::now() + Duration::hours(8)).to_rfc3339();
        write_token(&mgr, "dead@example.com", &past).await;
        write_token(&mgr, "live@example.com", &future).await;
        assert!(
            mgr.scan_claude_auth().await.authenticated,
            "a fresh token alongside an expired one must count as authenticated"
        );
    }

    /// No token files at all → not authenticated.
    #[tokio::test]
    async fn no_token_is_not_authenticated() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        assert!(
            !mgr.scan_claude_auth().await.authenticated,
            "no token files means not authenticated"
        );
    }

    /// A token file with no parseable `expired` field is treated as valid
    /// (fail-open) — we never regress a working install on a format we don't
    /// recognise; only a *provably* expired token is rejected (#438).
    #[tokio::test]
    async fn unparseable_token_is_authenticated_fail_open() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let auth_dir = mgr.auth_dir();
        tokio::fs::create_dir_all(&auth_dir).await.unwrap();
        // Valid JSON but no `expired` field → Unknown → fail-open.
        tokio::fs::write(
            auth_dir.join("claude-weird@example.com.json"),
            r#"{"access_token":"a","type":"claude"}"#,
        )
        .await
        .unwrap();
        assert!(
            mgr.scan_claude_auth().await.authenticated,
            "a token with no parseable expiry must fail open as authenticated"
        );
    }

    /// A "claude"-named SUBDIRECTORY alongside an expired token must not be
    /// read as a token (it would EISDIR → Unknown → fail-open). The expired
    /// token must still win → not authenticated.
    #[tokio::test]
    async fn claude_named_subdir_does_not_grant_auth() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let auth_dir = mgr.auth_dir();
        tokio::fs::create_dir_all(auth_dir.join("claude-logs"))
            .await
            .unwrap();
        let past = (Utc::now() - Duration::hours(2)).to_rfc3339();
        write_token(&mgr, "expired@example.com", &past).await;
        assert!(
            !mgr.scan_claude_auth().await.authenticated,
            "a claude-named subdir must not grant authentication while the only token is expired"
        );
    }

    /// #599: `status().token_expires_at` surfaces a fresh token's expiry so
    /// the UI can show the operator how long the login stays valid.
    #[tokio::test]
    async fn status_reports_fresh_token_expiry() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let future = (Utc::now() + Duration::hours(8)).to_rfc3339();
        write_token(&mgr, "fresh@example.com", &future).await;
        let status = mgr.status().await;
        assert!(status.claude_authenticated);
        assert_eq!(status.token_expires_at, Some(future));
    }

    /// #599: when every token is expired, `token_expires_at` still surfaces
    /// the newest expiry — "expired at X" — even though `claude_authenticated`
    /// is false, so the login banner can name when the last login died.
    #[tokio::test]
    async fn status_reports_newest_expiry_when_all_tokens_expired() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let older = (Utc::now() - Duration::hours(5)).to_rfc3339();
        let newer = (Utc::now() - Duration::hours(1)).to_rfc3339();
        write_token(&mgr, "older@example.com", &older).await;
        write_token(&mgr, "newer@example.com", &newer).await;
        let status = mgr.status().await;
        assert!(!status.claude_authenticated);
        assert_eq!(status.token_expires_at, Some(newer));
    }

    /// #599: among several FRESH tokens, the one expiring LATEST wins —
    /// the UI should report the longest remaining validity, not an
    /// arbitrary one.
    #[tokio::test]
    async fn status_reports_latest_expiry_among_multiple_fresh_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let sooner = (Utc::now() + Duration::hours(2)).to_rfc3339();
        let later = (Utc::now() + Duration::hours(9)).to_rfc3339();
        write_token(&mgr, "sooner@example.com", &sooner).await;
        write_token(&mgr, "later@example.com", &later).await;
        let status = mgr.status().await;
        assert!(status.claude_authenticated);
        assert_eq!(status.token_expires_at, Some(later));
    }

    /// #599: no tokens on disk at all → `token_expires_at` is `None`, never a
    /// guessed/placeholder timestamp.
    #[tokio::test]
    async fn status_reports_no_expiry_when_no_tokens_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let status = mgr.status().await;
        assert!(!status.claude_authenticated);
        assert_eq!(status.token_expires_at, None);
    }

    /// #599: a token with no parseable `expired` field carries no timestamp
    /// to surface — `token_expires_at` stays `None` even though the fail-open
    /// behavior still reports authenticated.
    #[tokio::test]
    async fn status_reports_no_expiry_for_unparseable_token() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let auth_dir = mgr.auth_dir();
        tokio::fs::create_dir_all(&auth_dir).await.unwrap();
        tokio::fs::write(
            auth_dir.join("claude-weird@example.com.json"),
            r#"{"access_token":"a","type":"claude"}"#,
        )
        .await
        .unwrap();
        let status = mgr.status().await;
        assert!(status.claude_authenticated);
        assert_eq!(status.token_expires_at, None);
    }

    /// #622 post-merge review finding 2 (RED — expected-red by inspection;
    /// Tier-0 forbids running `cargo test -p presenter-server` locally, so
    /// this cannot be executed on this box, only reasoned through the code
    /// path): `authenticated` becomes `true` here ONLY via the fail-open
    /// `Unknown` token — there is no fresh token anywhere in the scan. A
    /// SEPARATE, genuinely expired token also sits in the same auth dir.
    ///
    /// Before the fix, `expires_at = fresh_max.or_else(|| expired_max)` — with
    /// `fresh_max` empty, the EXPIRED token's past timestamp leaks through as
    /// if it were the validity backing `authenticated: true`, and the UI would
    /// show "Prihlásenie ku Claude platí do <a date in the past>". The fix
    /// gates the fallback on `authenticated` itself: authenticated must only
    /// ever surface a FRESH timestamp, `None` when the only backing evidence
    /// is an Unknown token.
    #[tokio::test]
    async fn status_hides_expired_timestamp_when_authenticated_via_unknown_token_only() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ProxyManager::new(tmp.path().to_path_buf());
        let auth_dir = mgr.auth_dir();
        tokio::fs::create_dir_all(&auth_dir).await.unwrap();
        // Unparseable expiry — fail-open, makes `authenticated: true` with no
        // fresh timestamp to back it.
        tokio::fs::write(
            auth_dir.join("claude-weird@example.com.json"),
            r#"{"access_token":"a","type":"claude"}"#,
        )
        .await
        .unwrap();
        // A SEPARATE, genuinely expired token in the same auth dir.
        let past = (Utc::now() - Duration::hours(2)).to_rfc3339();
        write_token(&mgr, "expired@example.com", &past).await;

        let status = mgr.status().await;
        assert!(
            status.claude_authenticated,
            "the Unknown token fail-opens auth"
        );
        assert_eq!(
            status.token_expires_at, None,
            "authenticated via the Unknown token alone must never surface the OTHER \
             (unrelated, expired) token's past timestamp as if it were validity"
        );
    }
}
