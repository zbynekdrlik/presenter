//! Operator-header AI connection indicator (#598): mirrors the Resolume
//! connection chip (`components/resolume_status.rs`, #564) so a logged-out
//! or otherwise broken AI proxy is visible up front instead of only being
//! discovered when a live verse request silently fails to arrive mid-event
//! (2026-07-26).
//!
//! Unlike the Resolume chips (one per host, identity in the label, severity
//! in the dot), there is exactly one AI proxy — so the label itself carries
//! the state, computed from the three NESTED `proxy.*` booleans
//! (`AiStatusResponse::proxy`), never the flat top-level `connected` field.
//! #597 already ANDs those three into `connected`, but a single bool still
//! can't say WHICH check failed, which is the whole point of this chip.
//!
//! Placement (#573): mounted in the operator header's TOP brand row, inside
//! `.operator__brand-nav`, immediately after `<ResolumeStatusChips />` —
//! connection/status indicators belong next to the surface-nav pills, never
//! next to the Stage Output select in `operator__header-right`.

use leptos::prelude::*;

use crate::api::ai::{check_status, AiStatusResponse};

const AI_STATUS_REFRESH_MS: u32 = 5_000;

/// How many CONSECUTIVE poll failures before the chip admits it does not
/// know, rather than clinging to a possibly long-stale last-known state.
/// Same threshold and reasoning as `pages/settings/video_sources.rs`'s
/// `STALE_AFTER_FAILURES` — a single failed poll is a blip (a page
/// navigation aborting an in-flight fetch is not an outage) and must not
/// flip the chip to a failure state or log anything.
pub(crate) const STALE_AFTER_FAILURES: u32 = 2;

pub(crate) fn is_stale(consecutive_failures: u32) -> bool {
    consecutive_failures >= STALE_AFTER_FAILURES
}

/// Which of the four states the chip is in right now. `None` covers both
/// "the first poll hasn't answered yet" and "polling has failed twice in a
/// row" — both are genuinely unknown, never a guessed failure state.
pub(crate) fn ai_chip_state(status: Option<&AiStatusResponse>) -> &'static str {
    match status {
        None => "checking",
        Some(s) if !s.proxy.binary_found => "missing-binary",
        Some(s) if !s.proxy.running => "proxy-down",
        Some(s) if !s.proxy.claude_authenticated => "logged-out",
        Some(_) => "ok",
    }
}

/// The chip's visible text — Slovak, matching the existing operator copy.
pub(crate) fn ai_chip_label(state: &str) -> &'static str {
    match state {
        "ok" => "AI: pripojené",
        "logged-out" => "AI: odhlásené",
        "proxy-down" => "AI: proxy nebeží",
        "missing-binary" => "AI: chýba binárka",
        _ => "AI: kontrolujem…",
    }
}

/// Dot color: green only when everything checks out, yellow while genuinely
/// unknown, red for any of the three confirmed problems.
pub(crate) fn ai_chip_dot(state: &str) -> &'static str {
    match state {
        "ok" => "green",
        "checking" => "yellow",
        _ => "red",
    }
}

/// Tooltip text — names the exact problem and tells the operator the chip
/// is clickable straight through to the AI panel where it's fixed.
pub(crate) fn ai_chip_tooltip(state: &str) -> &'static str {
    match state {
        "ok" => "AI je pripojená a prihlásená. Kliknutím otvoríš AI panel.",
        "logged-out" => {
            "AI proxy beží, ale nie je prihlásená ku Claude. Kliknutím otvoríš AI panel a prihlásiš sa."
        }
        "proxy-down" => "AI proxy nebeží. Kliknutím otvoríš AI panel.",
        "missing-binary" => "Na serveri chýba binárka AI proxy. Kliknutím otvoríš AI panel.",
        _ => "Zisťujem stav AI…",
    }
}

