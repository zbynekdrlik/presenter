//! Primary logged-out state for the AI panel (#599).
//!
//! Before this, a dead Claude login only showed up as a small "Claude: not
//! authenticated" line buried inside the AI panel's COLLAPSED settings
//! drawer — nothing pointed the operator at the existing login flow at
//! `/ui/operator/ai`. Real incident 2026-07-26: AI verses died right before
//! an event and were fixed via SSH instead of through the GUI.
//!
//! This banner is keyed ONLY on the observable `proxy.claudeAuthenticated`
//! (no TYPED `authentication_error` exists in the router — `authentication_error`
//! is only ever a free-text string an UPSTREAM call embeds in its failure
//! message, named as such in `router/ai.rs`'s `compute_ai_connected` doc
//! comment; there is nothing to pattern-match on client-side, so the UI keys
//! on `claudeAuthenticated == false` instead). It is ALWAYS mounted (never
//! conditionally rendered) so E2E tests assert on `data-visible`, never on
//! element presence — the same discipline as the toast component.
//!
//! `authenticated` is tri-state (`Option<bool>`, #622 post-merge review
//! finding 1): `None` means "no confirmed answer yet" (first status fetch
//! still in flight, or the last fetch failed) and must NEVER be treated as
//! "logged out" — before this, a failed fetch left the caller's plain `bool`
//! at its initial `false` default forever, painting a false accusation. Only
//! `Some(false)` — an actual, successful "not authenticated" response — shows
//! the logged-out content.
//!
//! The CTA reuses the EXISTING `proxy_login`/`proxy_complete_login` flow
//! already wired in `pages/ai.rs` (never duplicated here) — clicking it
//! calls back into `pages/ai.rs` via the `on_login` prop, which both starts
//! that flow and opens the settings drawer so the link/paste steps become
//! visible.

use leptos::prelude::*;

/// Whether the primary logged-out banner should be shown at all — ONLY on a
/// CONFIRMED `Some(false)`. `None` (unknown: no answer yet, or the last fetch
/// failed) must stay hidden — never accuse the operator of being logged out
/// on a guess (#622 post-merge review finding 1).
pub(crate) fn show_login_banner(authenticated: Option<bool>) -> bool {
    authenticated == Some(false)
}

/// Whether the "still valid, renew soon" note should be shown — only while
/// CONFIRMED authenticated (`Some(true)`) via a token whose expiry is
/// actually known (never for API-key auth, which carries no expiry at all,
/// and never while the auth state is merely unknown).
pub(crate) fn show_validity_note(authenticated: Option<bool>, expires_at: Option<&str>) -> bool {
    authenticated == Some(true) && expires_at.is_some()
}

/// Format an RFC3339 timestamp as `dd.mm.yyyy HH:MM` local time. Thin
/// wrapper over the shared `utils::timestamp::format_local_timestamp`
/// (#622 post-merge review finding 10 — this used to duplicate
/// `pages/settings::format_timestamp`'s parse/format/fallback body wholesale,
/// differing only in the strftime pattern).
pub(crate) fn format_expiry(value: &str) -> String {
    crate::utils::timestamp::format_local_timestamp(value, "%d.%m.%Y %H:%M")
}

/// Subtext under the CTA — names WHEN the last login died (when known) so
/// the operator isn't guessing whether this is a fresh install or a lapsed
/// renewal.
pub(crate) fn banner_subtext(expires_at: Option<&str>) -> String {
    match expires_at {
        Some(ts) => format!("Predchádzajúce prihlásenie vypršalo {}.", format_expiry(ts)),
        None => "Zatiaľ nie je prihlásený k Claude AI na tomto serveri.".to_string(),
    }
}

/// The "still valid" note text — sharpens to an explicit renew-soon warning
/// once inside the expiry window (#660: before this, the SAME flat line
/// rendered whether 8 hours or 8 minutes remained), otherwise the plain
/// "valid until" wording.
pub(crate) fn validity_text(expires_at: &str) -> String {
    if crate::components::ai_status::is_expiring_soon(expires_at, chrono::Utc::now()) {
        format!(
            "Prihlásenie ku Claude čoskoro vyprší ({}) — odporúčame sa znova prihlásiť.",
            format_expiry(expires_at)
        )
    } else {
        format!(
            "Prihlásenie ku Claude platí do {}.",
            format_expiry(expires_at)
        )
    }
}

