//! Server startup mode (#771).
//!
//! The deploy/pipeline/release "Validate database schema" steps run the release
//! binary against a COPY of the live database while the real `presenter.service`
//! is still running. Before #771 that probe binary took the ONE normal startup
//! path and brought up every integration — the Companion websocket (fixed port
//! 18175), the NDI source restore (a SECOND pipeline on the shared VA-API
//! encoder), the OSC listener, the AbleSet/Resolume pollers, sync, and the
//! Android launcher — so it collided with the live service's fixed ports and
//! duplicated the NDI pipeline mid-deploy, and the probe misreported the port
//! collision as a broken migration.
//!
//! [`StartupMode::Validate`] is the fix: migrate + open the DB + serve
//! `/healthz`, but start NO integration or background task. Selected once via
//! the `PRESENTER_STARTUP_MODE` env var (parsed in `ServerConfig::load`),
//! threaded through `AppState::from_config`, and gated at the single startup
//! seam — never sprinkled as per-module `if` checks.

use std::env;

/// The environment variable that selects the startup mode.
pub const STARTUP_MODE_ENV: &str = "PRESENTER_STARTUP_MODE";

/// How the server boots.
///
/// `Normal` is the production/dev boot: every integration and background task
/// starts. `Validate` is the schema-validation probe boot: migrations run, the
/// DB opens, `/healthz` serves, and NOTHING else starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartupMode {
    /// Full boot — all integrations + background tasks (the default).
    #[default]
    Normal,
    /// Schema-validation probe — migrate + serve `/healthz` only, no integrations.
    Validate,
}

impl StartupMode {
    /// Parse the mode from a raw env value.
    ///
    /// - `None` / empty → [`StartupMode::Normal`]
    /// - `validate` (case-insensitive, trimmed) → [`StartupMode::Validate`]
    /// - `normal` → [`StartupMode::Normal`]
    /// - anything else → [`StartupMode::Normal`] with a WARN (an unrecognised
    ///   value must NEVER silently skip integrations on a real boot)
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None => StartupMode::Normal,
            Some(value) if value.eq_ignore_ascii_case("validate") => StartupMode::Validate,
            Some(value) if value.eq_ignore_ascii_case("normal") => StartupMode::Normal,
            Some(other) => {
                tracing::warn!(
                    value = %other,
                    "unknown PRESENTER_STARTUP_MODE; defaulting to normal (integrations enabled)"
                );
                StartupMode::Normal
            }
        }
    }

    /// Parse the mode from the process environment (`PRESENTER_STARTUP_MODE`).
    pub fn from_env() -> Self {
        Self::parse(env::var(STARTUP_MODE_ENV).ok().as_deref())
    }

    /// Whether this mode starts integrations and background tasks.
    pub fn starts_integrations(self) -> bool {
        matches!(self, StartupMode::Normal)
    }

    /// Whether this is the schema-validation probe mode.
    pub fn is_validate(self) -> bool {
        matches!(self, StartupMode::Validate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_defaults_to_normal() {
        assert_eq!(StartupMode::parse(None), StartupMode::Normal);
    }

    #[test]
    fn empty_or_whitespace_defaults_to_normal() {
        assert_eq!(StartupMode::parse(Some("")), StartupMode::Normal);
        assert_eq!(StartupMode::parse(Some("   ")), StartupMode::Normal);
    }

    #[test]
    fn validate_value_selects_validate_mode() {
        assert_eq!(StartupMode::parse(Some("validate")), StartupMode::Validate);
    }

    #[test]
    fn validate_value_is_case_insensitive_and_trimmed() {
        assert_eq!(
            StartupMode::parse(Some("  VALIDATE  ")),
            StartupMode::Validate
        );
        assert_eq!(StartupMode::parse(Some("Validate")), StartupMode::Validate);
    }

    #[test]
    fn normal_value_selects_normal_mode() {
        assert_eq!(StartupMode::parse(Some("normal")), StartupMode::Normal);
    }

    #[test]
    fn unknown_value_defaults_to_normal() {
        // An unrecognised value must fall back to a FULL boot, never a silent
        // integrations-off boot (that would take a real prod deploy off-air).
        assert_eq!(StartupMode::parse(Some("garbage")), StartupMode::Normal);
    }

    #[test]
    fn default_is_normal() {
        assert_eq!(StartupMode::default(), StartupMode::Normal);
    }

    #[test]
    fn gating_helpers_match_the_mode() {
        assert!(StartupMode::Normal.starts_integrations());
        assert!(!StartupMode::Normal.is_validate());
        assert!(!StartupMode::Validate.starts_integrations());
        assert!(StartupMode::Validate.is_validate());
    }
}
