#![allow(clippy::needless_pass_by_value, clippy::too_many_lines)]

use std::sync::Arc;

use leptos::prelude::*;

use crate::ui::settings::{SettingsAndroidDisplayRow, SettingsHostRow};

#[component]
pub fn CompanionSettingsCard(enabled: bool, port_text: String) -> impl IntoView {
    view! {
        <section class="settings__card settings__card--feature">
            <header class="settings__card-header">
                <div>
                    <h2>"Companion"</h2>
                </div>
            </header>
            <form
                class="settings__form settings__form--compact"
                data-role="feature-companion-form"
                autocomplete="off"
            >
                <div class="settings__form-row settings__form-row--compact settings__form-row--inline">
                    <label class="settings__form-checkbox settings__form-checkbox--inline">
                        <input
                            type="checkbox"
                            data-role="feature-companion-toggle"
                            checked=enabled
                        />
                        <span>"Enable"</span>
                    </label>
                    <label class="settings__form-control--tiny">
                        <span>"Port"</span>
                        <input
                            type="number"
                            min="1"
                            max="65535"
                            value=port_text.clone()
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
    }
}

#[component]
pub fn ResolumeConnectionsCard(
    hosts: Arc<Vec<SettingsHostRow>>,
    host_count_text: String,
) -> impl IntoView {
    view! {
        <section class="settings__card">
            <header class="settings__card-header">
                <div>
                    <h2>"Resolume Arena Connections"</h2>
                    <p>
                        "Define Resolume web servers Presenter should control."
                    </p>
                </div>
                <div class="settings__badge-group">
                    <span class="settings__badge" data-role="host-count">{host_count_text}</span>
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
                    fallback={|| view! {
                        <li class="settings__list-empty" data-role="host-empty">
                            "No Resolume connections defined yet."
                        </li>
                    }}
                >
                    <For
                        each={
                            let hosts = Arc::clone(&hosts);
                            move || (*hosts).clone()
                        }
                        key=|host: &SettingsHostRow| host.id.clone()
                        children={|host: SettingsHostRow| {
                            let raw_state = host.status_state.to_lowercase();
                            let status_class = format!(
                                "settings__status settings__status--{}",
                                if raw_state.is_empty() { "disabled" } else { raw_state.as_str() }
                            );
                            let status_label = if raw_state.is_empty() {
                                "Disabled".to_string()
                            } else {
                                format!(
                                    "{}{}",
                                    raw_state
                                        .chars()
                                        .next()
                                        .map_or_else(String::new, |c| c.to_uppercase().collect::<String>()),
                                    raw_state.chars().skip(1).collect::<String>()
                                )
                            };
                            let latency_text = host
                                .last_latency_ms
                                .map_or_else(|| "—".to_string(), |ms| format!("{ms:.1} ms"));
                            let warning_text = host.status_message.clone().unwrap_or_default();
                            let warning_view = (!warning_text.is_empty()).then(|| {
                                view! {
                                    <p class="settings__list-meta settings__list-meta--warning">
                                        {format!("⚠ {warning_text}")}
                                    </p>
                                }
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
                                            <span class={status_class}>{status_label}</span>
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
                        <dd>"Append -u to force uppercase and -re to collapse multi-line text into a single space-delimited line."</dd>
                    </div>
                </dl>
            </section>
        </section>
    }
}

#[component]
pub fn AndroidStageSettingsCard(
    displays: Arc<Vec<SettingsAndroidDisplayRow>>,
    display_count_text: String,
) -> impl IntoView {
    view! {
        <section class="settings__card">
            <header class="settings__card-header">
                <div>
                    <h2>"Android Stage Launchers"</h2>
                    <p>
                        "Configure Fully Kiosk devices for remote presentation control."
                    </p>
                </div>
                <div class="settings__badge-group">
                    <span class="settings__badge" data-role="android-count">{display_count_text}</span>
                    <span class="settings__badge-label">"Devices"</span>
                </div>
            </header>
            <form class="settings__form" data-role="android-form" autocomplete="off">
                <input type="hidden" data-role="android-id" />
                <div class="settings__form-header">
                    <div>
                        <h3 data-role="android-form-title">"Add Android Stage Display"</h3>
                        <p data-role="android-form-subtitle">"Provide device host, ADB port, and launch component."</p>
                    </div>
                </div>
                <div class="settings__form-row">
                    <label>
                        <span>"Label"</span>
                        <input
                            type="text"
                            name="label"
                            data-role="android-label"
                            placeholder="Stage Left"
                            required
                        />
                    </label>
                    <label>
                        <span>"Hostname or IP"</span>
                        <input
                            type="text"
                            name="host"
                            data-role="android-host"
                            placeholder="sd1l.lan"
                            required
                        />
                    </label>
                    <label class="settings__form-control--small">
                        <span>"ADB Port"</span>
                        <input
                            type="number"
                            name="port"
                            data-role="android-port"
                            min="1"
                            max="65535"
                            value="5555"
                            required
                        />
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
                    >"Add Display"</button>
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
                        let displays = Arc::clone(&displays);
                        move || !displays.is_empty()
                    }
                    fallback={|| view! {
                        <li class="settings__list-empty" data-role="android-empty">
                            "No Android stage displays configured yet."
                        </li>
                    }}
                >
                    <For
                        each={
                            let displays = Arc::clone(&displays);
                            move || (*displays).clone()
                        }
                        key=|display: &SettingsAndroidDisplayRow| display.id.clone()
                        children={|display: SettingsAndroidDisplayRow| {
                            let state = display.status_state.to_lowercase();
                            let status_class = format!(
                                "settings__status settings__status--{}",
                                if state.is_empty() { "disabled" } else { state.as_str() }
                            );
                            let status_label = if state.is_empty() {
                                "Disabled".to_string()
                            } else {
                                format!(
                                    "{}{}",
                                    state
                                        .chars()
                                        .next()
                                        .map_or_else(String::new, |c| c.to_uppercase().collect::<String>()),
                                    state.chars().skip(1).collect::<String>()
                                )
                            };
                            let warning_text = display.status_message.clone().unwrap_or_default();
                            let warning_view = (!warning_text.is_empty()).then(|| {
                                view! {
                                    <p class="settings__list-meta settings__list-meta--warning">
                                        {format!("⚠ {warning_text}")}
                                    </p>
                                }
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
                                            <span>{format!(":{}", display.port)}</span>
                                        </p>
                                        <p class="settings__list-meta">{"Launcher "}{display.launch_component.clone()}</p>
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
    }
}

#[component]
#[allow(clippy::too_many_arguments)]
pub fn AbleSetOscSettingsCard(
    ableset_host_value: String,
    ableset_http_port_value: String,
    ableset_library_value: String,
    ableset_enabled: bool,
    osc_port_value: String,
    ableset_status_state: String,
    ableset_status_label: String,
    ableset_last_song_name: String,
    ableset_last_song_seen: String,
    ableset_last_error: Option<String>,
    osc_status_state: String,
    osc_status_label: String,
    osc_last_message_display: String,
    osc_last_note_display: String,
    osc_last_error: Option<String>,
) -> impl IntoView {
    let ableset_error_text = ableset_last_error.unwrap_or_default();
    let osc_error_visible = osc_last_error.is_some();
    let osc_error_text = osc_last_error.unwrap_or_default();

    view! {
        <section class="settings__card">
            <header class="settings__card-header">
                <div>
                    <h2>"AbleSet & OSC"</h2>
                    <p>"Configure Ableton automation and OSC bridges."</p>
                </div>
            </header>
            <form class="settings__form" data-role="ableset-form" autocomplete="off">
                <div class="settings__form-row settings__form-row--single">
                    <label class="settings__form-checkbox settings__form-checkbox--block">
                        <input type="checkbox" data-role="ableset-enabled" checked=ableset_enabled />
                        <span>"Enable Ableton automation"</span>
                    </label>
                </div>
                <div class="settings__form-row">
                    <label>
                        <span>"AbleSet Host"</span>
                        <input
                            type="text"
                            data-role="ableset-host"
                            value=ableset_host_value
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
                            value=ableset_http_port_value
                            required
                        />
                    </label>
                    <label>
                        <span>"Library Name"</span>
                        <input
                            type="text"
                            data-role="ableset-library"
                            value=ableset_library_value
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
                            value=osc_port_value
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
                    class={format!("settings__status settings__status--{ableset_status_state}")}
                    data-role="ableset-status-indicator"
                >{ableset_status_label}</span>
                <dl class="settings__status-list">
                    <div>
                        <dt>"Current song"</dt>
                        <dd data-role="ableset-status-song">{ableset_last_song_name}</dd>
                    </div>
                    <div>
                        <dt>"Last update"</dt>
                        <dd data-role="ableset-status-updated">{ableset_last_song_seen}</dd>
                    </div>
                </dl>
                <p class="settings__list-meta settings__list-meta--warning" data-role="ableset-status-error">
                    {ableset_error_text}
                </p>
            </div>
            <div class="settings__status-panel">
                <span
                    class={format!("settings__status settings__status--{osc_status_state}")}
                    data-role="osc-status-indicator"
                >{osc_status_label}</span>
                <dl class="settings__status-list">
                    <div>
                        <dt>"Last event"</dt>
                        <dd data-role="osc-status-last-message">{osc_last_message_display}</dd>
                    </div>
                    <div>
                        <dt>"Last note"</dt>
                        <dd data-role="osc-status-last-note">{osc_last_note_display}</dd>
                    </div>
                </dl>
                <p
                    class="settings__list-meta settings__list-meta--warning"
                    data-role="osc-status-error"
                    data-visible={if osc_error_visible { "true" } else { "false" }}
                >
                    {if osc_error_text.is_empty() {
                        String::new()
                    } else {
                        format!("⚠ {osc_error_text}")
                    }}
                </p>
            </div>
        </section>
    }
}
