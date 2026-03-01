use crate::{
    ableset::AbleSetStatusSnapshot,
    android_stage::AndroidStageDisplayStatusSnapshot,
    osc::OscStatusSnapshot,
    resolume::{ResolumeConnectionSnapshot, ResolumeConnectionState},
    state::{AppState, FeatureFlags},
};
use axum::response::Html;
use chrono::{DateTime, Local, Utc};
use leptos::prelude::*;
use presenter_core::{AbleSetSettings, OscSettings};
use reactive_graph::owner::Owner;
use serde::Serialize;
use serde_json::{json, to_string};
use std::sync::Arc;

use super::scripts;
use super::styles;
use super::utils::{escape_script_tag, json_safe};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsHostRow {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub is_enabled: bool,
    pub created_at: String,
    pub created_at_display: String,
    pub updated_at: String,
    pub updated_at_display: String,
    pub status_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ResolumeConnectionSnapshot>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsAndroidDisplayRow {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub launch_component: String,
    pub is_enabled: bool,
    pub created_at: String,
    pub created_at_display: String,
    pub updated_at: String,
    pub updated_at_display: String,
    pub status_state: String,
    pub last_attempt_display: String,
    pub last_success_display: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AndroidStageDisplayStatusSnapshot>,
}

#[component]
fn SettingsDocument(
    hosts: Vec<SettingsHostRow>,
    android_displays: Vec<SettingsAndroidDisplayRow>,
    osc_settings: OscSettings,
    osc_status: OscStatusSnapshot,
    ableset_settings: AbleSetSettings,
    ableset_status: AbleSetStatusSnapshot,
    features: FeatureFlags,
    script: String,
) -> impl IntoView {
    let hosts = Arc::new(hosts);
    let host_count_text = hosts.len().to_string();
    let android_displays = Arc::new(android_displays);
    let android_count_text = android_displays.len().to_string();
    let companion_enabled = features.companion_enabled;
    let companion_port_text = features.companion_port.to_string();
    let osc_port_value = osc_settings.listen_port.to_string();
    let osc_status_state = if !osc_status.enabled {
        "disabled".to_string()
    } else if osc_status.listening {
        "listening".to_string()
    } else {
        "enabled".to_string()
    };
    let osc_status_label = format!(
        "{}{}",
        osc_status_state
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
            .unwrap_or_else(String::new),
        osc_status_state.chars().skip(1).collect::<String>()
    );
    let osc_last_message_display = osc_status
        .last_message_at
        .map(format_settings_timestamp)
        .unwrap_or_else(|| "\u{2014}".to_string());
    let osc_last_note_display = osc_status
        .last_note
        .map(|note| {
            if let Some(velocity) = osc_status.last_velocity {
                format!("note {note} (vel {velocity})")
            } else {
                format!("note {note}")
            }
        })
        .unwrap_or_else(|| "\u{2014}".to_string());
    let osc_last_error = osc_status.last_error.clone();
    let ableset_host_value = ableset_settings.host.clone();
    let ableset_http_port_value = ableset_settings.http_port.to_string();
    let ableset_library_value = ableset_settings.library_name.clone();
    let ableset_enabled = ableset_settings.enabled;
    let ableset_last_song_name = ableset_status
        .last_song
        .as_ref()
        .map(|song| song.name.clone())
        .unwrap_or_else(|| "\u{2014}".to_string());
    let ableset_last_song_seen = ableset_status
        .last_song
        .as_ref()
        .and_then(|song| song.last_seen_at)
        .map(format_settings_timestamp)
        .unwrap_or_else(|| "\u{2014}".to_string());
    let ableset_status_state = if !ableset_status.enabled {
        "disabled"
    } else if ableset_status.tracking {
        "tracking"
    } else {
        "enabled"
    };
    let ableset_status_label = format!(
        "{}{}",
        ableset_status_state
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
            .unwrap_or_else(String::new),
        ableset_status_state.chars().skip(1).collect::<String>()
    );
    let ableset_last_error = ableset_status.last_error.clone();

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>"Presenter Settings"</title>
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <style>{styles::SETTINGS}</style>
            </head>
            <body class="settings" data-mode="create">
                <script>"if(window!==window.parent)document.body.classList.add('in-iframe');"</script>
                <header class="settings__header">
                    <div class="settings__header-title">
                        <h1>"Presenter Settings"</h1>
                        <p>"Configure integrations and controller connections."</p>
                    </div>
                    <nav class="settings__header-nav">
                        <a href="/" class="settings__link">"← Back to hub"</a>
                        <a href="/ui/stage-design" class="settings__link">"Stage Design →"</a>
                    </nav>
                </header>
                <main class="settings__main">
                    <section class="settings__card settings__card--feature">
                        <header class="settings__card-header">
                            <div>
                                <h2>"Companion"</h2>
                            </div>
                        </header>
                        <form class="settings__form settings__form--compact" data-role="feature-companion-form" autocomplete="off">
                            <div class="settings__form-row settings__form-row--compact settings__form-row--inline">
                                <label class="settings__form-checkbox settings__form-checkbox--inline">
                                    <input
                                        type="checkbox"
                                        data-role="feature-companion-toggle"
                                        checked={companion_enabled}
                                    />
                                    <span>"Enable"</span>
                                </label>
                                <label class="settings__form-control--tiny">
                                    <span>"Port"</span>
                                    <input
                                        type="number"
                                        min="1"
                                        max="65535"
                                        value={companion_port_text.clone()}
                                        data-role="feature-companion-port"
                                        required
                                    />
                                </label>
                                <button
                                    type="submit"
                                    class="settings__button settings__button--primary settings__button--compact"
                                    data-role="feature-submit"
                                >"Save"</button>
                            </div>
                            <p class="settings__form-status" data-role="feature-status" data-state="idle"></p>
                        </form>
                    </section>
                    <section class="settings__card">
                        <header class="settings__card-header">
                            <div>
                                <h2>"Resolume Arena Connections"</h2>
                                <p>
                                    "Define Resolume web servers Presenter should control."
                                </p>
                            </div>
                            <div class="settings__badge-group">
                                <span class="settings__badge" data-role="host-count">{host_count_text.clone()}</span>
                                <span class="settings__badge-label">"Hosts"</span>
                            </div>
                        </header>
                        <form class="settings__form" data-role="host-form" autocomplete="off">
                            <input type="hidden" data-role="host-id" />
                            <div class="settings__form-header">
                                <div>
                                    <h3 data-role="form-title">"Add Resolume Connection"</h3>
                                    <p data-role="form-subtitle">"Specify hostname, port, and availability."</p>
                                </div>
                            </div>
                            <div class="settings__form-row">
                                <label>
                                    <span>"Label"</span>
                                    <input
                                        type="text"
                                        name="label"
                                        data-role="host-label"
                                        placeholder="Main Arena"
                                        required
                                    />
                                </label>
                                <label>
                                    <span>"Hostname or DNS"</span>
                                    <input
                                        type="text"
                                        name="host"
                                        data-role="host-host"
                                        placeholder="resolume.lan"
                                        required
                                    />
                                </label>
                                <label class="settings__form-control--small">
                                    <span>"Port"</span>
                                    <input
                                        type="number"
                                        name="port"
                                        data-role="host-port"
                                        min="1"
                                        max="65535"
                                        value="8090"
                                        required
                                    />
                                </label>
                            </div>
                            <div class="settings__form-row settings__form-row--single">
                                <label class="settings__form-checkbox settings__form-checkbox--block">
                                    <input type="checkbox" name="isEnabled" data-role="host-enabled" checked />
                                    <span>"Enabled"</span>
                                </label>
                            </div>
                            <div class="settings__form-actions">
                                <button
                                    type="submit"
                                    class="settings__button settings__button--primary"
                                    data-role="host-submit"
                                >"Add Connection"</button>
                                <button
                                    type="button"
                                    class="settings__button settings__button--ghost"
                                    data-role="host-reset"
                                >"Cancel"</button>
                            </div>
                            <p class="settings__form-status" data-role="form-status" data-state="idle"></p>
                        </form>
                        <ul class="settings__list" data-role="resolume-host-list">
                            <Show
                                when={
                                    let hosts = Arc::clone(&hosts);
                                    move || !hosts.is_empty()
                                }
                                fallback={move || view! {
                                    <li class="settings__list-empty" data-role="host-empty">"No Resolume connections defined yet."</li>
                                }}
                            >
                                <For
                                    each={
                                        let hosts = Arc::clone(&hosts);
                                        move || (*hosts).clone()
                                    }
                                    key=|host: &SettingsHostRow| host.id.clone()
                                    children={|host: SettingsHostRow| {
                                        let raw_state = if host.status_state.is_empty() {
                                            "disabled".to_string()
                                        } else {
                                            host.status_state.to_lowercase()
                                        };
                                        let status_class =
                                            format!("settings__status settings__status--{}", raw_state);
                                        let status_label = format!(
                                            "{}{}",
                                            raw_state
                                                .chars()
                                                .next()
                                                .map(|c| c.to_uppercase().collect::<String>())
                                                .unwrap_or_else(String::new),
                                            raw_state.chars().skip(1).collect::<String>()
                                        );
                                        let latency_text = host
                                            .last_latency_ms
                                            .map(|ms| format!("{ms:.1} ms"))
                                            .unwrap_or_else(|| "—".to_string());
                                        let warning_text = host.status_message.clone().unwrap_or_default();
                                        let warning_view = (!warning_text.is_empty()).then(|| {
                                            view! { <p class="settings__list-meta settings__list-meta--warning">{format!("⚠ {warning_text}")}</p> }
                                        });
                                        let host_id_edit = host.id.clone();
                                        let host_id_delete = host.id.clone();
                                        view! {
                                            <li
                                                class="settings__list-item"
                                                data-id={host.id.clone()}
                                                data-enabled={host.is_enabled.to_string()}
                                            >
                                                <div class="settings__list-primary">
                                                    <div class="settings__list-title">
                                                        <span class="settings__host-label">{host.label.clone()}</span>
                                                        <span class={status_class}>{status_label.clone()}</span>
                                                    </div>
                                                    <p class="settings__list-line">
                                                        <code>{host.host.clone()}</code>
                                                        <span class="settings__host-port">{format!(":{}", host.port)}</span>
                                                    </p>
                                                    <p class="settings__list-meta">{"Updated "}{host.updated_at_display.clone()}</p>
                                                    <p class="settings__list-meta">{"Created "}{host.created_at_display.clone()}</p>
                                                    <p class="settings__list-meta">{"Latency "}{latency_text}</p>
                                                    {warning_view}
                                                </div>
                                                <div class="settings__list-actions">
                                                    <button
                                                        type="button"
                                                        class="settings__button settings__button--ghost"
                                                        data-role="host-edit"
                                                        data-id={host_id_edit}
                                                    >"Edit"</button>
                                                    <button
                                                        type="button"
                                                        class="settings__button settings__button--danger"
                                                        data-role="host-delete"
                                                        data-id={host_id_delete}
                                                    >"Delete"</button>
                                                </div>
                                            </li>
                                        }
                                    }}
                                />
                            </Show>
                        </ul>
                        <section class="settings__legend">
                            <h3>"Clip Tokens"</h3>
                            <p class="settings__legend-note">
                                "Presenter updates every clip whose name contains these tokens (for example, #main-a or #main-a-2) and alternates between A/B lanes so the next look is always preloaded."
                            </p>
                            <dl>
                                <div>
                                    <dt>"#main-a / #main-b"</dt>
                                    <dd>"Main lyric text, alternating between A and B for seamless cuts."</dd>
                                </div>
                                <div>
                                    <dt>"#translate-a / #translate-b"</dt>
                                    <dd>"Translation lyric text matched to each lane."</dd>
                                </div>
                                <div>
                                    <dt>"#bible-a / #bible-b"</dt>
                                    <dd>"Bible verse text with verse numbers (e.g. \"4. A bývalo...\")."</dd>
                                </div>
                                <div>
                                    <dt>"#bible-reference-a / #bible-reference-b"</dt>
                                    <dd>"Bible reference with translation code (e.g. \"1 Samuel 1:4-5 (ROH)\")."</dd>
                                </div>
                                <div>
                                    <dt>"#bible-translate-a / #bible-translate-b"</dt>
                                    <dd>"Secondary translation verse text with verse numbers, or empty if no secondary translation is configured."</dd>
                                </div>
                                <div>
                                    <dt>"#bible-translate-reference-a / #bible-translate-reference-b"</dt>
                                    <dd>"Secondary translation reference with its translation code."</dd>
                                </div>
                                <div>
                                    <dt>"#bible-clear"</dt>
                                    <dd>"Clears the Bible layer when triggered."</dd>
                                </div>
                                <div>
                                    <dt>"#song-name"</dt>
                                    <dd>"Displays the active song title (numeric prefixes like '001 ' are removed automatically)."</dd>
                                </div>
                                <div>
                                    <dt>"#band-name"</dt>
                                    <dd>"Displays the library/band the current song belongs to."</dd>
                                </div>
                                <div>
                                    <dt>"Suffixes: -u / -re"</dt>
                                    <dd>"Append -u to force uppercase and -re to collapse multi-line text into a single space-delimited line. Combine them (e.g., #translate-b-u-re) for stacked transforms."</dd>
                                </div>
                            </dl>
                        </section>
                    </section>
                    <section class="settings__card">
                        <header class="settings__card-header">
                            <div>
                                <h2>"Android Stage Launchers"</h2>
                                <p>"Keep each Android TV pinned to the Fully Kiosk stage display."</p>
                            </div>
                            <div class="settings__badge-group">
                                <span class="settings__badge" data-role="android-count">{android_count_text.clone()}</span>
                                <span class="settings__badge-label">"Displays"</span>
                            </div>
                        </header>
                        <form class="settings__form" data-role="android-form" autocomplete="off">
                            <input type="hidden" data-role="android-id" />
                            <div class="settings__form-header">
                                <div>
                                    <h3 data-role="android-form-title">"Add Android Stage Display"</h3>
                                    <p data-role="android-form-subtitle">"Presenter reconnects and relaunches Fully Kiosk whenever the device appears."</p>
                                </div>
                            </div>
                            <div class="settings__form-row">
                                <label>
                                    <span>"Label"</span>
                                    <input type="text" name="label" data-role="android-label" placeholder="Stage Left" required />
                                </label>
                                <label>
                                    <span>"Hostname or DNS"</span>
                                    <input type="text" name="host" data-role="android-host" placeholder="sd1l.lan" required />
                                </label>
                                <label class="settings__form-control--small">
                                    <span>"Port"</span>
                                    <input type="number" name="port" data-role="android-port" min="1" max="65535" value="5555" required />
                                </label>
                            </div>
                            <div class="settings__form-row settings__form-row--single">
                                <label>
                                    <span>"Launch Component"</span>
                                    <input
                                        type="text"
                                        name="launchComponent"
                                        data-role="android-component"
                                        placeholder="com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity"
                                        required
                                    />
                                </label>
                            </div>
                            <div class="settings__form-row settings__form-row--single">
                                <label class="settings__form-checkbox settings__form-checkbox--block">
                                    <input type="checkbox" name="isEnabled" data-role="android-enabled" checked />
                                    <span>"Enabled"</span>
                                </label>
                            </div>
                            <div class="settings__form-actions">
                                <button
                                    type="submit"
                                    class="settings__button settings__button--primary"
                                    data-role="android-submit"
                                >"Add Android Display"</button>
                                <button
                                    type="button"
                                    class="settings__button settings__button--ghost"
                                    data-role="android-reset"
                                >"Cancel"</button>
                            </div>
                            <p class="settings__form-status" data-role="android-form-status" data-state="idle"></p>
                        </form>
                        <ul class="settings__list" data-role="android-display-list">
                            <Show
                                when={
                                    let displays = Arc::clone(&android_displays);
                                    move || !displays.is_empty()
                                }
                                fallback={move || view! {
                                    <li class="settings__list-empty" data-role="android-empty">"No Android stage displays configured yet."</li>
                                }}
                            >
                                <For
                                    each={
                                        let displays = Arc::clone(&android_displays);
                                        move || (*displays).clone()
                                    }
                                    key=|display: &SettingsAndroidDisplayRow| display.id.clone()
                                    children={|display: SettingsAndroidDisplayRow| {
                                        let raw_state = if display.status_state.is_empty() {
                                            "disabled".to_string()
                                        } else {
                                            display.status_state.to_lowercase().replace(' ', "-")
                                        };
                                        let status_class =
                                            format!("settings__status settings__status--{}", raw_state);
                                        let status_label = display.status_state.clone();
                                        let warning_text = display.status_message.clone().unwrap_or_default();
                                        let warning_view = (!warning_text.is_empty()).then(|| {
                                            view! { <p class="settings__list-meta settings__list-meta--warning">{format!("⚠ {}", warning_text)}</p> }
                                        });
                                        let display_id_edit = display.id.clone();
                                        let display_id_delete = display.id.clone();
                                        view! {
                                            <li
                                                class="settings__list-item"
                                                data-id={display.id.clone()}
                                                data-enabled={display.is_enabled.to_string()}
                                            >
                                                <div class="settings__list-primary">
                                                    <div class="settings__list-title">
                                                        <span class="settings__host-label">{display.label.clone()}</span>
                                                        <span class={status_class}>{status_label}</span>
                                                    </div>
                                                    <p class="settings__list-line">
                                                        <code>{display.host.clone()}</code>
                                                        <span class="settings__host-port">{format!(":{}", display.port)}</span>
                                                    </p>
                                                    <p class="settings__list-meta">{"Component "}{display.launch_component.clone()}</p>
                                                    <p class="settings__list-meta">{"Last attempt "}{display.last_attempt_display.clone()}</p>
                                                    <p class="settings__list-meta">{"Last success "}{display.last_success_display.clone()}</p>
                                                    <p class="settings__list-meta">{"Updated "}{display.updated_at_display.clone()}</p>
                                                    <p class="settings__list-meta">{"Created "}{display.created_at_display.clone()}</p>
                                                    {warning_view}
                                                </div>
                                                <div class="settings__list-actions">
                                                    <button
                                                        type="button"
                                                        class="settings__button settings__button--ghost"
                                                        data-role="android-edit"
                                                        data-id={display_id_edit}
                                                    >"Edit"</button>
                                                    <button
                                                        type="button"
                                                        class="settings__button settings__button--danger"
                                                        data-role="android-delete"
                                                        data-id={display_id_delete}
                                                    >"Delete"</button>
                                                </div>
                                            </li>
                                        }
                                    }}
                                />
                            </Show>
                        </ul>
                    </section>
                    <section class="settings__card settings__card--ableton">
                        <header class="settings__card-header">
                            <div>
                                <h2>"Ableton Control"</h2>
                                <p>"Configure AbleSet tracking and Presenter's OSC listener."</p>
                            </div>
                        </header>
                        <form
                            class="settings__form settings__form--ableset"
                            data-role="ableset-form"
                            autocomplete="off"
                            data-mode={if ableset_enabled { "enabled" } else { "disabled" }}
                        >
                            <div class="settings__form-row settings__form-row--single">
                                <label class="settings__form-checkbox settings__form-checkbox--block">
                                    <input type="checkbox" data-role="ableset-enabled" checked={ableset_enabled} />
                                    <span>"Enable Ableton automation"</span>
                                </label>
                            </div>
                            <div class="settings__form-row">
                                <label>
                                    <span>"AbleSet Host"</span>
                                    <input
                                        type="text"
                                        data-role="ableset-host"
                                        value={ableset_host_value.clone()}
                                        required
                                    />
                                </label>
                                <label class="settings__form-control settings__form-control--small">
                                    <span>"HTTP Port"</span>
                                    <input
                                        type="number"
                                        data-role="ableset-http-port"
                                        min="1"
                                        max="65535"
                                        value={ableset_http_port_value.clone()}
                                        required
                                    />
                                </label>
                                <label>
                                    <span>"Library Name"</span>
                                    <input
                                        type="text"
                                        data-role="ableset-library"
                                        value={ableset_library_value.clone()}
                                        required
                                    />
                                </label>
                            </div>
                            <div class="settings__form-row settings__form-row--single">
                                <label class="settings__form-control settings__form-control--small">
                                    <span>"OSC Listener Port"</span>
                                    <input
                                        type="number"
                                        data-role="osc-port"
                                        min="1"
                                        max="65535"
                                        value={osc_port_value.clone()}
                                        required
                                    />
                                </label>
                            </div>
                            <div class="settings__form-actions">
                                <button
                                    type="submit"
                                    class="settings__button settings__button--primary"
                                    data-role="ableset-submit"
                                >"Save AbleSet Settings"</button>
                            </div>
                            <p class="settings__form-status" data-role="ableset-form-status" data-state="idle"></p>
                        </form>
                        <div class="settings__status-panel">
                            <span
                                class={format!("settings__status settings__status--{}", ableset_status_state)}
                                data-role="ableset-status-indicator"
                            >{ableset_status_label.clone()}</span>
                            <dl class="settings__status-list">
                                <div>
                                    <dt>"Current song"</dt>
                                    <dd data-role="ableset-status-song">{ableset_last_song_name.clone()}</dd>
                                </div>
                                <div>
                                    <dt>"Last update"</dt>
                                    <dd data-role="ableset-status-updated">{ableset_last_song_seen.clone()}</dd>
                                </div>
                            </dl>
                            <p class="settings__list-meta settings__list-meta--warning" data-role="ableset-status-error">
                                {ableset_last_error.clone().unwrap_or_default()}
                            </p>
                        </div>
                        <div class="settings__status-panel">
                            <span
                                class={format!("settings__status settings__status--{}", osc_status_state)}
                                data-role="osc-status-indicator"
                            >{osc_status_label.clone()}</span>
                            <dl class="settings__status-list">
                                <div>
                                    <dt>"Last event"</dt>
                                    <dd data-role="osc-status-last-message">{osc_last_message_display.clone()}</dd>
                                </div>
                                <div>
                                    <dt>"Last note"</dt>
                                    <dd data-role="osc-status-last-note">{osc_last_note_display.clone()}</dd>
                                </div>
                            </dl>
                            <p
                                class="settings__list-meta settings__list-meta--warning"
                                data-role="osc-status-error"
                                data-visible={if osc_last_error.is_some() { "true" } else { "false" }}
                            >{osc_last_error.clone().map(|err| format!("⚠ {err}")).unwrap_or_default()}</p>
                        </div>
                    </section>

                </main>
                <div class="settings__toast" data-role="toast" data-visible="false"></div>
                <script>{script}</script>
            </body>
        </html>
    }
}

