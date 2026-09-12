//! Credential redaction for any backend-error / diagnostic line before it is
//! logged or surfaced on `/healthz`/`/ai/status`.
//!
//! Extracted from the former `proxy_output_relay` module (#762) when the
//! bundled CLIProxyAPI proxy + Claude OAuth login flow were removed: the relay
//! itself is gone, but `redact_proxy_output_line` is still needed by
//! `ai::last_error` (#764) to strip a credential out of a completion error
//! body before it reaches the operator status. Its `SECRET_PREFIX_RE` also
//! covers OpenRouter `sk-or-v1-` keys (the #761 backend), so redaction lives
//! here in exactly one place. Pure + unit-tested, needs nothing but `regex`.

use regex::Regex;
use std::sync::LazyLock;

/// Marker substituted for any credential-shaped match `redact_proxy_output_line`
/// finds.
const REDACTED_MARKER: &str = "<redacted>";

/// Matches a Claude OAuth access/refresh token or an Anthropic API key by
/// their STABLE prefix (`sk-ant-oat`/`sk-ant-ort`/`sk-ant-api`, each
/// followed by a 2-digit version and a `-`), OR an OpenRouter API key by its
/// stable `sk-or-v1-` prefix (the #761 backend; #764 added it here so a key
/// in a completion error body is redacted before it reaches
/// `/healthz`/`/ai/status`). Anchored on the prefix, never on entropy, so it
/// never touches an unrelated opaque string that a backend legitimately logs
/// (e.g. a long hex integrity hash — a naive "any long hex/base64 string"
/// heuristic would have blanked that, destroying diagnostic value for no
/// safety gain).
static SECRET_PREFIX_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"sk-(?:ant-(?:oat|ort|api)\d{2}|or-v1)-[A-Za-z0-9_-]+").ok());

/// Matches an `Authorization: Bearer <token>` value, case-insensitively,
/// keeping the `Bearer` keyword and redacting only the token itself.
static BEARER_TOKEN_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)(bearer\s+)\S+").ok());

/// Matches a raw JSON credential field (`"access_token":"..."` etc.) —
/// defense-in-depth in case a backend ever dumps that JSON verbatim into a
/// log/error line.
static SECRET_JSON_FIELD_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)"(access_token|refresh_token|id_token|api[_-]?key|client_secret)"\s*:\s*"[^"]*""#,
    )
    .ok()
});

