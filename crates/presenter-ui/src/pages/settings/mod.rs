//! Settings page (`/ui/settings`) — Leptos migration of the former 1307-line
//! `settings_script.js` blob (#347).
//!
//! Reproduces every card the vanilla-JS page managed — Companion (feature
//! flags), operator Preferences (line-limit pref persisted in localStorage,
//! #272), Resolume hosts, Android stage launchers, Ableton/OSC, and NDI video
//! sources — using Leptos signals + `on:*` handlers + the `api::settings` /
//! `api::ndi` client fns. A 5s interval polls live status (matching the old
//! `STATUS_REFRESH_MS`), and a toast (the original `settings__toast` element)
//! gives feedback. The server `/integrations/*` + `/settings/features`
//! contracts are unchanged.

use leptos::prelude::*;

use crate::components::version_label::VersionLabel;

mod ableton;
mod android;
mod companion;
mod preferences;
mod resolume;
mod video_sources;

use ableton::AbletonCard;
use android::AndroidCard;
use companion::CompanionCard;
use preferences::PreferencesCard;
use resolume::ResolumeCard;
use video_sources::VideoSourcesCard;

/// How often live status (resolume/android/osc/ableset) is refreshed, matching
/// the former `STATUS_REFRESH_MS` constant in the JS blob.
pub(super) const STATUS_REFRESH_MS: u32 = 5_000;
/// Toast auto-hide delay, matching the JS blob's 4200ms.
const TOAST_HIDE_MS: u32 = 4_200;

/// Parse a port from form input, rejecting anything outside 1..=65535.
///
/// Returns `None` for empty / non-numeric / out-of-range input so the caller
/// shows "Port must be between 1 and 65535." — matching the original JS, which
/// validated `port < 1 || port > 65535` and threw. A naive `parse::<u16>()`
/// would make a too-large value (e.g. 99999) *overflow-fail* and silently fall
/// back to a default port, saving the wrong host (the bug this replaces). We
/// parse as `u32` first so an over-65535 value is rejected, not truncated.
pub(super) fn parse_port_in_range(raw: &str) -> Option<u16> {
    match raw.trim().parse::<u32>() {
        Ok(n) if (1..=65535).contains(&n) => Some(n as u16),
        _ => None,
    }
}

/// Format an RFC3339 timestamp string as `dd.mm.yyyy HH:MM:SS` in local time,
/// matching the old `Intl.DateTimeFormat('sk-SK', …)` output. Returns the raw
/// string unchanged when it cannot be parsed.
pub(super) fn format_timestamp(value: &str) -> String {
    use chrono::{DateTime, Local};
    match value.parse::<DateTime<chrono::Utc>>() {
        Ok(dt) => dt
            .with_timezone(&Local)
            .format("%d.%m.%Y %H:%M:%S")
            .to_string(),
        Err(_) => value.to_string(),
    }
}

