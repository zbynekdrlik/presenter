//! Relay the vendored CLIProxyAPI child's own stdout/stderr into
//! Presenter's tracing output, with credential redaction.
//!
//! Observability gap found while validating #675 (no dedicated ticket —
//! see the design comment posted on #675 for the full empirical +
//! upstream-source evidence this was built from). Before this,
//! `ProxyManager::start` (`proxy.rs`) ran the CLIProxyAPI child with
//! `Stdio::null()` on both streams, and the config this project generates
//! for it sets `logging-to-file: false` — so a failed OAuth refresh left
//! ZERO evidence of *why* it failed anywhere Presenter's own journal could
//! show it.
//!
//! Split out of `proxy.rs` rather than growing it further (the same reason
//! `refresh.rs` sits alongside it, not inside it — see that file's own doc
//! comment): this logic needs nothing from `ProxyManager`'s internals, only
//! `tokio::io` + `regex` + `tracing`, so it is fully decoupled and
//! independently testable.

use regex::Regex;
use std::sync::LazyLock;
use tokio::io::AsyncBufReadExt;
use tracing::info;

/// Marker substituted for any credential-shaped match `redact_proxy_output_line`
/// finds in the vendored proxy child's own log output.
const REDACTED_MARKER: &str = "<redacted>";

/// Matches a Claude OAuth access/refresh token or an Anthropic API key by
/// their STABLE prefix (`sk-ant-oat`/`sk-ant-ort`/`sk-ant-api`, each
/// followed by a 2-digit version and a `-`) — matches the on-disk token
/// shape `ProxyManager`'s own `write_token` test helper (`proxy.rs`)
/// mirrors, and Anthropic's published key format. Anchored on the prefix,
/// never on entropy, so it never touches an unrelated opaque string the
/// vendor legitimately logs (e.g. the management-asset integrity hash
/// observed live on a real run:
/// `hash=5eff7e63a6cafe5f32d3622877feb284a80a5ee60ede8c8790d6ab9516acb732`
/// — a naive "any long hex/base64 string" heuristic would have blanked
/// that, destroying diagnostic value for no safety gain).
static SECRET_PREFIX_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"sk-ant-(?:oat|ort|api)\d{2}-[A-Za-z0-9_-]+").ok());

/// Matches an `Authorization: Bearer <token>` value, case-insensitively,
/// keeping the `Bearer` keyword and redacting only the token itself.
static BEARER_TOKEN_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)(bearer\s+)\S+").ok());

/// Matches a raw JSON credential field (`"access_token":"..."` etc.) shaped
/// the way CLIProxyAPI's own on-disk token file, and Anthropic's OAuth
/// token response body, both use — defense-in-depth in case a FUTURE
/// vendor version ever dumps that JSON verbatim into a log line. Nothing
/// observed live does this today: a real run of v7.2.130 against a
/// deliberately invalid/expired token only ever logged Anthropic's own
/// generic OAuth error body (`{"error": "invalid_grant", ...}`), never the
/// token that was submitted — confirmed both empirically and by reading
/// `refreshTokensSingleFlight` in the upstream Go source at that exact
/// tag (`internal/auth/claude/anthropic_auth.go`): on a non-200 response it
/// only ever embeds the RESPONSE body in the returned error, never the
/// REQUEST body (which is the one that actually carries `refresh_token`).
static SECRET_JSON_FIELD_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)"(access_token|refresh_token|id_token|api[_-]?key|client_secret)"\s*:\s*"[^"]*""#,
    )
    .ok()
});

/// Defensive cap on a single captured+redacted line before it is logged, so
/// one pathological huge line from the child can never single-handedly
/// blow up the journal. Every line observed live from the real v7.2.130
/// binary (including a failed-refresh warning and a request-log line) was
/// well under 300 bytes; 4 KiB leaves generous headroom while still
/// bounding the worst case. (Ordinary total volume is not a concern here —
/// the periodic auto-refresh this closes the gap for only logs on
/// *failure*, per the upstream source, so a healthy install stays silent.)
const MAX_CAPTURED_LINE_LEN: usize = 4096;

/// Redact any credential-shaped substring from one line of the vendored
/// CLIProxyAPI child's own stdout/stderr before Presenter logs it. Pure and
/// unit-tested (see `tests` below) against both the real credential-bearing
/// shapes it targets AND the real clean lines observed live, so a
/// too-aggressive pattern that would blank legitimate diagnostic text gets
/// caught the same way a too-narrow one would.
pub(crate) fn redact_proxy_output_line(line: &str) -> String {
    // RED (#675-adjacent observability gap, no dedicated ticket): no
    // redaction implemented yet — this stub proves the "hides credential
    // material" tests below genuinely fail before the fix, per this
    // repo's regression-test-first discipline. See the GREEN commit for
    // the real implementation.
    line.to_string()
}

