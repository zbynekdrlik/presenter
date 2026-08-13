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

/// Pure predicate: is `expires_at` (an RFC3339 timestamp) within the
/// warning window of `now`? An unparseable timestamp or an ALREADY-expired
/// one is never "expiring soon" — an already-dead token is the existing
/// `logged-out` state's job (`ai_chip_state` checks `claude_authenticated`
/// first), and an unparseable one has nothing useful to warn about.
///
/// #660 / #675 review finding 4: the window itself and the actual
/// inside-the-window arithmetic now live in `presenter_core::ai_auth`
/// (shared with `ai::refresh::check_and_warn` in presenter-server) instead
/// of being hand-duplicated here — this wrapper only handles the `&str`
/// parsing this call site's callers happen to carry (`AiStatusResponse`
/// deserializes `token_expires_at` as a raw RFC3339 string).
pub(crate) fn is_expiring_soon(expires_at: &str, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Ok(parsed) = expires_at.parse::<chrono::DateTime<chrono::Utc>>() else {
        return false;
    };
    presenter_core::is_expiring_soon(parsed, now, presenter_core::EXPIRY_WARNING_WINDOW)
}

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
        // #679: a Claude-auth problem is only real when the configured
        // apiUrl actually requires it — a non-bundled endpoint (the #662
        // local-LLM scenario) never needs a Claude login, so these two
        // branches are gated on `requires_claude_auth`. `missing-binary`/
        // `proxy-down` above stay unconditional — the bundled proxy
        // process's own liveness is unrelated to which endpoint is
        // currently configured.
        Some(s) if s.requires_claude_auth && !s.proxy.claude_authenticated => "logged-out",
        // #660: authenticated right now, but the token is about to die —
        // warn BEFORE it happens, not only after (the `logged-out` branch
        // above already covers "already dead").
        Some(s)
            if s.requires_claude_auth
                && s.proxy
                    .token_expires_at
                    .as_deref()
                    .is_some_and(|ts| is_expiring_soon(ts, chrono::Utc::now())) =>
        {
            "expiring-soon"
        }
        Some(_) => "ok",
    }
}

/// The chip's visible text — Slovak, matching the existing operator copy.
pub(crate) fn ai_chip_label(state: &str) -> &'static str {
    match state {
        "ok" => "AI: pripojené",
        "expiring-soon" => "AI: čoskoro treba prihlásiť",
        "logged-out" => "AI: odhlásené",
        "proxy-down" => "AI: proxy nebeží",
        "missing-binary" => "AI: chýba binárka",
        _ => "AI: kontrolujem…",
    }
}

/// Dot color: green only when everything checks out, yellow while genuinely
/// unknown OR still working-but-needs-attention-soon, red for a confirmed
/// problem.
pub(crate) fn ai_chip_dot(state: &str) -> &'static str {
    match state {
        "ok" => "green",
        "checking" | "expiring-soon" => "yellow",
        _ => "red",
    }
}

