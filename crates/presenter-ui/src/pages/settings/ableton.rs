//! Ableton control card (AbleSet + OSC) for the settings page (#347).

use leptos::prelude::*;

use super::{capitalize, format_timestamp, parse_port_in_range, ToastHandle, STATUS_REFRESH_MS};
use crate::api::settings::{self, OscStatusDto};
use presenter_core::{AbleSetSettingsDraft, AbleSetStatusSnapshot, OscSettingsDraft, VelocityMode};

#[component]
pub fn AbletonCard(toast: ToastHandle) -> impl IntoView {
    let enabled = RwSignal::new(false);
    let host = RwSignal::new(String::from("fohabl.lan"));
    let http_port = RwSignal::new(String::from("80"));
    let library = RwSignal::new(String::from("NEW LEVEL"));
    let osc_port = RwSignal::new(String::from("39051"));
    let song_prefix_length = RwSignal::new(3u8);
    let form_status = RwSignal::new(String::new());
    let form_state = RwSignal::new(String::from("idle"));
    let busy = RwSignal::new(false);

    let ableset_status = RwSignal::new(Option::<AbleSetStatusSnapshot>::None);
    let osc_status = RwSignal::new(Option::<OscStatusDto>::None);

    // Initial config + status, then 5s status poll.
    leptos::task::spawn_local(async move {
        if let Ok(cfg) = settings::get_ableset_settings().await {
            enabled.set(cfg.enabled);
            host.set(cfg.host);
            http_port.set(cfg.http_port.to_string());
            library.set(cfg.library_name);
            osc_port.set(cfg.osc_port.to_string());
            song_prefix_length.set(cfg.song_prefix_length);
        }
        if let Ok(status) = settings::get_ableset_status().await {
            ableset_status.set(Some(status));
        }
        if let Ok(status) = settings::get_osc_status().await {
            osc_status.set(Some(status));
        }
    });
    {
        let interval = gloo_timers::callback::Interval::new(STATUS_REFRESH_MS, move || {
            leptos::task::spawn_local(async move {
                if let Ok(status) = settings::get_ableset_status().await {
                    ableset_status.set(Some(status));
                }
                if let Ok(status) = settings::get_osc_status().await {
                    osc_status.set(Some(status));
                }
            });
        });
        interval.forget();
    }

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let want_enabled = enabled.get_untracked();
        let host_val = host.get_untracked().trim().to_string();
        // Ableton ports stay lenient: invalid / out-of-range input falls back to
        // the sane default (the original used `toNumber(value, fallback)`, which
        // fell back on non-numeric input; we additionally fall back on
        // out-of-range, which is safe because the draft field is `u16` and the
        // server still validates). `parse_port_in_range` avoids the u16
        // overflow-truncation a naive parse would hit on a too-large value.
        let http_port_val = parse_port_in_range(&http_port.get_untracked()).unwrap_or(80);
        let library_val = library.get_untracked().trim().to_string();
        let listen_port = parse_port_in_range(&osc_port.get_untracked()).unwrap_or(39051);
        let prefix = song_prefix_length.get_untracked().max(1);
        busy.set(true);
        form_state.set("loading".to_string());
        form_status.set("Saving Ableton settings…".to_string());
        leptos::task::spawn_local(async move {
            let ableset_draft = AbleSetSettingsDraft {
                enabled: want_enabled,
                host: host_val,
                osc_port: listen_port,
                http_port: http_port_val,
                library_name: library_val,
                song_prefix_length: prefix,
            };
            let osc_draft = OscSettingsDraft {
                enabled: want_enabled,
                listen_port,
                address_pattern: "/note".to_string(),
                velocity_mode: VelocityMode::OneBased,
            };
            let result = async {
                let cfg = settings::update_ableset_settings(&ableset_draft).await?;
                settings::update_osc_settings(&osc_draft).await?;
                Ok::<_, crate::api::ApiError>(cfg)
            }
            .await;
            match result {
                Ok(cfg) => {
                    enabled.set(cfg.enabled);
                    host.set(cfg.host);
                    http_port.set(cfg.http_port.to_string());
                    library.set(cfg.library_name);
                    osc_port.set(cfg.osc_port.to_string());
                    song_prefix_length.set(cfg.song_prefix_length);
                    form_state.set("success".to_string());
                    form_status.set("Ableton settings saved.".to_string());
                    toast.show("Ableton settings saved.", "success");
                    if let Ok(status) = settings::get_ableset_status().await {
                        ableset_status.set(Some(status));
                    }
                    if let Ok(status) = settings::get_osc_status().await {
                        osc_status.set(Some(status));
                    }
                }
                Err(err) => {
                    form_state.set("error".to_string());
                    form_status.set(format!("Failed to update Ableton settings. {err}"));
                    toast.show("Unable to update Ableton settings.", "error");
                }
            }
            busy.set(false);
        });
    };

    let ableset_state_label = move || {
        let status = ableset_status.get();
        let s = status.as_ref();
        if s.map(|s| s.enabled).unwrap_or(false) {
            if s.map(|s| s.tracking).unwrap_or(false) {
                "tracking"
            } else {
                "enabled"
            }
        } else {
            "disabled"
        }
    };
    let osc_state_label = move || {
        let status = osc_status.get();
        let s = status.as_ref();
        if s.map(|s| s.enabled).unwrap_or(false) {
            if s.map(|s| s.listening).unwrap_or(false) {
                "listening"
            } else {
                "enabled"
            }
        } else {
            "disabled"
        }
    };

    view! {
        <section class="settings__card settings__card--ableton">
            <header class="settings__card-header">
                <div>
                    <h2>"Ableton Control"</h2>
                    <p>"Configure AbleSet tracking and Presenter's OSC listener."</p>
                </div>
            </header>
            <form class="settings__form settings__form--ableset" data-role="ableset-form" autocomplete="off"
                data-mode=move || if enabled.get() { "enabled" } else { "disabled" }
                on:submit=on_submit>
                <div class="settings__form-row settings__form-row--single">
                    <label class="settings__form-checkbox settings__form-checkbox--block">
                        <input type="checkbox" data-role="ableset-enabled"
                            prop:checked=move || enabled.get()
                            on:change=move |ev| enabled.set(event_target_checked(&ev)) />
                        <span>"Enable Ableton automation"</span>
                    </label>
                </div>
                <div class="settings__form-row">
                    <label>
                        <span>"AbleSet Host"</span>
                        <input type="text" data-role="ableset-host" required
                            prop:value=move || host.get()
                            on:input=move |ev| host.set(event_target_value(&ev)) />
                    </label>
                    <label class="settings__form-control settings__form-control--small">
                        <span>"HTTP Port"</span>
                        <input type="number" data-role="ableset-http-port" min="1" max="65535" required
                            prop:value=move || http_port.get()
                            on:input=move |ev| http_port.set(event_target_value(&ev)) />
                    </label>
                    <label>
                        <span>"Library Name"</span>
                        <input type="text" data-role="ableset-library" required
                            prop:value=move || library.get()
                            on:input=move |ev| library.set(event_target_value(&ev)) />
                    </label>
                </div>
                <div class="settings__form-row settings__form-row--single">
                    <label class="settings__form-control settings__form-control--small">
                        <span>"OSC Listener Port"</span>
                        <input type="number" data-role="osc-port" min="1" max="65535" required
                            prop:value=move || osc_port.get()
                            on:input=move |ev| osc_port.set(event_target_value(&ev)) />
                    </label>
                </div>
                <div class="settings__form-actions">
                    <button type="submit" class="settings__button settings__button--primary"
                        data-role="ableset-submit" prop:disabled=move || busy.get()>"Save AbleSet Settings"</button>
                </div>
                <p class="settings__form-status" data-role="ableset-form-status" data-state=move || form_state.get()>
                    {move || form_status.get()}
                </p>
            </form>
            <div class="settings__status-panel">
                <span class=move || format!("settings__status settings__status--{}", ableset_state_label())
                    data-role="ableset-status-indicator" data-state=ableset_state_label>
                    {move || capitalize(ableset_state_label())}
                </span>
                <dl class="settings__status-list">
                    <div>
                        <dt>"Current song"</dt>
                        <dd data-role="ableset-status-song">
                            {move || ableset_status.get()
                                .and_then(|s| s.last_song.map(|song| song.name))
                                .filter(|n| !n.is_empty())
                                .unwrap_or_else(|| "—".to_string())}
                        </dd>
                    </div>
                    <div>
                        <dt>"Last update"</dt>
                        <dd data-role="ableset-status-updated">
                            {move || ableset_status.get()
                                .and_then(|s| s.last_song.and_then(|song| song.last_seen_at))
                                .map(|t| format_timestamp(&t.to_rfc3339()))
                                .unwrap_or_else(|| "—".to_string())}
                        </dd>
                    </div>
                </dl>
                <p class="settings__list-meta settings__list-meta--warning" data-role="ableset-status-error"
                    data-visible=move || if ableset_status.get().and_then(|s| s.last_error).is_some() { "true" } else { "false" }>
                    {move || ableset_status.get().and_then(|s| s.last_error)
                        .map(|e| format!("⚠ {e}")).unwrap_or_default()}
                </p>
            </div>
            <div class="settings__status-panel">
                <span class=move || format!("settings__status settings__status--{}", osc_state_label())
                    data-role="osc-status-indicator" data-state=osc_state_label>
                    {move || capitalize(osc_state_label())}
                </span>
                <dl class="settings__status-list">
                    <div>
                        <dt>"Last event"</dt>
                        <dd data-role="osc-status-last-message">
                            {move || osc_status.get()
                                .and_then(|s| s.last_message_at)
                                .map(|t| format_timestamp(&t))
                                .unwrap_or_else(|| "—".to_string())}
                        </dd>
                    </div>
                    <div>
                        <dt>"Last note"</dt>
                        <dd data-role="osc-status-last-note">
                            {move || {
                                let status = osc_status.get();
                                match status.as_ref().and_then(|s| s.last_note) {
                                    Some(note) => {
                                        match status.as_ref().and_then(|s| s.last_velocity) {
                                            Some(vel) => format!("note {note} (vel {vel})"),
                                            None => format!("note {note}"),
                                        }
                                    }
                                    None => "—".to_string(),
                                }
                            }}
                        </dd>
                    </div>
                </dl>
                <p class="settings__list-meta settings__list-meta--warning" data-role="osc-status-error"
                    data-visible=move || if osc_status.get().and_then(|s| s.last_error).is_some() { "true" } else { "false" }>
                    {move || osc_status.get().and_then(|s| s.last_error)
                        .map(|e| format!("⚠ {e}")).unwrap_or_default()}
                </p>
            </div>
        </section>
    }
}