/// Truncate an already-redacted line to `MAX_CAPTURED_LINE_LEN` at a safe
/// UTF-8 char boundary, appending a marker. Truncation always happens
/// AFTER redaction, never before: redacting a shorter string can only ever
/// remove MORE of a would-be secret than redacting a truncated fragment
/// could, never less.
fn truncate_for_log(mut line: String) -> String {
    if line.len() <= MAX_CAPTURED_LINE_LEN {
        return line;
    }
    let mut cut = MAX_CAPTURED_LINE_LEN;
    while cut > 0 && !line.is_char_boundary(cut) {
        cut -= 1;
    }
    line.truncate(cut);
    line.push_str(" …[truncated]");
    line
}

/// Read `stream` line-by-line until EOF, redact + bound each non-empty
/// line, and hand `(source, safe_line)` to `sink`. Pure I/O-in,
/// callback-out shape so the SAME function backs both the production relay
/// (`spawn_proxy_output_relay`, whose sink logs via `tracing::info!`) and a
/// test double (whose sink is a plain collector) — the wiring under test in
/// `tests::relay_reads_and_redacts_a_real_stream_end_to_end` is this exact
/// function, not a stand-in for it. A stream that behaves like
/// `Stdio::null()` (immediate EOF, zero bytes) yields zero calls to `sink`
/// — the same test proves that distinction too.
async fn relay_captured_lines<R>(stream: R, source: &'static str, mut sink: impl FnMut(&str, &str))
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut reader = tokio::io::BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF: child exited or closed the pipe
            Ok(_) => {
                let trimmed = line.trim_end_matches(['\n', '\r']);
                if trimmed.is_empty() {
                    continue;
                }
                let safe = truncate_for_log(redact_proxy_output_line(trimmed));
                sink(source, &safe);
            }
            Err(e) => {
                sink(
                    source,
                    &format!("error reading cli-proxy-api {source}: {e}"),
                );
                break;
            }
        }
    }
}

