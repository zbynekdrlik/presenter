//! Backend-agnostic AI health summary for `/healthz` (#760).
//!
//! The SNV production AI assistant sat dead for 14 days because NOTHING
//! external could see the failure — the only signals were a journal WARN, an
//! operator-header chip (opened only during a service), and a deploy-gate
//! `::warning::` nobody reads. This module surfaces the SAME connectivity
//! verdict `/ai/status` computes on `/healthz`, so an external watchdog can
//! poll it and alert the owner within minutes.
//!
//! Deliberately backend-agnostic (`{connected, error, model}`, no OAuth
//! fields) so it survives the CLIProxyAPI -> OpenRouter migration (#662):
//! `connected`/`error` come from the shared `evaluate_ai_status`, whose
//! OAuth-specific input only participates when the bundled proxy is in use.
//!
//! Lives in its own small module rather than growing `router.rs` or the
//! already-over-warning-cap `router/ai.rs` (project file-line gate).

use super::ai::evaluate_ai_status;
use crate::state::AppState;
use serde_json::{json, Value};
use tracing::warn;

/// Build the `/healthz` `ai` object. Best-effort: a settings/DB failure is
/// folded into `connected: false` + a generic `error` string rather than
/// failing the whole readiness probe (the AI verdict must never take
/// `/healthz` itself down). Performs one bounded (3s) connectivity probe via
/// `evaluate_ai_status` — see its doc comment.
pub(super) async fn render_ai_health(state: &AppState) -> Value {
    match evaluate_ai_status(state).await {
        Ok((status, model)) => ai_health_json(status.connected, status.error.as_deref(), &model),
        Err(e) => {
            // Detail to the log (may contain DB internals); a stable, generic
            // string to the public endpoint.
            warn!(?e, "/healthz AI status check failed");
            ai_health_json(false, Some("AI status check failed"), "")
        }
    }
}

/// Pure render of the backend-agnostic `ai` object. `error` is `null` when
/// connected, else a human-readable string — the KEY is always present so a
/// watchdog can rely on the schema.
fn ai_health_json(connected: bool, error: Option<&str>, model: &str) -> Value {
    json!({
        "connected": connected,
        "error": error,
        "model": model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_state_has_null_error_and_passes_model_through() {
        let v = ai_health_json(true, None, "anthropic/claude-3.5-sonnet");
        assert_eq!(v["connected"], serde_json::json!(true));
        assert!(v["error"].is_null(), "connected -> error must be null");
        assert_eq!(v["model"], serde_json::json!("anthropic/claude-3.5-sonnet"));
    }

    #[test]
    fn unreachable_state_carries_the_error_string() {
        let v = ai_health_json(false, Some("AI proxy unreachable"), "some-model");
        assert_eq!(v["connected"], serde_json::json!(false));
        assert_eq!(v["error"], serde_json::json!("AI proxy unreachable"));
        assert_eq!(v["model"], serde_json::json!("some-model"));
    }

    #[test]
    fn schema_keys_are_always_present() {
        // A watchdog keys on these three fields — they must exist in every
        // state, incl. the DB-failure fallback (empty model, generic error).
        let v = ai_health_json(false, Some("AI status check failed"), "");
        assert!(v.get("connected").is_some());
        assert!(v.get("error").is_some());
        assert!(v.get("model").is_some());
        assert_eq!(v["model"], serde_json::json!(""));
    }
}