pub async fn render_settings_ui(state: &AppState) -> anyhow::Result<Html<String>> {
    let hosts = state.list_resolume_hosts().await?;
    let statuses = state.resolume_status_snapshot().await;
    let android_displays = state.list_android_stage_displays().await?;
    let android_statuses = state.android_stage_status_snapshot().await;
    let osc_settings = state.osc_settings().await?;
    let osc_status = state.osc_status_snapshot().await;
    let ableset_settings = state.ableset_settings().await?;
    let ableset_status = state.ableset_status_snapshot().await;
    let feature_flags = state.feature_flags();

    let host_rows: Vec<SettingsHostRow> = hosts
        .into_iter()
        .map(|host| {
            let created_display = format_settings_timestamp(host.created_at);
            let updated_display = format_settings_timestamp(host.updated_at);
            let status = statuses
                .get(&host.id)
                .cloned()
                .unwrap_or_else(ResolumeConnectionSnapshot::disabled);
            let status_state = match status.state {
                ResolumeConnectionState::Disabled => "Disabled".to_string(),
                ResolumeConnectionState::Connecting => "Connecting".to_string(),
                ResolumeConnectionState::Connected => "Connected".to_string(),
                ResolumeConnectionState::Error => "Error".to_string(),
            };
            SettingsHostRow {
                id: host.id.to_string(),
                label: host.label,
                host: host.host,
                port: host.port,
                is_enabled: host.is_enabled,
                created_at: host.created_at.to_rfc3339(),
                created_at_display: created_display,
                updated_at: host.updated_at.to_rfc3339(),
                updated_at_display: updated_display,
                status_state,
                status_message: status.last_error.clone(),
                last_latency_ms: status.last_latency_ms,
                status: Some(status),
            }
        })
        .collect();

    let android_rows: Vec<SettingsAndroidDisplayRow> = android_displays
        .into_iter()
        .map(|display| {
            let status = android_statuses
                .get(&display.id)
                .cloned()
                .unwrap_or_else(AndroidStageDisplayStatusSnapshot::disabled);
            let status_state = match status.state {
                crate::android_stage::AndroidStageDisplayState::Disabled => "Disabled".to_string(),
                crate::android_stage::AndroidStageDisplayState::Connecting => {
                    "Connecting".to_string()
                }
                crate::android_stage::AndroidStageDisplayState::Launching => {
                    "Launching".to_string()
                }
                crate::android_stage::AndroidStageDisplayState::Running => "Running".to_string(),
                crate::android_stage::AndroidStageDisplayState::Error => "Error".to_string(),
            };
            let created_display = format_settings_timestamp(display.created_at);
            let updated_display = format_settings_timestamp(display.updated_at);
            let last_attempt_display = status
                .last_attempt
                .map_or_else(|| "\u{2014}".to_string(), format_settings_timestamp);
            let last_success_display = status
                .last_success
                .map_or_else(|| "\u{2014}".to_string(), format_settings_timestamp);
            SettingsAndroidDisplayRow {
                id: display.id.to_string(),
                label: display.label,
                host: display.host,
                port: display.port,
                launch_component: display.launch_component,
                is_enabled: display.is_enabled,
                created_at: display.created_at.to_rfc3339(),
                created_at_display: created_display,
                updated_at: display.updated_at.to_rfc3339(),
                updated_at_display: updated_display,
                status_state,
                last_attempt_display,
                last_success_display,
                status_message: status.last_error.clone(),
                status: Some(status),
            }
        })
        .collect();

    let hosts_json = escape_script_tag(&to_string(&host_rows).unwrap_or_else(|_| "[]".to_string()));
    let android_json =
        escape_script_tag(&to_string(&android_rows).unwrap_or_else(|_| "[]".to_string()));

    let osc_config_json = json_safe(&json!({
        "enabled": osc_settings.enabled,
        "listenPort": osc_settings.listen_port,
        "addressPattern": osc_settings.address_pattern,
        "velocityMode": osc_settings.velocity_mode,
    }));
    let osc_status_json = json_safe(&osc_status);
    let ableset_config_json = json_safe(&json!({
        "enabled": ableset_settings.enabled,
        "host": ableset_settings.host,
        "httpPort": ableset_settings.http_port,
        "oscPort": ableset_settings.osc_port,
        "libraryName": ableset_settings.library_name,
        "songPrefixLength": ableset_settings.song_prefix_length,
    }));
    let ableset_status_json = json_safe(&ableset_status);
    let feature_json = json_safe(&feature_flags);

    let script = scripts::SETTINGS
        .replace("__RESOLUME_HOSTS__", &hosts_json)
        .replace("__ANDROID_STAGE_DISPLAYS__", &android_json)
        .replace("__OSC_CONFIG__", &osc_config_json)
        .replace("__OSC_STATUS__", &osc_status_json)
        .replace("__ABLESET_CONFIG__", &ableset_config_json)
        .replace("__ABLESET_STATUS__", &ableset_status_json)
        .replace("__FEATURE_FLAGS__", &feature_json);

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! {
            <SettingsDocument
                hosts=host_rows.clone()
                android_displays=android_rows.clone()
                osc_settings=osc_settings.clone()
                osc_status=osc_status.clone()
                ableset_settings=ableset_settings.clone()
                ableset_status=ableset_status.clone()
                features=feature_flags.clone()
                script=script.clone()
            />
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}

fn format_settings_timestamp(value: DateTime<Utc>) -> String {
    let local = value.with_timezone(&Local);
    local.format("%d.%m.%Y %H:%M:%S").to_string()
}
