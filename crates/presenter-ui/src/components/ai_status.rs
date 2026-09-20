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

/// The server (`router::ai_health::apply_last_completion_failure`) prefixes a
/// FOLD-WINDOW last-completion transport failure with this exact text (#764).
/// The UI distinguishes it from a probe/connectivity outage on this prefix so
/// an operator is told "posledné volanie zlyhalo" — not "AI nedostupná" — after
/// a single failed request while the backend is still reachable (#784). Kept in
/// sync by the `not_connected_*` tests below.
const LAST_CALL_FAILED_PREFIX: &str = "posledné AI volanie zlyhalo";

fn is_last_call_failed(error: &str) -> bool {
    error.starts_with(LAST_CALL_FAILED_PREFIX)
}

/// Which chip state the poll result maps to. `None` (checking) covers both
/// "the first poll hasn't answered yet" and "polling failed twice in a row" —
/// both genuinely unknown, never a guessed failure. A `connected:false` splits
/// two ways (#784): a fold-window LAST-CALL failure (backend reachable, one call
/// failed) vs a genuine probe/connectivity/budget outage.
pub(crate) fn ai_chip_state(status: Option<&AiStatusResponse>) -> &'static str {
    match status {
        None => "checking",
        Some(s) if !s.connected => {
            // #784: a fold-window last-completion failure must NOT alarm the
            // operator as a full outage — the backend is still reachable.
            if s.error.as_deref().is_some_and(is_last_call_failed) {
                "last_call_failed"
            } else {
                // A confirmed backend problem: unreachable endpoint, invalid
                // model id, or a metered backend that 200s `/models` but 403s
                // completions (exhausted budget / revoked key, #764).
                "unavailable"
            }
        }
        Some(_) => "ok",
    }
}

/// The chip's visible text — Slovak, matching the existing operator copy.
pub(crate) fn ai_chip_label(state: &str) -> &'static str {
    match state {
        "ok" => "AI: pripojené",
        // #784: one failed request while the backend is reachable — softer than
        // a full outage. The concrete excerpt rides in the tooltip.
        "last_call_failed" => "AI: posledné volanie zlyhalo",
        // A confirmed outage (budget/credit/key/model/connectivity) — the
        // concrete reason is in the tooltip (see `AiStatusChip`).
        "unavailable" => "AI nedostupná",
        _ => "AI: kontrolujem…",
    }
}

/// Dot color: green when connected, yellow while genuinely unknown OR after a
/// single last-call failure (attention, not a full outage — #784), red for a
/// confirmed outage.
pub(crate) fn ai_chip_dot(state: &str) -> &'static str {
    match state {
        "ok" => "green",
        "checking" | "last_call_failed" => "yellow",
        _ => "red",
    }
}

/// Tooltip text — names the state and tells the operator the chip is clickable
/// straight through to the AI panel.
pub(crate) fn ai_chip_tooltip(state: &str) -> &'static str {
    match state {
        "ok" => "AI je pripojená. Kliknutím otvoríš AI panel.",
        // #784: last-call failure — the concrete excerpt is appended by the
        // component when available (see `AiStatusChip`).
        "last_call_failed" => {
            "Posledné AI volanie zlyhalo, ale AI je dostupná. Kliknutím otvoríš AI panel."
        }
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
    // #764/#784: for a failure state, surface the concrete backend `error` in
    // the tooltip so the operator sees WHY. `unavailable` (a confirmed outage)
    // gets the "AI je nedostupná: …" framing; `last_call_failed` shows the
    // server's already-Slovak "posledné AI volanie zlyhalo: …" excerpt as-is.
    // Every other state keeps its fixed copy.
    let tooltip = move || {
        status.with(|s| {
            let st = ai_chip_state(s.as_ref());
            let concrete = s
                .as_ref()
                .and_then(|r| r.error.as_deref())
                .filter(|e| !e.is_empty());
            match (st, concrete) {
                ("unavailable", Some(err)) => {
                    format!("AI je nedostupná: {err}. Kliknutím otvoríš AI panel.")
                }
                ("last_call_failed", Some(err)) => {
                    format!("{err}. Kliknutím otvoríš AI panel.")
                }
                _ => ai_chip_tooltip(st).to_string(),
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
    fn not_connected_probe_outage_is_unavailable() {
        // A genuine probe/connectivity/budget outage (no fold prefix) → the
        // full-outage "AI nedostupná" state.
        let s = status(false, Some("AI proxy unreachable"));
        assert_eq!(ai_chip_state(Some(&s)), "unavailable");
    }

    #[test]
    fn not_connected_last_call_failure_is_its_own_state() {
        // #784: a fold-window last-completion transport failure (backend
        // reachable, one call failed) must be the softer `last_call_failed`
        // state — NOT the full-outage `unavailable` that says "AI down".
        let s = status(false, Some("posledné AI volanie zlyhalo: budget exceeded"));
        assert_eq!(ai_chip_state(Some(&s)), "last_call_failed");
    }

    #[test]
    fn labels_are_slovak_and_state_specific() {
        assert_eq!(ai_chip_label("ok"), "AI: pripojené");
        assert_eq!(ai_chip_label("unavailable"), "AI nedostupná");
        assert_eq!(
            ai_chip_label("last_call_failed"),
            "AI: posledné volanie zlyhalo"
        );
        assert_eq!(ai_chip_label("checking"), "AI: kontrolujem…");
    }

    #[test]
    fn dot_colors_by_state() {
        assert_eq!(ai_chip_dot("ok"), "green");
        assert_eq!(ai_chip_dot("checking"), "yellow");
        // #784: a single last-call failure is attention (yellow), not a full
        // outage (red).
        assert_eq!(ai_chip_dot("last_call_failed"), "yellow");
        assert_eq!(ai_chip_dot("unavailable"), "red");
    }

    #[test]
    fn tooltip_names_the_state_and_the_click_target() {
        assert!(ai_chip_tooltip("unavailable").contains("nedostupná"));
        assert!(ai_chip_tooltip("last_call_failed").contains("zlyhalo"));
        for state in ["ok", "unavailable", "last_call_failed"] {
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
