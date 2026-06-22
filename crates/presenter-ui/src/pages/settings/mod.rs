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

pub(super) fn parse_port(raw: &str, fallback: u16) -> u16 {
    raw.trim().parse::<u16>().unwrap_or(fallback)
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
#[component]
pub fn SettingsPage() -> impl IntoView {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "settings");
        let _ = body.set_attribute("data-mode", "create");
    }

    // ── Toast state (shared across every card) ──────────────────────────────
    let toast = ToastHandle {
        message: RwSignal::new(String::new()),
        visible: RwSignal::new(false),
        state: RwSignal::new(String::from("info")),
    };

    view! {
        <div class="settings-layout">
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