#[component]
pub fn AiStatusChip() -> impl IntoView {
    let status = RwSignal::new(None::<AiStatusResponse>);
    let poll_failures = RwSignal::new(0u32);

    let poll = move || {
        leptos::task::spawn_local(async move {
            match check_status().await {
                Ok(resp) => {
                    poll_failures.set(0);
                    status.set(Some(resp));
                }
                // One failure is a blip — never swallow the second, but
                // never scream at the first either (see `STALE_AFTER_FAILURES`).
                Err(err) => {
                    let failures = poll_failures.get_untracked() + 1;
                    poll_failures.set(failures);
                    if is_stale(failures) {
                        if !is_stale(failures - 1) {
                            leptos::logging::warn!(
                                "AI status poll failed {failures}x in a row — \
                                 showing the chip as unknown rather than stale: {err}"
                            );
                        }
                        status.set(None);
                    }
                }
            }
        });
    };
    poll();

    // `forget()` — the timer dies with page navigation (no client-side
    // router), same as every other operator-header/settings poller. Not
    // `on_cleanup`: `gloo_timers::Interval` is not `Send`, which the host
    // (non-wasm) `cargo test --lib` build of this crate requires.
    let interval = gloo_timers::callback::Interval::new(AI_STATUS_REFRESH_MS, move || {
        poll();
    });
    interval.forget();

    let state = move || status.with(|s| ai_chip_state(s.as_ref()));
    let dot_class = move || {
        format!(
            "operator__ai-dot operator__ai-dot--{}",
            ai_chip_dot(state())
        )
    };
    let label = move || ai_chip_label(state());
    let tooltip = move || ai_chip_tooltip(state());

    view! {
        <a
            class="operator__ai-chip"
            data-role="ai-status-chip"
            data-state=state
            title=tooltip
            href="/ui/operator/ai"
        >
            <span class=dot_class aria-hidden="true"></span>
            <span class="operator__ai-chip-label">{label}</span>
        </a>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ai::ProxyStatus;

    fn status(binary_found: bool, running: bool, claude_authenticated: bool) -> AiStatusResponse {
        AiStatusResponse {
            connected: binary_found && running && claude_authenticated,
            error: None,
            proxy: ProxyStatus {
                running,
                port: 18787,
                api_url: "http://127.0.0.1:18787/v1".to_string(),
                binary_found,
                claude_authenticated,
                token_expires_at: None,
            },
        }
    }

    #[test]
    fn no_status_yet_is_checking() {
        assert_eq!(ai_chip_state(None), "checking");
    }

    #[test]
    fn all_three_signals_true_is_ok() {
        let s = status(true, true, true);
        assert_eq!(ai_chip_state(Some(&s)), "ok");
    }

    #[test]
    fn not_authenticated_is_the_most_common_real_state() {
        let s = status(true, true, false);
        assert_eq!(ai_chip_state(Some(&s)), "logged-out");
    }

    #[test]
    fn proxy_not_running_is_reported_even_when_binary_found() {
        let s = status(true, false, true);
        assert_eq!(ai_chip_state(Some(&s)), "proxy-down");
    }

    #[test]
    fn missing_binary_wins_over_the_other_two_flags() {
        // running/claude_authenticated can't be meaningfully true if the
        // binary itself is missing, but prove the precedence anyway.
        let s = status(false, true, true);
        assert_eq!(ai_chip_state(Some(&s)), "missing-binary");
    }

    #[test]
    fn labels_are_slovak_and_state_specific() {
        assert_eq!(ai_chip_label("ok"), "AI: pripojené");
        assert_eq!(ai_chip_label("logged-out"), "AI: odhlásené");
        assert_eq!(ai_chip_label("proxy-down"), "AI: proxy nebeží");
        assert_eq!(ai_chip_label("missing-binary"), "AI: chýba binárka");
        assert_eq!(ai_chip_label("checking"), "AI: kontrolujem…");
    }

    #[test]
    fn only_ok_is_green_only_checking_is_yellow_the_rest_are_red() {
        assert_eq!(ai_chip_dot("ok"), "green");
        assert_eq!(ai_chip_dot("checking"), "yellow");
        assert_eq!(ai_chip_dot("logged-out"), "red");
        assert_eq!(ai_chip_dot("proxy-down"), "red");
        assert_eq!(ai_chip_dot("missing-binary"), "red");
    }

    #[test]
    fn tooltip_names_the_exact_problem_and_the_click_target() {
        assert!(ai_chip_tooltip("logged-out").contains("nie je prihlásená"));
        assert!(ai_chip_tooltip("proxy-down").contains("nebeží"));
        assert!(ai_chip_tooltip("missing-binary").contains("chýba"));
        for state in ["ok", "logged-out", "proxy-down", "missing-binary"] {
            assert!(ai_chip_tooltip(state).contains("AI panel"));
        }
    }

    #[test]
    fn one_failed_poll_is_a_blip_two_in_a_row_is_stale() {
        assert!(!is_stale(0));
        assert!(!is_stale(1));
        assert!(is_stale(2));
        assert!(is_stale(7));
    }
}