#[component]
pub fn AiLoginBanner<F>(
    authenticated: RwSignal<Option<bool>>,
    token_expires_at: RwSignal<Option<String>>,
    on_login: F,
) -> impl IntoView
where
    F: Fn() + 'static,
{
    let visible = move || show_login_banner(authenticated.get()).to_string();
    let subtext = move || banner_subtext(token_expires_at.get().as_deref());
    let validity_visible = move || {
        show_validity_note(authenticated.get(), token_expires_at.get().as_deref()).to_string()
    };
    // #622 post-merge review finding 8: gate the validity-note TEXT NODE's
    // render on `show_validity_note` itself, not only on the wrapper's
    // CSS-hidden `data-visible` attribute — the wrapper div stays always
    // mounted (E2E asserts `data-visible`, same discipline as the login
    // banner above), but its text content is never computed while hidden.
    let validity_content = move || {
        let expires = token_expires_at.get();
        if show_validity_note(authenticated.get(), expires.as_deref()) {
            expires.map(|ts| validity_text(&ts))
        } else {
            None
        }
    };

    view! {
        <div class="ai-chat__login-status">
            <div
                class="ai-chat__login-banner"
                data-role="ai-login-banner"
                data-visible=visible
            >
                <p class="ai-chat__login-banner-title">"Nie si prihlásený ku Claude AI"</p>
                <p class="ai-chat__login-banner-subtext">{subtext}</p>
                <button
                    type="button"
                    class="ai-chat__btn ai-chat__btn--primary"
                    data-role="ai-login-cta"
                    on:click=move |_| on_login()
                >
                    "Prihlásiť sa"
                </button>
            </div>
            <div
                class="ai-chat__token-validity"
                data-role="ai-token-validity"
                data-visible=validity_visible
            >
                {validity_content}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_shown_only_on_confirmed_not_authenticated() {
        assert!(show_login_banner(Some(false)));
        assert!(!show_login_banner(Some(true)));
    }

    /// #622 post-merge review finding 1: `None` (no confirmed answer yet —
    /// first fetch still in flight, or the last fetch failed) must NEVER be
    /// treated as "logged out". Before the fix this state didn't exist —
    /// callers collapsed it onto `false`, which is exactly the false
    /// accusation this test guards against.
    #[test]
    fn banner_hidden_while_auth_state_is_unknown() {
        assert!(!show_login_banner(None));
    }

    #[test]
    fn validity_note_needs_both_confirmed_authenticated_and_a_known_expiry() {
        assert!(!show_validity_note(
            Some(false),
            Some("2026-08-05T10:00:00Z")
        ));
        assert!(!show_validity_note(Some(true), None));
        assert!(show_validity_note(Some(true), Some("2026-08-05T10:00:00Z")));
    }

    #[test]
    fn validity_note_hidden_while_auth_state_is_unknown_even_with_a_known_expiry() {
        assert!(!show_validity_note(None, Some("2026-08-05T10:00:00Z")));
    }

    #[test]
    fn expiry_is_formatted_as_local_date_time() {
        // A UTC timestamp with a known offset — assert only that it parses
        // into the expected calendar date, since the exact hour depends on
        // the CI runner's local timezone.
        let formatted = format_expiry("2026-08-05T10:00:00Z");
        assert!(formatted.contains("2026"), "got: {formatted}");
    }

    #[test]
    fn unparseable_timestamp_falls_back_to_the_raw_string() {
        assert_eq!(format_expiry("not-a-date"), "not-a-date");
    }

    #[test]
    fn subtext_names_the_expiry_when_known() {
        let text = banner_subtext(Some("2026-08-05T10:00:00Z"));
        assert!(text.contains("vypršalo"), "got: {text}");
    }

    #[test]
    fn subtext_is_generic_when_expiry_unknown() {
        let text = banner_subtext(None);
        assert!(!text.contains("vypršalo"), "got: {text}");
    }

    #[test]
    fn validity_text_names_the_expiry() {
        // Computed relative to "now" (not a hardcoded past date) so this
        // stays a genuine "plenty of time left" case regardless of when the
        // suite runs.
        let far_future = (chrono::Utc::now() + chrono::Duration::hours(8)).to_rfc3339();
        let text = validity_text(&far_future);
        assert!(text.contains("platí do"), "got: {text}");
        assert!(!text.contains("čoskoro"), "got: {text}");
    }

    // #660: the SAME flat "platí do" wording used to render whether 8 hours
    // or 8 minutes remained — this must now sharpen to an explicit warning
    // once inside the expiry window.
    #[test]
    fn validity_text_warns_explicitly_once_inside_the_expiry_window() {
        let soon = (chrono::Utc::now() + chrono::Duration::minutes(30)).to_rfc3339();
        let text = validity_text(&soon);
        assert!(text.contains("čoskoro vyprší"), "got: {text}");
        assert!(
            text.contains("odporúčame sa znova prihlásiť"),
            "got: {text}"
        );
    }
}