/// Redact any credential-shaped substring from one line before Presenter logs
/// it or surfaces it. Pure and unit-tested (see `tests` below) against both
/// the real credential-bearing shapes it targets AND real clean lines, so a
/// too-aggressive pattern that would blank legitimate diagnostic text gets
/// caught the same way a too-narrow one would.
pub(crate) fn redact_proxy_output_line(line: &str) -> String {
    let mut out = line.to_string();
    if let Some(re) = SECRET_PREFIX_RE.as_ref() {
        if re.is_match(&out) {
            out = re.replace_all(&out, REDACTED_MARKER).into_owned();
        }
    }
    if let Some(re) = BEARER_TOKEN_RE.as_ref() {
        if re.is_match(&out) {
            let replacement = format!("${{1}}{REDACTED_MARKER}");
            out = re.replace_all(&out, replacement.as_str()).into_owned();
        }
    }
    if let Some(re) = SECRET_JSON_FIELD_RE.as_ref() {
        if re.is_match(&out) {
            let replacement = format!(r#""$1":"{REDACTED_MARKER}""#);
            out = re.replace_all(&out, replacement.as_str()).into_owned();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_hides_claude_oauth_access_token() {
        let line = "loaded auth: sk-ant-oat01-AABBCCDDEEFF00112233445566778899AABBCCDDEEFF0011";
        let redacted = redact_proxy_output_line(line);
        assert!(
            !redacted.contains("AABBCCDDEEFF"),
            "the raw access token must not survive redaction: {redacted}"
        );
        assert!(
            redacted.contains(REDACTED_MARKER),
            "a redaction marker must appear in place of the token: {redacted}"
        );
    }

    #[test]
    fn redact_hides_claude_oauth_refresh_token() {
        let line = "refreshing with sk-ant-ort01-ZZYYXXWWVVUUTTSSRRQQPPOONNMMLLKKJJIIHHGG";
        let redacted = redact_proxy_output_line(line);
        assert!(
            !redacted.contains("ZZYYXXWWVVUUTTSS"),
            "the raw refresh token must not survive redaction: {redacted}"
        );
        assert!(redacted.contains(REDACTED_MARKER));
    }

    #[test]
    fn redact_hides_anthropic_api_key() {
        let line = "using key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH";
        let redacted = redact_proxy_output_line(line);
        assert!(
            !redacted.contains("abcdefghijklmnop"),
            "the raw API key must not survive redaction: {redacted}"
        );
        assert!(redacted.contains(REDACTED_MARKER));
    }

    #[test]
    fn redact_hides_openrouter_api_key() {
        // #761/#764: the backend is now OpenRouter, whose keys carry the
        // `sk-or-v1-` prefix. A key in a completion error body must be
        // redacted before it can reach `/healthz`/`/ai/status`.
        let line = "invalid credentials: sk-or-v1-FAKEnotARealKey_test_XYZ";
        let redacted = redact_proxy_output_line(line);
        assert!(
            !redacted.contains("sk-or-v1-"),
            "the raw OpenRouter API key must not survive redaction: {redacted}"
        );
        assert!(redacted.contains(REDACTED_MARKER));
    }

    #[test]
    fn redact_hides_bearer_authorization_header_value() {
        // Deliberately NOT `sk-ant-...`-shaped, so this proves
        // BEARER_TOKEN_RE catches it independently of SECRET_PREFIX_RE.
        let line = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.OPAQUE-JWT-VALUE.sig";
        let redacted = redact_proxy_output_line(line);
        assert!(
            !redacted.contains("OPAQUE-JWT-VALUE"),
            "the raw bearer token must not survive redaction: {redacted}"
        );
        assert!(
            redacted.contains("Bearer"),
            "the Bearer keyword itself is not a secret and should stay for readability: {redacted}"
        );
        assert!(redacted.contains(REDACTED_MARKER));
    }

    #[test]
    fn redact_hides_json_credential_fields() {
        let line =
            r#"{"access_token":"abc123secret","refresh_token":"def456secret","type":"claude"}"#;
        let redacted = redact_proxy_output_line(line);
        assert!(!redacted.contains("abc123secret"), "{redacted}");
        assert!(!redacted.contains("def456secret"), "{redacted}");
        assert!(
            redacted.contains("\"access_token\":\"<redacted>\""),
            "{redacted}"
        );
        assert!(
            redacted.contains("\"refresh_token\":\"<redacted>\""),
            "{redacted}"
        );
    }

    /// A real backend error line carrying no credential (only a generic OAuth
    /// error body, a status code, and a retry counter) MUST pass through
    /// byte-for-byte unchanged — proves the filter does not over-redact
    /// ordinary diagnostic text.
    #[test]
    fn redact_leaves_the_real_refresh_failure_line_unchanged() {
        let real_line = r#"[2026-08-12 22:36:48] [--------] [warn ] [anthropic_auth.go:655] Token refresh attempt 1 failed: token refresh failed with status 400: {"error": "invalid_grant", "error_description": "Refresh token not found or invalid"}"#;
        assert_eq!(redact_proxy_output_line(real_line), real_line);
    }

    /// A line containing a long legitimate hex integrity hash must also pass
    /// through unchanged — proves the patterns are anchored on STRUCTURE
    /// (known key names / known token prefixes), never on a blind "long
    /// opaque string" entropy heuristic that would needlessly blank this.
    #[test]
    fn redact_leaves_the_real_management_asset_hash_line_unchanged() {
        let real_line = "[2026-08-12 22:37:04] [--------] [info ] [updater.go:308] management asset updated from fallback page successfully (hash=5eff7e63a6cafe5f32d3622877feb284a80a5ee60ede8c8790d6ab9516acb732)";
        assert_eq!(redact_proxy_output_line(real_line), real_line);
    }
}
