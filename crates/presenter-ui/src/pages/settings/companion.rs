//! Companion (feature-flags) card for the settings page (#347).

use leptos::prelude::*;

use crate::api::settings::{self, FeatureFlagsDraft};

#[component]
pub fn CompanionCard() -> impl IntoView {
    let enabled = RwSignal::new(false);
    let port = RwSignal::new(String::from("18175"));
    let status_msg = RwSignal::new(String::new());
    let status_state = RwSignal::new(String::from("idle"));
    let busy = RwSignal::new(false);

    leptos::task::spawn_local(async move {
        if let Ok(flags) = settings::get_features().await {
            enabled.set(flags.companion_enabled);
            port.set(flags.companion_port.to_string());
        }
    });

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let parsed = port.get_untracked().trim().parse::<u16>();
        let port_val = match parsed {
            Ok(p) if p >= 1 => p,
            _ => {
                status_state.set("error".to_string());
                status_msg.set("Port must be between 1 and 65535.".to_string());
                return;
            }
        };
        let want_enabled = enabled.get_untracked();
        busy.set(true);
        status_state.set("info".to_string());
        status_msg.set("Saving…".to_string());
        leptos::task::spawn_local(async move {
            let draft = FeatureFlagsDraft {
                companion_enabled: want_enabled,
                companion_port: port_val,
            };
            match settings::update_features(&draft).await {
                Ok(flags) => {
                    enabled.set(flags.companion_enabled);
                    port.set(flags.companion_port.to_string());
                    status_state.set("success".to_string());
                    status_msg.set("Saved.".to_string());
                }
                Err(err) => {
                    status_state.set("error".to_string());
                    status_msg.set(format!("Unable to save Companion settings. {err}"));
                }
            }
            busy.set(false);
        });
    };

    view! {
        <section class="settings__card settings__card--feature">
            <header class="settings__card-header">
                <div><h2>"Companion"</h2></div>
            </header>
            <form
                class="settings__form settings__form--compact"
                data-role="feature-companion-form"
                autocomplete="off"
                on:submit=on_submit
            >
                <div class="settings__form-row settings__form-row--compact settings__form-row--inline">
                    <label class="settings__form-checkbox settings__form-checkbox--inline">
                        <input
                            type="checkbox"
                            data-role="feature-companion-toggle"
                            prop:checked=move || enabled.get()
                            prop:disabled=move || busy.get()
                            on:change=move |ev| {
                                enabled.set(event_target_checked(&ev));
                                status_state.set("idle".to_string());
                                status_msg.set(String::new());
                            }
                        />
                        <span>"Enable"</span>
                    </label>
                    <label class="settings__form-control--tiny">
                        <span>"Port"</span>
                        <input
                            type="number"
                            min="1"
                            max="65535"
                            data-role="feature-companion-port"
                            prop:value=move || port.get()
                            prop:disabled=move || busy.get()
                            on:input=move |ev| {
                                port.set(event_target_value(&ev));
                                status_state.set("idle".to_string());
                                status_msg.set(String::new());
                            }
                            required
                            aria-required="true"
                            aria-describedby="feature-companion-status"
                            aria-invalid=move || (status_state.get() == "error").to_string()
                        />
                    </label>
                    <button
                        type="submit"
                        class="settings__button settings__button--primary settings__button--compact"
                        data-role="feature-submit"
                        prop:disabled=move || busy.get()
                    >"Save"</button>
                </div>
                <p
                    id="feature-companion-status"
                    class="settings__form-status"
                    data-role="feature-status"
                    data-state=move || status_state.get()
                >{move || status_msg.get()}</p>
            </form>
        </section>
    }
}