pub(super) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Shared toast handle threaded into every card. `RwSignal` is `Copy + Send +
/// Sync`, so this passes cleanly into Leptos `For`/event closures (a plain
/// closure prop is neither `Send` nor `Sync`). Mirrors the old `showToast`
/// helper: shows a message + variant, then auto-hides after `TOAST_HIDE_MS`.
#[derive(Clone, Copy)]
pub(super) struct ToastHandle {
    message: RwSignal<String>,
    visible: RwSignal<bool>,
    state: RwSignal<String>,
}

impl ToastHandle {
    pub(super) fn show(self, msg: &str, variant: &str) {
        self.state.set(variant.to_string());
        self.message.set(msg.to_string());
        self.visible.set(true);
        let visible = self.visible;
        gloo_timers::callback::Timeout::new(TOAST_HIDE_MS, move || {
            visible.set(false);
        })
        .forget();
    }
}

/// Settings page — configuration for all integrations.
///
/// Renders in two contexts (#462):
/// - **Standalone** `/ui/settings` (`embedded = false`): owns the `<body>` for
///   full-page dark styling and shows its own page header.
/// - **Embedded** as a native operator panel (`embedded = true`): the operator
///   shell already provides the nav + version chrome, so the page header is
///   skipped and the shared `<body>` class is left untouched. This replaces the
///   former `<iframe src="/ui/settings">`, which double-rendered the header and
///   clipped scrolling to the fixed iframe height.
#[component]
pub fn SettingsPage(#[prop(optional)] embedded: bool) -> impl IntoView {
    // Standalone page only: own the <body> for full-page styling, and restore it
    // on unmount so an in-app navigation can't leave a stale `.settings` class
    // behind (a contributor to the "header multiplied on each visit" report).
    if !embedded {
        if let Some(body) = crate::utils::window::document_body() {
            let _ = body.set_attribute("class", "settings");
            let _ = body.set_attribute("data-mode", "create");
        }
        on_cleanup(|| {
            if let Some(body) = crate::utils::window::document_body() {
                let _ = body.set_attribute("class", "");
                let _ = body.remove_attribute("data-mode");
            }
        });
    }

    // ── Toast state (shared across every card) ──────────────────────────────
    let toast = ToastHandle {
        message: RwSignal::new(String::new()),
        visible: RwSignal::new(false),
        state: RwSignal::new(String::from("info")),
    };

    view! {
        <div class="settings-layout">
            {(!embedded).then(|| view! {
                <header class="settings__header">
                    <div class="settings__header-title">
                        <h1>"Presenter Settings"</h1>
                        <p>"Configure integrations and controller connections."</p>
                    </div>
                    <nav class="settings__header-nav">
                        <a href="/" class="settings__link">"← Back to hub"</a>
                        <span class="settings__version"><VersionLabel /></span>
                    </nav>
                </header>
            })}
            <main class="settings__main">
                <CompanionCard />
                <PreferencesCard />
                <ResolumeCard toast=toast />
                <AndroidCard toast=toast />
                <AbletonCard toast=toast />
                <VideoSourcesCard toast=toast />
            </main>
            <div
                class="settings__toast"
                data-role="toast"
                data-visible=move || if toast.visible.get() { "true" } else { "false" }
                data-state=move || toast.state.get()
            >
                {move || toast.message.get()}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::parse_port_in_range;

    // #455: out-of-range / non-numeric / empty input must return None so the
    // caller shows "Port must be between 1 and 65535." instead of silently
    // falling back to a default port. These cases also pin the boundary so the
    // mutation gate cannot survive a flipped comparison on `1..=65535`.

    #[test]
    fn zero_is_rejected() {
        // Lower-boundary-minus-one: 0 is not a valid port.
        assert_eq!(parse_port_in_range("0"), None);
    }

    #[test]
    fn one_is_accepted() {
        // Lower boundary: smallest valid port.
        assert_eq!(parse_port_in_range("1"), Some(1));
    }

    #[test]
    fn max_port_is_accepted() {
        // Upper boundary: largest valid port.
        assert_eq!(parse_port_in_range("65535"), Some(65535));
    }

    #[test]
    fn just_over_max_is_rejected() {
        // Upper-boundary-plus-one: this is exactly the value a naive
        // `parse::<u16>()` would overflow-fail on — must be rejected, not
        // truncated to a wrapped u16.
        assert_eq!(parse_port_in_range("65536"), None);
    }

    #[test]
    fn the_99999_bug_value_is_rejected() {
        // The concrete value from the #455 report.
        assert_eq!(parse_port_in_range("99999"), None);
    }

    #[test]
    fn typical_default_ports_round_trip() {
        // The fallbacks the call sites used must still parse cleanly.
        assert_eq!(parse_port_in_range("8090"), Some(8090));
        assert_eq!(parse_port_in_range("5555"), Some(5555));
        assert_eq!(parse_port_in_range("39051"), Some(39051));
    }

    #[test]
    fn whitespace_is_trimmed() {
        assert_eq!(parse_port_in_range("  443  "), Some(443));
    }

    #[test]
    fn empty_and_non_numeric_are_rejected() {
        assert_eq!(parse_port_in_range(""), None);
        assert_eq!(parse_port_in_range("   "), None);
        assert_eq!(parse_port_in_range("abc"), None);
        assert_eq!(parse_port_in_range("80x"), None);
        assert_eq!(parse_port_in_range("-1"), None);
    }
}
