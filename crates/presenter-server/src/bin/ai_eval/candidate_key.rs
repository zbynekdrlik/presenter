//! Resolve the candidate endpoint's API key (OpenAI-compatible Bearer auth)
//! from the environment for the `drive` stage.
//!
//! The original harness targeted a LOCAL, keyless candidate endpoint
//! (llama.cpp / another local OpenAI-compatible baseline), so `drive`
//! hard-coded `AiSettings.api_key = None`. #662's OpenRouter rescope
//! (owner, 2026-09-12) makes the candidate a HOSTED, key-authenticated
//! endpoint (`https://openrouter.ai/api/v1`), which 401s without a Bearer
//! token. `ai::client` already sends `Authorization: Bearer <key>` whenever
//! `AiSettings.api_key` is `Some(non-empty)` — the only gap was `drive`
//! never populating it. This resolves it from the environment so no key is
//! ever passed on the command line (visible to `ps`), committed, or baked
//! into a trace.
//!
//! Precedence: `OPENROUTER_API_KEY` (the key the #662 sweep uses, matching
//! the ticket + the odoo-erp AI helper convention) first, then the generic
//! `PRESENTER_AI_API_KEY` the production client's `AiSettings::default()`
//! already reads — so an operator who already exported the production var
//! does not need a second one. An empty or whitespace-only value is treated
//! as unset (a keyless local endpoint must keep working with the var absent
//! OR blank), never sent as `Bearer ` with an empty token.

/// Env vars checked, in precedence order, for the candidate Bearer key.
const KEY_ENV_VARS: [&str; 2] = ["OPENROUTER_API_KEY", "PRESENTER_AI_API_KEY"];

/// Pure resolver — takes a getter closure instead of reading `std::env`
/// itself, so it is unit-testable without mutating process-global state
/// (which would race every other test in this binary reading the same key;
/// same pattern as `ai::client::parse_max_tokens` /
/// `ai::context_budget::parse_context_budget_bytes`).
pub fn resolve_candidate_api_key(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    for name in KEY_ENV_VARS {
        if let Some(value) = get(name) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Read the candidate API key from the real process environment.
pub fn candidate_api_key() -> Option<String> {
    resolve_candidate_api_key(|name| std::env::var(name).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn getter(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    #[test]
    fn none_when_no_var_set() {
        assert_eq!(resolve_candidate_api_key(getter(&[])), None);
    }

    #[test]
    fn reads_openrouter_key() {
        assert_eq!(
            resolve_candidate_api_key(getter(&[("OPENROUTER_API_KEY", "sk-or-abc")])),
            Some("sk-or-abc".to_string())
        );
    }

    #[test]
    fn openrouter_takes_precedence_over_presenter_var() {
        let g = getter(&[
            ("OPENROUTER_API_KEY", "sk-or-primary"),
            ("PRESENTER_AI_API_KEY", "sk-presenter-fallback"),
        ]);
        assert_eq!(
            resolve_candidate_api_key(g),
            Some("sk-or-primary".to_string())
        );
    }

    #[test]
    fn falls_back_to_presenter_var_when_openrouter_unset() {
        assert_eq!(
            resolve_candidate_api_key(getter(&[("PRESENTER_AI_API_KEY", "sk-presenter")])),
            Some("sk-presenter".to_string())
        );
    }

    #[test]
    fn empty_or_whitespace_value_is_treated_as_unset() {
        assert_eq!(
            resolve_candidate_api_key(getter(&[("OPENROUTER_API_KEY", "")])),
            None
        );
        assert_eq!(
            resolve_candidate_api_key(getter(&[("OPENROUTER_API_KEY", "   ")])),
            None
        );
    }

    #[test]
    fn blank_openrouter_falls_through_to_presenter_var() {
        // A blank primary must not shadow a real fallback.
        let g = getter(&[
            ("OPENROUTER_API_KEY", "  "),
            ("PRESENTER_AI_API_KEY", "sk-presenter"),
        ]);
        assert_eq!(
            resolve_candidate_api_key(g),
            Some("sk-presenter".to_string())
        );
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        // The secrets file is stored with a trailing newline; a naive read
        // would send `Bearer sk-or-abc\n`.
        assert_eq!(
            resolve_candidate_api_key(getter(&[("OPENROUTER_API_KEY", "sk-or-abc\n")])),
            Some("sk-or-abc".to_string())
        );
    }
}