/// Tooltip text — names the exact problem and tells the operator the chip
/// is clickable straight through to the AI panel where it's fixed.
pub(crate) fn ai_chip_tooltip(state: &str) -> &'static str {
    match state {
        "ok" => "AI je pripojená a prihlásená. Kliknutím otvoríš AI panel.",
        "expiring-soon" => {
            "AI je pripojená, ale prihlásenie ku Claude čoskoro vyprší. Kliknutím otvoríš AI panel a znova sa prihlásiš."
        }
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
        status_with_expiry(binary_found, running, claude_authenticated, None)
    }

    fn status_with_expiry(
        binary_found: bool,
        running: bool,
        claude_authenticated: bool,
        token_expires_at: Option<&str>,
    ) -> AiStatusResponse {
        status_full(
            binary_found,
            running,
            claude_authenticated,
            token_expires_at,
            true,
        )
    }

    /// Full constructor, including `requires_claude_auth` (#679) — the other
    /// helpers above default it to `true` (the pre-#679 bundled-proxy
    /// behavior) so every existing test keeps its original meaning
    /// unchanged.
    fn status_full(
        binary_found: bool,
        running: bool,
        claude_authenticated: bool,
        token_expires_at: Option<&str>,
        requires_claude_auth: bool,
    ) -> AiStatusResponse {
        AiStatusResponse {
            connected: binary_found && running && (!requires_claude_auth || claude_authenticated),
            error: None,
            proxy: ProxyStatus {
                running,
                port: 18787,
                api_url: "http://127.0.0.1:18787/v1".to_string(),
                binary_found,
                claude_authenticated,
                token_expires_at: token_expires_at.map(str::to_string),
            },
            model_valid: true,
            requires_claude_auth,
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

    // #679: a non-bundled `apiUrl` (the #662 local-LLM scenario) never needs
    // a Claude login — `logged-out`/`expiring-soon` must never fire when
    // `requires_claude_auth` is false, even if `claude_authenticated` is
    // false or a stale expiry timestamp happens to be present.

    #[test]
    fn not_authenticated_is_ok_when_claude_auth_is_not_required() {
        let s = status_full(true, true, false, None, false);
        assert_eq!(ai_chip_state(Some(&s)), "ok");
    }

    #[test]
    fn expiring_soon_is_suppressed_when_claude_auth_is_not_required() {
        let now = chrono::Utc::now();
        let soon = (now + chrono::Duration::minutes(30)).to_rfc3339();
        let s = status_full(true, true, true, Some(&soon), false);
        assert_eq!(ai_chip_state(Some(&s)), "ok");
    }

    // #679 review finding 3: `missing-binary`/`proxy-down` are about the
    // BUNDLED proxy PROCESS itself — unrelated to which apiUrl is
    // currently configured — so they must stay unconditional even when
    // `requires_claude_auth` is false.

    #[test]
    fn missing_binary_still_reported_when_claude_auth_is_not_required() {
        let s = status_full(false, true, true, None, false);
        assert_eq!(ai_chip_state(Some(&s)), "missing-binary");
    }

    #[test]
    fn proxy_down_still_reported_when_claude_auth_is_not_required() {
        let s = status_full(true, false, true, None, false);
        assert_eq!(ai_chip_state(Some(&s)), "proxy-down");
    }

    // #660: authenticated but the token is about to expire — a NEW state
    // between "logged-out" (already dead) and "ok" (healthy, plenty of time
    // left). This is the whole point of the ticket: the operator must see
    // this BEFORE the token dies, not only after.

    #[test]
    fn expiring_soon_is_reported_when_token_dies_within_the_window() {
        let now = chrono::Utc::now();
        let soon = (now + chrono::Duration::minutes(30)).to_rfc3339();
        let s = status_with_expiry(true, true, true, Some(&soon));
        assert_eq!(ai_chip_state(Some(&s)), "expiring-soon");
    }

    #[test]
    fn ok_when_token_has_plenty_of_time_left() {
        let now = chrono::Utc::now();
        let plenty = (now + chrono::Duration::hours(8)).to_rfc3339();
        let s = status_with_expiry(true, true, true, Some(&plenty));
        assert_eq!(ai_chip_state(Some(&s)), "ok");
    }

    #[test]
    fn logged_out_wins_over_expiring_soon_when_already_dead() {
        // claude_authenticated=false must report "logged-out" regardless of
        // whatever stale expiry timestamp happens to still be present.
        let now = chrono::Utc::now();
        let past = (now - chrono::Duration::hours(1)).to_rfc3339();
        let s = status_with_expiry(true, true, false, Some(&past));
        assert_eq!(ai_chip_state(Some(&s)), "logged-out");
    }

    #[test]
    fn ok_when_no_expiry_is_known_at_all() {
        // API-key auth, or a token whose expiry couldn't be parsed
        // server-side — `token_expires_at: None` must never be treated as
        // "expiring soon".
        let s = status_with_expiry(true, true, true, None);
        assert_eq!(ai_chip_state(Some(&s)), "ok");
    }

    #[test]
    fn is_expiring_soon_true_inside_the_window() {
        let now = chrono::Utc::now();
        let soon = (now + chrono::Duration::minutes(30)).to_rfc3339();
        assert!(is_expiring_soon(&soon, now));
    }

    #[test]
    fn is_expiring_soon_false_well_outside_the_window() {
        let now = chrono::Utc::now();
        let far = (now + chrono::Duration::hours(8)).to_rfc3339();
        assert!(!is_expiring_soon(&far, now));
    }

    #[test]
    fn is_expiring_soon_false_once_already_expired() {
        let now = chrono::Utc::now();
        let past = (now - chrono::Duration::minutes(5)).to_rfc3339();
        assert!(!is_expiring_soon(&past, now));
    }

    #[test]
    fn is_expiring_soon_false_for_an_unparseable_timestamp() {
        let now = chrono::Utc::now();
        assert!(!is_expiring_soon("not-a-date", now));
    }

    #[test]
    fn labels_are_slovak_and_state_specific() {
        assert_eq!(ai_chip_label("ok"), "AI: pripojené");
        assert_eq!(
            ai_chip_label("expiring-soon"),
            "AI: čoskoro treba prihlásiť"
        );
        assert_eq!(ai_chip_label("logged-out"), "AI: odhlásené");
        assert_eq!(ai_chip_label("proxy-down"), "AI: proxy nebeží");
        assert_eq!(ai_chip_label("missing-binary"), "AI: chýba binárka");
        assert_eq!(ai_chip_label("checking"), "AI: kontrolujem…");
    }

    #[test]
    fn ok_is_green_checking_and_expiring_soon_are_yellow_the_rest_are_red() {
        assert_eq!(ai_chip_dot("ok"), "green");
        assert_eq!(ai_chip_dot("checking"), "yellow");
        assert_eq!(ai_chip_dot("expiring-soon"), "yellow");
        assert_eq!(ai_chip_dot("logged-out"), "red");
        assert_eq!(ai_chip_dot("proxy-down"), "red");
        assert_eq!(ai_chip_dot("missing-binary"), "red");
    }

    #[test]
    fn tooltip_names_the_exact_problem_and_the_click_target() {
        assert!(ai_chip_tooltip("logged-out").contains("nie je prihlásená"));
        assert!(ai_chip_tooltip("expiring-soon").contains("čoskoro vyprší"));
        assert!(ai_chip_tooltip("proxy-down").contains("nebeží"));
        assert!(ai_chip_tooltip("missing-binary").contains("chýba"));
        for state in [
            "ok",
            "expiring-soon",
            "logged-out",
            "proxy-down",
            "missing-binary",
        ] {
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
