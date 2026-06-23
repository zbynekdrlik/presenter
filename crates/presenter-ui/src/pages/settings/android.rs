//! Android stage launchers card for the settings page (#347).

use leptos::prelude::*;

use super::{capitalize, format_timestamp, parse_port_in_range, ToastHandle, STATUS_REFRESH_MS};
use crate::api::settings::{self, AndroidDisplayDraft, AndroidDisplayDto};
use crate::components::modal::confirm;

#[component]
pub fn AndroidCard(toast: ToastHandle) -> impl IntoView {
    let displays = RwSignal::new(Vec::<AndroidDisplayDto>::new());
    let editing_id = RwSignal::new(Option::<String>::None);
    let label = RwSignal::new(String::new());
    let host = RwSignal::new(String::new());
    let port = RwSignal::new(String::from("5555"));
    let component = RwSignal::new(String::from("com.tcl.browser"));
    let enabled = RwSignal::new(true);
    let form_status = RwSignal::new(String::new());
    let form_state = RwSignal::new(String::from("idle"));
    let busy = RwSignal::new(false);

    let refresh = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = settings::list_android_displays().await {
                displays.set(list);
            }
        });
    };

    refresh();
    {
        let interval = gloo_timers::callback::Interval::new(STATUS_REFRESH_MS, move || {
            leptos::task::spawn_local(async move {
                if let Ok(list) = settings::list_android_displays().await {
                    displays.set(list);
                }
            });
        });
        interval.forget();
    }

    let reset_form = move || {
        editing_id.set(None);
        label.set(String::new());
        host.set(String::new());
        port.set("5555".to_string());
        component.set("com.tcl.browser".to_string());
        enabled.set(true);
        form_status.set(String::new());
        form_state.set("idle".to_string());
    };

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let label_val = label.get_untracked().trim().to_string();
        let host_val = host.get_untracked().trim().to_string();
        let component_val = component.get_untracked().trim().to_string();
        if label_val.is_empty() {
            form_state.set("error".to_string());
            form_status.set("Label cannot be empty.".to_string());
            return;
        }
        if host_val.is_empty() {
            form_state.set("error".to_string());
            form_status.set("Host cannot be empty.".to_string());
            return;
        }
        let Some(port_val) = parse_port_in_range(&port.get_untracked()) else {
            form_state.set("error".to_string());
            form_status.set("Port must be between 1 and 65535.".to_string());
            return;
        };
        if component_val.is_empty() {
            form_state.set("error".to_string());
            form_status.set("Launch component cannot be empty.".to_string());
            return;
        }
        let draft = AndroidDisplayDraft {
            label: label_val,
            host: host_val,
            port: port_val,
            launch_component: component_val,
            is_enabled: enabled.get_untracked(),
        };
        let editing = editing_id.get_untracked();
        busy.set(true);
        form_state.set("loading".to_string());
        form_status.set(
            if editing.is_some() {
                "Updating display…"
            } else {
                "Creating display…"
            }
            .to_string(),
        );
        leptos::task::spawn_local(async move {
            let result = match &editing {
                Some(id) => settings::update_android_display(id, &draft).await,
                None => settings::create_android_display(&draft).await,
            };
            match result {
                Ok(_) => {
                    if let Ok(list) = settings::list_android_displays().await {
                        displays.set(list);
                    }
                    reset_form();
                    toast.show(
                        if editing.is_some() {
                            "Saved Android stage display."
                        } else {
                            "Added Android stage display."
                        },
                        "success",
                    );
                }
                Err(err) => {
                    form_state.set("error".to_string());
                    form_status.set(format!("Unable to save display. {err}"));
                    toast.show(&format!("Unable to save display. {err}"), "error");
                }
            }
            busy.set(false);
        });
    };

    let start_edit = move |id: String| {
        if let Some(d) = displays.get_untracked().iter().find(|d| d.id == id) {
            editing_id.set(Some(d.id.clone()));
            label.set(d.label.clone());
            host.set(d.host.clone());
            port.set(d.port.to_string());
            component.set(d.launch_component.clone());
            enabled.set(d.is_enabled);
            form_status.set(String::new());
            form_state.set("idle".to_string());
        }
    };

    let delete_display = move |id: String| {
        let name = displays
            .get_untracked()
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.label.clone())
            .unwrap_or_else(|| "this display".to_string());
        if !confirm(&format!("Remove {name}? Presenter will stop reconnecting.")) {
            return;
        }
        leptos::task::spawn_local(async move {
            match settings::delete_android_display(&id).await {
                Ok(()) => {
                    if let Ok(list) = settings::list_android_displays().await {
                        displays.set(list);
                    }
                    if editing_id.get_untracked().as_deref() == Some(id.as_str()) {
                        reset_form();
                    }
                    toast.show("Deleted Android stage display.", "success");
                }
                Err(err) => toast.show(&format!("Unable to delete display. {err}"), "error"),
            }
        });
    };

    let test_display = move |id: String| {
        leptos::task::spawn_local(async move {
            match settings::launch_android_display(&id).await {
                Ok(()) => {
                    toast.show("Launch queued — refreshing status…", "success");
                    gloo_timers::future::TimeoutFuture::new(600).await;
                    if let Ok(list) = settings::list_android_displays().await {
                        displays.set(list);
                    }
                }
                Err(err) => toast.show(&format!("Unable to trigger launch. {err}"), "error"),
            }
        });
    };

    view! {
        <section class="settings__card">
            <header class="settings__card-header">
                <div>
                    <h2>"Android Stage Launchers"</h2>
                    <p>"Keep each Android TV pinned to the stage display."</p>
                </div>
                <div class="settings__badge-group">
                    <span class="settings__badge" data-role="android-count">
                        {move || displays.get().len().to_string()}
                    </span>
                    <span class="settings__badge-label">"Displays"</span>
                </div>
            </header>
            <form class="settings__form" data-role="android-form" autocomplete="off"
                data-mode=move || if editing_id.get().is_some() { "edit" } else { "create" }
                on:submit=on_submit>
                <div class="settings__form-header">
                    <div>
                        <h3 data-role="android-form-title">
                            {move || if editing_id.get().is_some() { "Edit Android Stage Display" } else { "Add Android Stage Display" }}
                        </h3>
                        <p data-role="android-form-subtitle">
                            {move || if editing_id.get().is_some() {
                                "Update connection details or disable auto-launch."
                            } else {
                                "Presenter reconnects and reopens the stage URL in the launch package whenever the device appears."
                            }}
                        </p>
                    </div>
                </div>
                <div class="settings__form-row">
                    <label>
                        <span>"Label"</span>
                        <input type="text" data-role="android-label" placeholder="Stage Left" required
                            prop:value=move || label.get()
                            on:input=move |ev| label.set(event_target_value(&ev)) />
                    </label>
                    <label>
                        <span>"Hostname or DNS"</span>
                        <input type="text" data-role="android-host" placeholder="sd1l.lan" required
                            prop:value=move || host.get()
                            on:input=move |ev| host.set(event_target_value(&ev)) />
                    </label>
                    <label class="settings__form-control--small">
                        <span>"Port"</span>
                        <input type="number" data-role="android-port" min="1" max="65535" required
                            prop:value=move || port.get()
                            on:input=move |ev| port.set(event_target_value(&ev)) />
                    </label>
                </div>
                <div class="settings__form-row settings__form-row--single">
                    <label>
                        <span>"Launch Package"</span>
                        <input type="text" data-role="android-component" placeholder="com.tcl.browser" required
                            prop:value=move || component.get()
                            on:input=move |ev| component.set(event_target_value(&ev)) />
                    </label>
                </div>
                <div class="settings__form-row settings__form-row--single">
                    <label class="settings__form-checkbox settings__form-checkbox--block">
                        <input type="checkbox" data-role="android-enabled"
                            prop:checked=move || enabled.get()
                            on:change=move |ev| enabled.set(event_target_checked(&ev)) />
                        <span>"Enabled"</span>
                    </label>
                </div>
                <div class="settings__form-actions">
                    <button type="submit" class="settings__button settings__button--primary"
                        data-role="android-submit" prop:disabled=move || busy.get()>
                        {move || if editing_id.get().is_some() { "Save Changes" } else { "Add Android Display" }}
                    </button>
                    <button type="button" class="settings__button settings__button--ghost"
                        data-role="android-reset" on:click=move |_| reset_form()>"Cancel"</button>
                </div>
                <p class="settings__form-status" data-role="android-form-status" data-state=move || form_state.get()>
                    {move || form_status.get()}
                </p>
            </form>
            <ul class="settings__list" data-role="android-display-list">
                <Show
                    when=move || !displays.get().is_empty()
                    fallback=|| view! {
                        <li class="settings__list-empty" data-role="android-empty">"No Android stage displays configured yet."</li>
                    }
                >
                    <For
                        each=move || displays.get()
                        // Key encodes status so the 5s poll re-renders a row when
                        // its launch state / timestamps / error change, not only on edit.
                        key=|d: &AndroidDisplayDto| {
                            let s = d.status.as_ref();
                            format!(
                                "{}-{}-{}-{}-{}-{}",
                                d.id,
                                d.updated_at,
                                s.map(|s| s.state.as_str()).unwrap_or(""),
                                s.and_then(|s| s.last_attempt.as_deref()).unwrap_or(""),
                                s.and_then(|s| s.last_success.as_deref()).unwrap_or(""),
                                s.and_then(|s| s.last_error.as_deref()).unwrap_or(""),
                            )
                        }
                        children=move |d: AndroidDisplayDto| {
                            let status = d.status.clone();
                            let raw_state_src = status.as_ref()
                                .map(|s| s.state.clone())
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| if d.is_enabled { "Connecting".into() } else { "Disabled".into() });
                            let normalized_state = raw_state_src.to_lowercase().replace(' ', "-");
                            let status_class = format!("settings__status settings__status--{normalized_state}");
                            let status_label = capitalize(&raw_state_src);
                            let last_attempt = status.as_ref()
                                .and_then(|s| s.last_attempt.clone())
                                .map(|t| format_timestamp(&t))
                                .unwrap_or_else(|| "—".to_string());
                            let last_success = status.as_ref()
                                .and_then(|s| s.last_success.clone())
                                .map(|t| format_timestamp(&t))
                                .unwrap_or_else(|| "—".to_string());
                            let updated = format_timestamp(&d.updated_at);
                            let created = format_timestamp(&d.created_at);
                            let warning = status.as_ref().and_then(|s| s.last_error.clone());
                            let warning_view = warning.map(|err| view! {
                                <p class="settings__list-meta settings__list-meta--warning">{format!("⚠ {err}")}</p>
                            });
                            let id_test = d.id.clone();
                            let id_edit = d.id.clone();
                            let id_delete = d.id.clone();
                            view! {
                                <li class="settings__list-item" data-id=d.id.clone() data-enabled=d.is_enabled.to_string()>
                                    <div class="settings__list-primary">
                                        <div class="settings__list-title">
                                            <span class="settings__host-label">{d.label.clone()}</span>
                                            <span class=status_class>{status_label}</span>
                                        </div>
                                        <p class="settings__list-line">
                                            <code>{d.host.clone()}</code>
                                            <span class="settings__host-port">{format!(":{}", d.port)}</span>
                                        </p>
                                        <p class="settings__list-meta">{format!("Component {}", d.launch_component)}</p>
                                        <p class="settings__list-meta">{format!("Last attempt {last_attempt}")}</p>
                                        <p class="settings__list-meta">{format!("Last success {last_success}")}</p>
                                        <p class="settings__list-meta">{format!("Updated {updated}")}</p>
                                        <p class="settings__list-meta">{format!("Created {created}")}</p>
                                        {warning_view}
                                    </div>
                                    <div class="settings__list-actions">
                                        <button type="button" class="settings__button settings__button--ghost"
                                            data-role="android-test" data-id=id_test.clone()
                                            on:click=move |_| test_display(id_test.clone())>"Test"</button>
                                        <button type="button" class="settings__button settings__button--ghost"
                                            data-role="android-edit" data-id=id_edit.clone()
                                            on:click=move |_| start_edit(id_edit.clone())>"Edit"</button>
                                        <button type="button" class="settings__button settings__button--danger"
                                            data-role="android-delete" data-id=id_delete.clone()
                                            on:click=move |_| delete_display(id_delete.clone())>"Delete"</button>
                                    </div>
                                </li>
                            }
                        }
                    />
                </Show>
            </ul>
        </section>
    }
}
