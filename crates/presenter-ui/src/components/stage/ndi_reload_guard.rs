//! #410 server-liveness gate for the #401 last-resort stage-page reload.
//!
//! The #401 watchdog (`ndi_watchdog.rs`) reloads the whole stage page as a
//! last resort when no frame has decoded across reconnect attempts for the
//! no-frames horizon. That assumed reloading could recover ANY stuck stream.
//! But when the active NDI source is legitimately offline (Resolume silent),
//! the server has NO streaming pipeline (`/healthz` `ndi_pipelines` empty) and
//! no consumer can ever decode a frame — so the page reloaded every ~60s
//! forever (benign but wasteful).
//!
//! This module decides, given the server's pipeline state, whether the reload
//! can even help. The decision is split into small pure functions
//! (`should_reload_given_pipeline_state`, `healthz_body_has_streaming_pipeline`)
//! so the gate is unit-testable on host without a browser or a running server;
//! `fetch_healthz_has_streaming_pipeline` is the browser-only wiring. Split
//! into its own file to keep `ndi_watchdog.rs` under the size cap.

use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

/// LAST-RESORT reload gate (#410): given whether the SERVER currently has an
/// actively-streaming NDI pipeline, should the stuck stage page reload?
///
/// - `true` (server is streaming, but this consumer decoded nothing) → reload:
///   the page/consumer is stuck and a fresh WHEP negotiation + DOM can recover
///   it (the #401 case).
/// - `false` (no streaming pipeline) → DON'T reload: the source itself is down
///   (Resolume silent / `ndi_pipelines` empty), so reloading cannot produce
///   frames — it would just loop every ~60s. Keep waiting instead.
///
/// Pure + side-effect-free so the gate is unit-testable on host without a
/// browser or a running server. The fetch-failure case is handled by the
/// caller (it defaults to reloading), NOT here.
pub(crate) fn should_reload_given_pipeline_state(server_has_streaming_pipeline: bool) -> bool {
    server_has_streaming_pipeline
}

/// Parse a `/healthz` JSON body and decide whether the server has at least one
/// actively-streaming NDI pipeline. An entry counts as "streaming" when its
/// `state` is `"streaming"` or `"starting"` (about to deliver frames) — a
/// `"stopped"` / `"errored"` pipeline, or an empty/absent `ndi_pipelines`
/// array, means the source is NOT delivering media.
///
/// Pure over the response text so it is unit-testable on host. A body that
/// fails to parse returns `false` here — but callers treat a FAILED fetch
/// (network error, non-2xx) as "reload anyway", so a parse-of-garbage that
/// only happens on a successful-but-malformed response stays conservative
/// (no reload) rather than masking a real stuck consumer.
pub(crate) fn healthz_body_has_streaming_pipeline(body: &str) -> bool {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    let Some(pipelines) = json.get("ndi_pipelines").and_then(|v| v.as_array()) else {
        return false;
    };
    pipelines.iter().any(|p| {
        matches!(
            p.get("state").and_then(|s| s.as_str()),
            Some("streaming") | Some("starting")
        )
    })
}

/// Fetch `/healthz` and report whether the server has an actively-streaming NDI
/// pipeline. Returns `true` on ANY fetch/response failure so a transient
/// `/healthz` outage never SUPPRESSES a genuinely-needed last-resort reload
/// (#410 — additive guard, must not break #401 recovery).
pub(crate) async fn fetch_healthz_has_streaming_pipeline() -> bool {
    async fn try_fetch() -> Option<bool> {
        let window = leptos::web_sys::window()?;
        let resp_val = JsFuture::from(window.fetch_with_str("/healthz"))
            .await
            .ok()?;
        let resp: leptos::web_sys::Response = resp_val.dyn_into().ok()?;
        if !resp.ok() {
            return None;
        }
        let text = JsFuture::from(resp.text().ok()?).await.ok()?;
        let body = text.as_string()?;
        Some(healthz_body_has_streaming_pipeline(&body))
    }
    // Fetch/parse failure → default to TRUE (reload), preserving #401 behavior.
    try_fetch().await.unwrap_or(true)
}

