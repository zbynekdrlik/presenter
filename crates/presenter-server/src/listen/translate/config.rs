//! The guest translation env settings (#999 L1), read once at startup:
//! `PRESENTER_LISTEN_GEMINI_API_KEY` (unset = nobody can listen: the page says
//! so), `PRESENTER_LISTEN_GEMINI_MODEL` (default [`DEFAULT_MODEL`], the R2
//! decision — a preview model id churns, so it is a setting),
//! `PRESENTER_LISTEN_GEMINI_URL` (default [`DEFAULT_URL`]; tests and E2E point
//! it at a fake, `ws://` allowed) and `PRESENTER_LISTEN_GUEST_MIC` (the mic id
//! whose audio is translated, default [`DEFAULT_SOURCE_MIC`] = the preacher's
//! handheld on Dante), `PRESENTER_GUEST_PORT` (the guest listener: a second
//! TCP port that serves ONLY the guest surface — the public no-login tunnel
//! points there, so a missing setting means no listener, never the app; unset =
//! no guest page on this box, #999 arch review) and `PRESENTER_GUEST_HOST`
//! (only the public address the operator's QR page prints) and
//! `PRESENTER_LISTEN_GEMINI_CONTEXT` (the setup's `systemInstruction`, the
//! translation context; default [`DEFAULT_CONTEXT`], #999 L6c). An empty or
//! whitespace-only value counts as unset. `Debug` never prints the key.

pub const KEY_VAR: &str = "PRESENTER_LISTEN_GEMINI_API_KEY";
pub const MODEL_VAR: &str = "PRESENTER_LISTEN_GEMINI_MODEL";
pub const URL_VAR: &str = "PRESENTER_LISTEN_GEMINI_URL";
pub const SOURCE_MIC_VAR: &str = "PRESENTER_LISTEN_GUEST_MIC";
pub const GUEST_HOST_VAR: &str = "PRESENTER_GUEST_HOST";
pub const GUEST_PORT_VAR: &str = "PRESENTER_GUEST_PORT";
pub const CONTEXT_VAR: &str = "PRESENTER_LISTEN_GEMINI_CONTEXT";

/// Gemini Live's WebSocket (`BidiGenerateContent`, API v1beta).
pub const DEFAULT_URL: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
/// FINDING R2 (#999): the only Gemini model that translates continuously.
pub const DEFAULT_MODEL: &str = "gemini-3.5-live-translate-preview";
/// `HAND 1 post` on SNV's Dante (#933): the preacher.
pub const DEFAULT_SOURCE_MIC: &str = "dante-mic-1";
/// The translation context every session's setup carries as its
/// `systemInstruction` (#999 L6c, owner 27.11: "Newlevel Church" must come
/// out as the church's name). The doc names it the place for "custom
/// translation context (such as glossaries, terminology, and register)".
/// Measured on Slovak speech, 3 clips × EN/DE × 3 runs each way: without it
/// "New Level Church" every time, once "New Level Chair", "veríme" heard as
/// "v Ríme" ("in Rome", 6/6 runs of one clip) and "zbor" as a choir; with it
/// "Newlevel Church" 36/36 and those errors gone. The repo is public: only
/// the church's public name belongs here, never an address.
pub const DEFAULT_CONTEXT: &str = "This is a live sermon at the Slovak church \"Newlevel Church\" (also written \"New Level Church\"). \"Newlevel Church\" is the church's name: keep it exactly as \"Newlevel Church\" in every language, never translate, respell or split it. Use the register of a Christian sermon and the usual Bible terms of the target language.";

#[derive(Clone, PartialEq, Eq)]
pub struct GeminiEnv {
    pub url: String,
    pub key: Option<String>,
    pub model: String,
    pub source_mic: String,
    /// The guests' public hostname (for the QR page), lowercase, without a port.
    pub guest_host: Option<String>,
    /// The port of the guest-only listener.
    pub guest_port: Option<u16>,
    /// The setup's `systemInstruction` text.
    pub context: String,
}

impl GeminiEnv {
    pub fn from_env() -> Self {
        let env = Self::from_vars(|name| std::env::var(name).ok());
        tracing::info!(target: super::TARGET, env = ?env, "guest translation settings read");
        env
    }

