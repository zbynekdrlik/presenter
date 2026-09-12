//! Env-precedence for AI settings (#761): the `PRESENTER_AI_*` env vars must
//! win over the persisted DB `ai-settings` row for the EFFECTIVE call/status
//! path.
//!
//! Root cause this module fixes: `get_settings_internal` reads the DB
//! `ai-settings` row first and only falls back to the env-aware
//! `AiSettings::default()` when NO row exists. Every prod DB (SNV/PP/dev)
//! still holds a pre-migration row pinning `apiUrl` to the old bundled proxy
//! (`http://127.0.0.1:18787/v1`), so `PRESENTER_AI_*` was silently ignored —
//! the deploy-written `/etc/presenter/ai.env` (the #761 OpenRouter switch)
//! would never take effect. `resolve_effective_settings` now applies these
//! overrides for the reachable call/status path ONLY; the persist/display
//! path (`get_settings_internal`) stays raw so env never leaks into a
//! saved/displayed DB row (the #679/#683 data-loss class).
//!
//! The helpers are PARAMETERIZED on the override values (not reading env
//! themselves) so precedence is unit-testable WITHOUT mutating process-global
//! env — a mutated env var races every other test in this binary reading the
//! same key (the same rationale as `parse_idle_clear_minutes` in `ai.rs`).
//! `apply_env_overrides` is the thin env-reading wrapper the production path
//! calls.

use crate::ai::AiSettings;

/// Read one env var as an override: `Some(value)` only when the var is set AND
/// non-empty, else `None`. An empty value (`PRESENTER_AI_API_KEY=`) is treated
/// as "no override" so a deploy that clears a secret cleanly falls back to the
/// stored/default value instead of sending an empty key.
fn env_override(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Apply explicit override values on top of `settings`, in place. A
/// `Some(non-empty)` override wins over the stored value; `None` leaves the
/// stored value untouched. Pure — the caller supplies the override values, so
/// precedence is testable without touching process-global env.
pub(super) fn apply_settings_overrides(
    settings: &mut AiSettings,
    url_override: Option<String>,
    key_override: Option<String>,
    model_override: Option<String>,
) {
    if let Some(url) = url_override {
        settings.api_url = url;
    }
    if let Some(key) = key_override {
        settings.api_key = Some(key);
    }
    if let Some(model) = model_override {
        settings.model = model;
    }
}

/// Apply the `PRESENTER_AI_{API_URL,API_KEY,MODEL}` env-var overrides on top of
/// `settings` for the EFFECTIVE call/status path (#761). Thin wrapper reading
/// the three env vars via [`env_override`] and delegating to
/// [`apply_settings_overrides`].
pub(super) fn apply_env_overrides(settings: &mut AiSettings) {
    apply_settings_overrides(
        settings,
        env_override("PRESENTER_AI_API_URL"),
        env_override("PRESENTER_AI_API_KEY"),
        env_override("PRESENTER_AI_MODEL"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_settings() -> AiSettings {
        // A settings row as prod DBs actually hold it: the bundled proxy URL,
        // no key, the old proxy-only model — exactly the state env must beat.
        AiSettings {
            api_url: "http://127.0.0.1:18787/v1".to_string(),
            api_key: None,
            model: "claude-opus-4-6".to_string(),
            system_prompt_extra: None,
        }
    }

    #[test]
    fn env_url_wins_over_db_api_url() {
        let mut s = db_settings();
        apply_settings_overrides(
            &mut s,
            Some("https://openrouter.ai/api/v1".to_string()),
            None,
            None,
        );
        assert_eq!(
            s.api_url, "https://openrouter.ai/api/v1",
            "PRESENTER_AI_API_URL must win over the stored DB apiUrl"
        );
        // Untouched fields stay as the DB row had them.
        assert_eq!(s.model, "claude-opus-4-6");
        assert_eq!(s.api_key, None);
    }

    #[test]
    fn env_model_and_key_win_over_db() {
        let mut s = db_settings();
        apply_settings_overrides(
            &mut s,
            None,
            Some("sk-or-secret".to_string()),
            Some("google/gemini-3.8-flash".to_string()),
        );
        assert_eq!(s.model, "google/gemini-3.8-flash");
        assert_eq!(s.api_key.as_deref(), Some("sk-or-secret"));
        // apiUrl had no override → stored value kept.
        assert_eq!(s.api_url, "http://127.0.0.1:18787/v1");
    }

    #[test]
    fn no_override_leaves_db_values_intact() {
        let mut s = db_settings();
        let before = s.clone();
        apply_settings_overrides(&mut s, None, None, None);
        assert_eq!(s.api_url, before.api_url);
        assert_eq!(s.model, before.model);
        assert_eq!(s.api_key, before.api_key);
    }

    #[test]
    fn empty_env_value_is_not_an_override() {
        // `env_override` maps "" → None, so a cleared PRESENTER_AI_API_KEY does
        // NOT overwrite a stored key with an empty string. Assert that
        // apply_settings_overrides given None keeps the stored key.
        let mut s = AiSettings {
            api_key: Some("stored-key".to_string()),
            ..db_settings()
        };
        apply_settings_overrides(&mut s, None, None, None);
        assert_eq!(
            s.api_key.as_deref(),
            Some("stored-key"),
            "a None (empty/unset) key override must not clear a stored key"
        );
    }
}
