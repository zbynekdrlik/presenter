//! Credential preflight for the AI call/status path (#762 CI follow-up).
//!
//! Root cause this module fixes: since #762 the default `apiUrl` is OpenRouter
//! (`https://openrouter.ai/api/v1`). A keyless request to a REMOTE backend
//! egresses a real, billable/telemetry call and comes back 401/403 — the exact
//! failure that reddened Playwright E2E (2/3) in run 34720592378, where the CI
//! test server had no key and the spec expected the old dead-proxy "failed to
//! reach" text. It is also a genuine prod hazard: a misconfigured instance
//! (env/DB with a remote URL but no key) would silently egress on every chat.
//!
//! Policy: fail fast BEFORE any HTTP when the effective settings have no API
//! key AND the `api_url` host is **non-loopback**. Loopback (`127.0.0.1`,
//! `::1`, `localhost`) is the legitimate keyless local-backend case (llama.cpp
//! / CLIProxyAPI on `127.0.0.1:18787`) and must keep working — so this is a
//! host classification, not a string match against the default URL, and it
//! also protects a hand-edited remote URL and any future default.
//!
//! Pure + parameterized on `AiSettings` — no env, no network — so it is
//! unit-testable without touching process-global state (same discipline as
//! `ai_env::apply_settings_overrides`).

use super::AiSettings;
use std::net::IpAddr;

/// Operator-facing message shown when a keyless request would go to a remote
/// backend. Slovak, actionable, names both configuration surfaces.
pub(crate) const MISSING_KEY_MESSAGE: &str =
    "AI nie je nakonfigurované: chýba API kľúč (PRESENTER_AI_API_KEY alebo Nastavenia → AI)";

/// Whether `api_url`'s host is a loopback address — a LOCAL OpenAI-compatible
/// backend (llama.cpp / CLIProxyAPI on `127.0.0.1`) that legitimately needs no
/// API key. `127.0.0.1`, `::1` (with or without URL brackets) and `localhost`
/// all count. An unparseable or host-less URL is treated as NON-loopback so the
/// guard fails fast rather than risk a keyless egress.
fn is_loopback_host(api_url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(api_url) else {
        return false;
    };
    match parsed.host_str() {
        Some(host) => {
            let host = host.trim_start_matches('[').trim_end_matches(']');
            if host.eq_ignore_ascii_case("localhost") {
                return true;
            }
            host.parse::<IpAddr>()
                .map(|ip| ip.is_loopback())
                .unwrap_or(false)
        }
        None => false,
    }
}

/// If the effective settings would send a KEYLESS request to a NON-loopback
/// backend, return the operator-facing error message so the caller can fail
/// fast with ZERO network egress; otherwise `None` (proceed with the call).
///
/// A remote OpenAI-compatible backend always requires auth, so a keyless
/// request is a misconfiguration that egresses a billable/telemetry call and
/// returns 401/403. Loopback backends are exempt (see [`is_loopback_host`]).
pub(crate) fn missing_key_for_remote_backend(settings: &AiSettings) -> Option<&'static str> {
    let has_key = settings.api_key.as_deref().is_some_and(|k| !k.is_empty());
    if has_key || is_loopback_host(&settings.api_url) {
        return None;
    }
    Some(MISSING_KEY_MESSAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(api_url: &str, api_key: Option<&str>) -> AiSettings {
        AiSettings {
            api_url: api_url.to_string(),
            api_key: api_key.map(|k| k.to_string()),
            model: "test-model".to_string(),
            system_prompt_extra: None,
        }
    }

    // --- keyless + REMOTE host → fail fast with the credential message ---

    #[test]
    fn keyless_openrouter_default_is_blocked() {
        let got = missing_key_for_remote_backend(&settings("https://openrouter.ai/api/v1", None));
        assert_eq!(got, Some(MISSING_KEY_MESSAGE));
        assert!(
            MISSING_KEY_MESSAGE.contains("API kľúč"),
            "operator message must mention the missing API key"
        );
    }

    #[test]
    fn keyless_arbitrary_remote_host_is_blocked() {
        // Policy is "non-loopback host", not a match against the default URL —
        // so a hand-edited remote URL is caught too.
        assert_eq!(
            missing_key_for_remote_backend(&settings("https://api.example.com/v1", None)),
            Some(MISSING_KEY_MESSAGE)
        );
    }

    #[test]
    fn keyless_empty_key_string_is_treated_as_no_key() {
        assert_eq!(
            missing_key_for_remote_backend(&settings("https://openrouter.ai/api/v1", Some(""))),
            Some(MISSING_KEY_MESSAGE)
        );
    }

    // --- keyless + LOOPBACK host → allowed (local backend needs no key) ---

    #[test]
    fn keyless_loopback_ipv4_is_allowed() {
        assert_eq!(
            missing_key_for_remote_backend(&settings("http://127.0.0.1:1/v1", None)),
            None
        );
    }

    #[test]
    fn keyless_loopback_ipv4_with_proxy_port_is_allowed() {
        assert_eq!(
            missing_key_for_remote_backend(&settings("http://127.0.0.1:18787/v1", None)),
            None
        );
    }

    #[test]
    fn keyless_localhost_is_allowed() {
        assert_eq!(
            missing_key_for_remote_backend(&settings("http://localhost:8080/v1", None)),
            None
        );
    }

    #[test]
    fn keyless_loopback_ipv6_is_allowed() {
        assert_eq!(
            missing_key_for_remote_backend(&settings("http://[::1]:8080/v1", None)),
            None
        );
    }

    // --- key present → always allowed, loopback or remote ---

    #[test]
    fn keyed_remote_is_allowed() {
        assert_eq!(
            missing_key_for_remote_backend(&settings(
                "https://openrouter.ai/api/v1",
                Some("sk-or-testkey")
            )),
            None
        );
    }
}