/// Spawn a background task relaying the vendored CLIProxyAPI child's own
/// stdout/stderr into Presenter's tracing output for the lifetime of the
/// stream (i.e. until the child exits or closes the pipe) — `target:
/// "cli_proxy_api"` clearly labels each line as coming from the child
/// rather than looking like Presenter's own log line, and every line is
/// redacted first via `redact_proxy_output_line`.
///
/// `logging-to-file: true` was considered as an alternative lever and
/// rejected: confirmed live against the real v7.2.130 binary, that flag
/// does not add anything to the journal at all — it REDIRECTS the child's
/// own logger away from stdout entirely into rotating files under
/// `<auth-dir>/logs/main.log`, which would need a separate file read at
/// 3am instead of `journalctl -u presenter-dev`, exactly the diagnosis gap
/// this closes. Callers must keep the generated config at
/// `logging-to-file: false` for this relay to see anything at all.
pub(crate) fn spawn_proxy_output_relay<R>(stream: R, source: &'static str)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(relay_captured_lines(stream, source, |source, line| {
        info!(target: "cli_proxy_api", source = %source, line = %line, "cli-proxy-api output line");
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures below marked "real line" are QUOTED VERBATIM from a live run
    // of the actual vendored v7.2.130 `cli-proxy-api` binary in a scratch
    // dir on dev2, given a deliberately invalid/expired token — never
    // invented text.

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

    /// A real refresh-failure line MUST pass through byte-for-byte
    /// unchanged: it carries no credential, only Anthropic's own generic
    /// OAuth error body, a status code, and a retry counter. Proves the
    /// filter does not over-redact ordinary diagnostic text — this is the
    /// exact line #675's own token-expiry investigation depends on being
    /// legible in the journal.
    #[test]
    fn redact_leaves_the_real_refresh_failure_line_unchanged() {
        let real_line = r#"[2026-08-12 22:36:48] [--------] [warn ] [anthropic_auth.go:655] Token refresh attempt 1 failed: token refresh failed with status 400: {"error": "invalid_grant", "error_description": "Refresh token not found or invalid"}"#;
        assert_eq!(redact_proxy_output_line(real_line), real_line);
    }

    /// A real line containing a long legitimate hex integrity hash must
    /// also pass through unchanged — proves the patterns are anchored on
    /// STRUCTURE (known key names / known token prefixes), never on a
    /// blind "long opaque string" entropy heuristic that would needlessly
    /// blank this.
    #[test]
    fn redact_leaves_the_real_management_asset_hash_line_unchanged() {
        let real_line = "[2026-08-12 22:37:04] [--------] [info ] [updater.go:308] management asset updated from fallback page successfully (hash=5eff7e63a6cafe5f32d3622877feb284a80a5ee60ede8c8790d6ab9516acb732)";
        assert_eq!(redact_proxy_output_line(real_line), real_line);
    }

    #[test]
    fn truncate_for_log_leaves_short_lines_untouched() {
        let short = "a short diagnostic line";
        assert_eq!(truncate_for_log(short.to_string()), short);
    }

    #[test]
    fn truncate_for_log_caps_long_lines_at_a_char_boundary() {
        // A multi-byte char (é = 2 bytes in UTF-8) placed right where a
        // byte-oblivious truncate would land, to prove the char-boundary
        // search actually matters here and isn't a no-op on ASCII.
        let mut long = "x".repeat(MAX_CAPTURED_LINE_LEN - 1);
        long.push('é');
        long.push_str(&"y".repeat(50));
        let out = truncate_for_log(long);
        assert!(
            out.len() <= MAX_CAPTURED_LINE_LEN + " …[truncated]".len(),
            "must be capped near the limit, got {} bytes",
            out.len()
        );
        assert!(
            out.ends_with(" …[truncated]"),
            "must carry the truncation marker: {out}"
        );
    }

    /// The core "capture is wired" proof: `relay_captured_lines` is the
    /// EXACT function `spawn_proxy_output_relay` (used by
    /// `ProxyManager::start`) hands a real child's piped stdout/stderr to —
    /// this test drives it with a real in-memory async stream (not a mock
    /// of the function itself) and proves lines that actually arrive on
    /// the stream are (a) genuinely read and forwarded to the sink, and
    /// (b) redacted before they get there.
    #[tokio::test]
    async fn relay_reads_and_redacts_a_real_stream_end_to_end() {
        use tokio::io::AsyncWriteExt;

        let (mut writer, reader) = tokio::io::duplex(4096);
        let secret_line = "loaded auth: sk-ant-oat01-AABBCCDDEEFF00112233445566778899AABBCC\n";
        let clean_line = "API server started successfully on: 127.0.0.1:18787\n";
        writer
            .write_all(secret_line.as_bytes())
            .await
            .expect("write secret line");
        writer
            .write_all(clean_line.as_bytes())
            .await
            .expect("write clean line");
        writer.shutdown().await.expect("close write half → EOF");

        let captured = std::cell::RefCell::new(Vec::<(String, String)>::new());
        relay_captured_lines(reader, "stdout", |source, line| {
            captured
                .borrow_mut()
                .push((source.to_string(), line.to_string()));
        })
        .await;

        let captured = captured.into_inner();
        assert_eq!(
            captured.len(),
            2,
            "both lines written to the stream must reach the sink: {captured:?}"
        );
        assert!(captured.iter().all(|(source, _)| source == "stdout"));
        assert!(
            !captured[0].1.contains("AABBCCDDEEFF"),
            "the secret line's raw token must not reach the sink: {captured:?}"
        );
        assert!(captured[0].1.contains(REDACTED_MARKER));
        assert_eq!(
            captured[1].1,
            clean_line.trim_end(),
            "a clean line must pass through unredacted"
        );
    }

    /// The contrasting half of the "not null" proof: a stream that behaves
    /// exactly like `Stdio::null()` (opened, then immediately closed with
    /// zero bytes ever written) must yield ZERO sink calls. Together with
    /// the test above, this proves the relay's observable behavior
    /// genuinely depends on the stream actually carrying data — i.e. that
    /// wiring a REAL piped stream (as `ProxyManager::start` now does) is
    /// what makes the difference, not an accident of the sink always
    /// firing anyway.
    #[tokio::test]
    async fn relay_of_an_empty_null_like_stream_calls_sink_zero_times() {
        use tokio::io::AsyncWriteExt;

        let (mut writer, reader) = tokio::io::duplex(64);
        writer.shutdown().await.expect("close write half → EOF");

        let captured = std::cell::RefCell::new(Vec::<(String, String)>::new());
        relay_captured_lines(reader, "stdout", |source, line| {
            captured
                .borrow_mut()
                .push((source.to_string(), line.to_string()));
        })
        .await;

        assert!(
            captured.into_inner().is_empty(),
            "a null-like (empty) stream must never reach the sink"
        );
    }
}
