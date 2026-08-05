//! Primary logged-out state for the AI panel (#599).
//!
//! Before this, a dead Claude login only showed up as a small "Claude: not
//! authenticated" line buried inside the AI panel's COLLAPSED settings
//! drawer — nothing pointed the operator at the existing login flow at
//! `/ui/operator/ai`. Real incident 2026-07-26: AI verses died right before
//! an event and were fixed via SSH instead of through the GUI.
//!
//! This banner is keyed ONLY on the observable `proxy.claudeAuthenticated`
//! (there is no `authentication_error` typed error anywhere in the code —
//! see issue #599 comment recording that design decision). It is ALWAYS
//! mounted (never conditionally rendered) so E2E tests assert on
//! `data-visible`, never on element presence — the same discipline as the
//! toast component.
//!
//! The CTA reuses the EXISTING `proxy_login`/`proxy_complete_login` flow
//! already wired in `pages/ai.rs` (never duplicated here) — clicking it
//! calls back into `pages/ai.rs` via the `on_login` prop, which both starts
//! that flow and opens the settings drawer so the link/paste steps become
//! visible.

use leptos::prelude::*;

/// Whether the primary logged-out banner should be shown at all.
pub(crate) fn show_login_banner(authenticated: bool) -> bool {
    !authenticated
}

/// Whether the "still valid, renew soon" note should be shown — only while
/// authenticated via a token whose expiry is actually known (never for
/// API-key auth, which carries no expiry at all).
pub(crate) fn show_validity_note(authenticated: bool, expires_at: Option<&str>) -> bool {
    authenticated && expires_at.is_some()
}

/// Format an RFC3339 timestamp as `dd.mm.yyyy HH:MM` local time, mirroring
/// `pages/settings::format_timestamp`. Falls back to the raw string when it
/// cannot be parsed — never hide the operator-relevant timestamp.
pub(crate) fn format_expiry(value: &str) -> String {
    use chrono::{DateTime, Local};
    match value.parse::<DateTime<chrono::Utc>>() {
        Ok(dt) => dt
            .with_timezone(&Local)
            .format("%d.%m.%Y %H:%M")
            .to_string(),
        Err(_) => value.to_string(),
    }
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

/// The "still valid, renew soon" note text.
pub(crate) fn validity_text(expires_at: &str) -> String {
    format!(
        "Prihlásenie ku Claude platí do {}.",
        format_expiry(expires_at)
    )
}

#[component]
pub fn AiLoginBanner<F>(
    authenticated: RwSignal<bool>,
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
    let validity = move || {
        token_expires_at
            .get()
            .map(|ts| validity_text(&ts))
            .unwrap_or_default()
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
                {validity}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_shown_only_when_not_authenticated() {
        assert!(show_login_banner(false));
        assert!(!show_login_banner(true));
    }

    #[test]
    fn validity_note_needs_both_authenticated_and_a_known_expiry() {
        assert!(!show_validity_note(false, Some("2026-08-05T10:00:00Z")));
        assert!(!show_validity_note(true, None));
        assert!(show_validity_note(true, Some("2026-08-05T10:00:00Z")));
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
        let text = validity_text("2026-08-05T10:00:00Z");
        assert!(text.contains("platí do"), "got: {text}");
    }
}