/// LAST-RESORT escalation decision (#401): should the stage page perform a full
/// `window.location.reload()` because video has been dead for too long despite
/// the reconnect loop continuously retrying? `ms_since_last_decoded_frame` is
/// measured across the WHOLE page session, so the only way it grows past
/// `reload_threshold_ms` is a genuinely stuck stream reconnect alone has NOT
/// recovered. Pure + side-effect-free so it is unit-testable without a browser.
pub(crate) fn should_escalate_reload(
    ms_since_last_decoded_frame: f64,
    reload_threshold_ms: f64,
) -> bool {
    // Strictly greater-than so a tick landing exactly AT the threshold waits one
    // more tick (no off-by-one reload on the boundary).
    ms_since_last_decoded_frame > reload_threshold_ms
}

/// TEST-ONLY (#422): whether the page URL carries `?ndiReloadSkipHealthz=1`,
/// which makes the last-resort reload bypass the #410 `/healthz` streaming gate.
/// The E2E uses it to exercise the real `window.location.reload()` path
/// deterministically — a pipeline-kill cannot create the gate's
/// "server-streaming-but-this-consumer-stuck" precondition. Production stage
/// pages never set it, so prod always takes the full gated path. Read ONCE.
pub(crate) fn reload_skip_healthz_from_url() -> bool {
    leptos::web_sys::window()
        .and_then(|w| w.location().search().ok())
        .and_then(|search| {
            leptos::web_sys::UrlSearchParams::new_with_str(&search)
                .ok()
                .and_then(|p| p.get("ndiReloadSkipHealthz"))
        })
        .as_deref()
        == Some("1")
}

#[cfg(test)]
mod tests {
    use super::{healthz_body_has_streaming_pipeline, should_reload_given_pipeline_state};

    // ─────────────────────────────────────────────────────────────────────
    // #410 server-liveness reload gate.
    //
    // The #401 last-resort reload assumed reloading could recover any stuck
    // stream. But when the SOURCE itself is legitimately offline (Resolume
    // silent → the server has NO streaming pipeline, /healthz ndi_pipelines is
    // empty), no consumer can ever decode a frame, so the page reloaded every
    // ~60s forever — benign but wasteful. The gate distinguishes
    // "source legitimately down" (don't reload, keep waiting) from
    // "my page/consumer is stuck while the server IS streaming" (reload, the
    // #401 recovery). A failed /healthz fetch must NOT suppress a needed
    // reload, so the caller defaults to reloading on fetch error.
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn reload_when_server_has_a_streaming_pipeline_but_consumer_is_stuck() {
        // Server is streaming, but THIS page decoded nothing for the whole
        // window → the consumer/page is stuck → reload (the #401 recovery).
        assert!(should_reload_given_pipeline_state(true));
    }

    #[test]
    fn suppress_reload_when_server_has_no_streaming_pipeline() {
        // No streaming pipeline → the source itself is down → reloading cannot
        // help → DON'T reload, keep waiting.
        assert!(!should_reload_given_pipeline_state(false));
    }

    #[test]
    fn healthz_with_streaming_pipeline_means_server_is_streaming() {
        let body = r#"{"status":"ok","version":"0.4.138","channel":"dev",
            "ndi_pipelines":[{"source_id":"abc","state":"streaming"}]}"#;
        assert!(healthz_body_has_streaming_pipeline(body));
    }

    #[test]
    fn healthz_with_starting_pipeline_counts_as_streaming() {
        // A "starting" pipeline is about to deliver frames — treat it as live
        // so we still attempt the consumer-stuck reload.
        let body = r#"{"ndi_pipelines":[{"source_id":"abc","state":"starting"}]}"#;
        assert!(healthz_body_has_streaming_pipeline(body));
    }

    #[test]
    fn healthz_empty_pipelines_means_source_down() {
        // The exact #410 symptom: source offline → empty ndi_pipelines.
        let body = r#"{"status":"ok","ndi_pipelines":[]}"#;
        assert!(!healthz_body_has_streaming_pipeline(body));
    }

    #[test]
    fn healthz_only_errored_or_stopped_pipeline_means_source_down() {
        let body = r#"{"ndi_pipelines":[
            {"source_id":"a","state":"errored","last_error":"no source"},
            {"source_id":"b","state":"stopped"}]}"#;
        assert!(!healthz_body_has_streaming_pipeline(body));
    }

    #[test]
    fn healthz_missing_field_or_garbage_means_not_streaming() {
        // No ndi_pipelines key, or unparseable body → not streaming (the
        // SUCCESSFUL-but-malformed case; a FAILED fetch is the caller's
        // reload-anyway default, not this function's).
        assert!(!healthz_body_has_streaming_pipeline(r#"{"status":"ok"}"#));
        assert!(!healthz_body_has_streaming_pipeline("not json at all"));
    }
}