    /// The settings from a variable lookup (the process env in production).
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Self {
        let read = |name: &str| {
            get(name)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        Self {
            url: read(URL_VAR).unwrap_or_else(|| DEFAULT_URL.to_string()),
            key: read(KEY_VAR),
            model: read(MODEL_VAR).unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            source_mic: read(SOURCE_MIC_VAR).unwrap_or_else(|| DEFAULT_SOURCE_MIC.to_string()),
            guest_host: read(GUEST_HOST_VAR).map(|host| bare_host(&host)),
            guest_port: read(GUEST_PORT_VAR).and_then(|port| {
                let parsed = port.parse::<u16>().ok().filter(|p| *p != 0);
                if parsed.is_none() {
                    tracing::warn!(target: super::TARGET, variable = GUEST_PORT_VAR, value = port.as_str(), "not a port number: no guest listener");
                }
                parsed
            }),
            context: read(CONTEXT_VAR).unwrap_or_else(|| DEFAULT_CONTEXT.to_string()),
        }
    }
}

/// A `Host` value as the guest host is compared: lowercase, no port, no
/// trailing dot.
pub fn bare_host(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    let without_port = match host.rsplit_once(':') {
        Some((name, port))
            if (!name.contains(':') || name.ends_with(']'))
                && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            name
        }
        _ => host.as_str(),
    };
    without_port.trim_end_matches('.').to_string()
}

impl std::fmt::Debug for GeminiEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key = if self.key.is_some() {
            "<set>"
        } else {
            "<unset>"
        };
        f.debug_struct("GeminiEnv")
            .field("url", &self.url)
            .field("key", &key)
            .field("model", &self.model)
            .field("source_mic", &self.source_mic)
            .field("guest_host", &self.guest_host)
            .field("guest_port", &self.guest_port)
            .field("context_chars", &self.context.chars().count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(vars: &[(&str, &str)]) -> GeminiEnv {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        GeminiEnv::from_vars(|name| map.get(name).cloned())
    }

    #[test]
    fn defaults_overrides_and_blank_values() {
        let defaults = env(&[]);
        assert_eq!(defaults.url, DEFAULT_URL);
        assert!(defaults
            .url
            .starts_with("wss://generativelanguage.googleapis.com/ws/"));
        assert_eq!(defaults.model, "gemini-3.5-live-translate-preview");
        assert_eq!(defaults.key, None);
        assert_eq!(defaults.source_mic, "dante-mic-1");

        let set = env(&[
            (URL_VAR, "ws://127.0.0.1:9/fake"),
            (KEY_VAR, " test-key-not-real\n"),
            (MODEL_VAR, "gemini-9-live-translate"),
            (SOURCE_MIC_VAR, "replay"),
            (GUEST_HOST_VAR, " Listen.Example.org:443 "),
        ]);
        assert_eq!(set.url, "ws://127.0.0.1:9/fake");
        assert_eq!(set.key.as_deref(), Some("test-key-not-real"));
        assert_eq!(set.model, "gemini-9-live-translate");
        assert_eq!(set.source_mic, "replay");
        assert_eq!(set.guest_host.as_deref(), Some("listen.example.org"));
        assert_eq!(defaults.guest_host, None);
        assert_eq!(defaults.guest_port, None);
        assert_eq!(env(&[(GUEST_PORT_VAR, " 8099 ")]).guest_port, Some(8099));
        for bad in ["0", "99999", "http", "-1"] {
            assert_eq!(env(&[(GUEST_PORT_VAR, bad)]).guest_port, None, "{bad}");
        }

        assert_eq!(defaults.context, DEFAULT_CONTEXT);
        assert!(defaults.context.contains("\"Newlevel Church\""));
        assert_eq!(
            env(&[(CONTEXT_VAR, " A sermon at Example Chapel. ")]).context,
            "A sermon at Example Chapel."
        );

        let blank = env(&[
            (KEY_VAR, "  "),
            (MODEL_VAR, ""),
            (SOURCE_MIC_VAR, " "),
            (CONTEXT_VAR, " \n"),
        ]);
        assert_eq!(blank.context, DEFAULT_CONTEXT);
        assert_eq!(blank.key, None);
        assert_eq!(blank.model, DEFAULT_MODEL);
        assert_eq!(blank.source_mic, DEFAULT_SOURCE_MIC);
    }

    #[test]
    fn a_host_is_compared_lowercase_without_its_port() {
        assert_eq!(bare_host("Listen.Example.org"), "listen.example.org");
        assert_eq!(bare_host("listen.example.org:8080"), "listen.example.org");
        assert_eq!(bare_host("listen.example.org."), "listen.example.org");
        assert_eq!(bare_host("localhost:18999"), "localhost");
        assert_eq!(bare_host("[::1]:80"), "[::1]");
    }

    #[test]
    fn debug_never_prints_the_key() {
        let printed = format!("{:?}", env(&[(KEY_VAR, "test-key-not-real")]));
        assert!(!printed.contains("test-key-not-real"), "{printed}");
        assert!(printed.contains("key: \"<set>\""), "{printed}");
        assert!(format!("{:?}", env(&[])).contains("key: \"<unset>\""));
    }
}
