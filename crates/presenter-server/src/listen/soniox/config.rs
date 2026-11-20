//! The Soniox env settings (#852 C3-3, plan Decision 1), read once at
//! startup: `PRESENTER_LISTEN_SONIOX_API_KEY` (unset = "Chýba kľúč"),
//! `PRESENTER_LISTEN_SONIOX_MODEL` (default [`DEFAULT_MODEL`]),
//! `PRESENTER_LISTEN_SONIOX_URL` (default [`DEFAULT_URL`]; tests point it at
//! a mock, `ws://` allowed) and `PRESENTER_LISTEN_SONIOX_ENDPOINT_DELAY_MS`
//! (unset = endpoint detection off, the measured config; any whole number,
//! 0 included, turns it on, clamped to 500–3000 ms). An empty or
//! whitespace-only value counts as unset. `Debug` never prints the key.

use super::wire::{DEFAULT_MODEL, DEFAULT_URL};

pub const KEY_VAR: &str = "PRESENTER_LISTEN_SONIOX_API_KEY";
pub const MODEL_VAR: &str = "PRESENTER_LISTEN_SONIOX_MODEL";
pub const URL_VAR: &str = "PRESENTER_LISTEN_SONIOX_URL";
pub const ENDPOINT_DELAY_VAR: &str = "PRESENTER_LISTEN_SONIOX_ENDPOINT_DELAY_MS";

#[derive(Clone, PartialEq, Eq)]
pub struct SonioxEnv {
    pub url: String,
    pub key: Option<String>,
    pub model: String,
    pub endpoint_delay_ms: Option<u32>,
}

impl SonioxEnv {
    pub fn from_env() -> Self {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// The settings from a variable lookup (the process env in production).
    /// An endpoint delay that is not a whole number is ignored with a WARN
    /// (endpoint detection stays off).
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Self {
        let read = |name: &str| {
            get(name)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let endpoint_delay_ms = read(ENDPOINT_DELAY_VAR).and_then(|value| {
            let parsed = value.parse::<u32>().ok();
            if parsed.is_none() {
                tracing::warn!(
                    target: "presenter::listen::asr",
                    variable = ENDPOINT_DELAY_VAR,
                    value = value.as_str(),
                    "not a whole number of ms, endpoint detection stays off"
                );
            }
            parsed
        });
        Self {
            url: read(URL_VAR).unwrap_or_else(|| DEFAULT_URL.to_string()),
            key: read(KEY_VAR),
            model: read(MODEL_VAR).unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            endpoint_delay_ms,
        }
    }
}

impl std::fmt::Debug for SonioxEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key = if self.key.is_some() {
            "<set>"
        } else {
            "<unset>"
        };
        f.debug_struct("SonioxEnv")
            .field("url", &self.url)
            .field("key", &key)
            .field("model", &self.model)
            .field("endpoint_delay_ms", &self.endpoint_delay_ms)
            .finish()
    }
}

#[cfg(test)]
mod tests;
