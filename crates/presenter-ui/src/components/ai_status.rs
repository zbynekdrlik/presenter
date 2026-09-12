//! Operator-header AI connection indicator (#598): mirrors the Resolume
//! connection chip (`components/resolume_status.rs`, #564) so a broken AI
//! backend is visible up front instead of only being discovered when a live
//! verse request silently fails to arrive mid-event (2026-07-26).
//!
//! Since #762 removed the bundled CLIProxyAPI proxy + Claude OAuth, the chip is
//! backend-agnostic: it reads only the flat `connected`/`error` fields of
//! `/ai/status` (there is no nested `proxy` object and no Claude login state).
//! Three states: `ok` (connected), `unavailable` (a confirmed backend problem —
//! unreachable, invalid model, exhausted budget/credit/key, #764), and
//! `checking` (no confirmed answer yet, or polling failed twice in a row).
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

/// Which of the three states the chip is in right now. `None` covers both
/// "the first poll hasn't answered yet" and "polling has failed twice in a
/// row" — both are genuinely unknown, never a guessed failure state.
pub(crate) fn ai_chip_state(status: Option<&AiStatusResponse>) -> &'static str {
    match status {
        None => "checking",
        // A confirmed backend problem carried by the flat `connected` field:
        // unreachable endpoint, invalid model id, or a metered backend that
        // 200s `/models` but 403s completions (exhausted budget / revoked key,
        // #764). The concrete reason rides in `error` (see the tooltip).
        Some(s) if !s.connected => "unavailable",
        Some(_) => "ok",
    }
}

/// The chip's visible text — Slovak, matching the existing operator copy.
pub(crate) fn ai_chip_label(state: &str) -> &'static str {
    match state {
        "ok" => "AI: pripojené",
        // The concrete reason (budget/credit/key/model/connectivity) is in the
        // tooltip (see `AiStatusChip`).
        "unavailable" => "AI: nedostupné",
        _ => "AI: kontrolujem…",
    }
}

/// Dot color: green when connected, yellow while genuinely unknown, red for a
/// confirmed problem.
pub(crate) fn ai_chip_dot(state: &str) -> &'static str {
    match state {
        "ok" => "green",
        "checking" => "yellow",
        _ => "red",
    }
}

/// Tooltip text — names the state and tells the operator the chip is clickable
/// straight through to the AI panel.
pub(crate) fn ai_chip_tooltip(state: &str) -> &'static str {
    match state {
        "ok" => "AI je pripojená. Kliknutím otvoríš AI panel.",
        // Generic fallback when the concrete `error` string is not available
        // (the component appends it when it is — see `AiStatusChip`).
        "unavailable" => "AI je nedostupná. Kliknutím otvoríš AI panel.",
        _ => "Zisťujem stav AI…",
    }
}

#[component]
pub fn AiStatusChip() -> impl IntoView {
    let status = RwSignal::new(None::<AiStatusResponse>);
    let poll_failures = RwSignal::new(0u32);
    // #622 post-merge review finding 3(c): an in-flight guard (never start a
    // new poll while one is still awaiting a response — no pile-up) plus a
    // monotonic sequence counter (a response that is no longer the LATEST
    // issued poll can never apply its data — defense in depth if a call ever
    // races the guard, e.g. a future manual "check now" trigger).
    let in_flight = RwSignal::new(false);
    let poll_seq = RwSignal::new(0u64);

    let poll = move || {
        if in_flight.get_untracked() {
            return;
        }
        let seq = poll_seq.get_untracked() + 1;
        poll_seq.set(seq);
        in_flight.set(true);
        leptos::task::spawn_local(async move {
            let result = check_status().await;
            if poll_seq.get_untracked() == seq {
                match result {
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
                                // #622 post-merge review finding 4: this used
                                // to be `warn!`, which fires a console.warn on
                                // every genuine 2nd-consecutive-failure — the
                                // E2E zero-console assertion (rightly) treats
                                // that as a bug. `log!` (console.log) is not
                                // collected by the zero-console helper and
                                // this is still fully visible in devtools.
                                leptos::logging::log!(
                                    "AI status poll failed {failures}x in a row — \
                                     showing the chip as unknown rather than stale: {err}"
                                );
                            }
                            status.set(None);
                        }
                    }
                }
            }
            in_flight.set(false);
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
    // #764: for the `unavailable` state, surface the concrete backend `error`
    // (budget/credit/key/model/connectivity) in the tooltip so the operator
    // sees WHY, not just that AI is down; every other state keeps its fixed copy.
    let tooltip = move || {
        status.with(|s| {
            let st = ai_chip_state(s.as_ref());
            if st == "unavailable" {
                match s.as_ref().and_then(|r| r.error.as_deref()) {
                    Some(err) if !err.is_empty() => {
                        format!("AI je nedostupná: {err}. Kliknutím otvoríš AI panel.")
                    }
                    _ => ai_chip_tooltip(st).to_string(),
                }
            } else {
                ai_chip_tooltip(st).to_string()
            }
        })
    };

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

    /// A `/ai/status` response with the given flat `connected`/`error` — the
    /// backend-agnostic shape since #762 (no `proxy`, no `requiresClaudeAuth`).
    fn status(connected: bool, error: Option<&str>) -> AiStatusResponse {
        AiStatusResponse {
            connected,
            error: error.map(str::to_string),
            model_valid: true,
        }
    }

    #[test]
    fn no_status_yet_is_checking() {
        assert_eq!(ai_chip_state(None), "checking");
    }

    #[test]
    fn connected_is_ok() {
        let s = status(true, None);
        assert_eq!(ai_chip_state(Some(&s)), "ok");
    }

    #[test]
    fn not_connected_is_unavailable() {
        // #764: a metered OpenRouter backend that 200s /models but 403s
        // completions (exhausted budget) reports connected:false — the chip
        // must surface that generically, not show "AI: pripojené".
        let s = status(false, Some("posledné AI volanie zlyhalo: budget exceeded"));
        assert_eq!(ai_chip_state(Some(&s)), "unavailable");
    }

    #[test]
    fn labels_are_slovak_and_state_specific() {
        assert_eq!(ai_chip_label("ok"), "AI: pripojené");
        assert_eq!(ai_chip_label("unavailable"), "AI: nedostupné");
        assert_eq!(ai_chip_label("checking"), "AI: kontrolujem…");
    }

    #[test]
    fn ok_is_green_checking_is_yellow_unavailable_is_red() {
        assert_eq!(ai_chip_dot("ok"), "green");
        assert_eq!(ai_chip_dot("checking"), "yellow");
        assert_eq!(ai_chip_dot("unavailable"), "red");
    }

    #[test]
    fn tooltip_names_the_state_and_the_click_target() {
        assert!(ai_chip_tooltip("unavailable").contains("nedostupná"));
        for state in ["ok", "unavailable"] {
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
