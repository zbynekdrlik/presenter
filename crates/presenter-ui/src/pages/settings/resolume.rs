//! Resolume Arena connections card for the settings page (#347).

use leptos::prelude::*;

use super::{capitalize, format_timestamp, parse_port_in_range, ToastHandle, STATUS_REFRESH_MS};
use crate::api::settings::{self, ResolumeHostDraft, ResolumeHostDto};
use crate::components::modal::confirm;

#[component]
pub fn ResolumeCard(toast: ToastHandle) -> impl IntoView {
    let hosts = RwSignal::new(Vec::<ResolumeHostDto>::new());
    let editing_id = RwSignal::new(Option::<String>::None);
    let label = RwSignal::new(String::new());
    let host = RwSignal::new(String::new());
    let port = RwSignal::new(String::from("8090"));
    let host_enabled = RwSignal::new(true);
    let form_status = RwSignal::new(String::new());
    let form_state = RwSignal::new(String::from("idle"));
    let busy = RwSignal::new(false);

    let refresh = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = settings::list_resolume_hosts().await {
                hosts.set(list);
            }
        });
    };

    // Initial load + 5s status poll.
    refresh();
    {
        let interval = gloo_timers::callback::Interval::new(STATUS_REFRESH_MS, move || {
            leptos::task::spawn_local(async move {
                if let Ok(list) = settings::list_resolume_hosts().await {
                    hosts.set(list);
                }
            });
        });
        interval.forget();
    }

    let reset_form = move || {
        editing_id.set(None);
        label.set(String::new());
        host.set(String::new());
        port.set("8090".to_string());
        host_enabled.set(true);
        form_status.set(String::new());
        form_state.set("idle".to_string());
        if let Some(body) = crate::utils::window::document_body() {
            let _ = body.set_attribute("data-mode", "create");
        }
    };

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let label_val = label.get_untracked().trim().to_string();
        let host_val = host.get_untracked().trim().to_string();
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
        let draft = ResolumeHostDraft {
            label: label_val,
            host: host_val,
            port: port_val,
            is_enabled: host_enabled.get_untracked(),
        };
        let editing = editing_id.get_untracked();
        busy.set(true);
        form_state.set("info".to_string());
        form_status.set(
            if editing.is_some() {
                "Saving changes…"
            } else {
                "Creating connection…"
            }
            .to_string(),
        );
        leptos::task::spawn_local(async move {
            let result = match &editing {
                Some(id) => settings::update_resolume_host(id, &draft).await,
                None => settings::create_resolume_host(&draft).await,
            };
            match result {
                Ok(_) => {
                    if let Ok(list) = settings::list_resolume_hosts().await {
                        hosts.set(list);
                    }
                    toast.show(
                        if editing.is_some() {
                            "Updated Resolume connection."
                        } else {
                            "Added Resolume connection."
                        },
                        "success",
                    );
                    reset_form();
                }
                Err(err) => {
                    form_state.set("error".to_string());
                    form_status.set(format!("Unable to save connection. {err}"));
                }
            }
            busy.set(false);
        });
    };

    let start_edit = move |id: String| {
        if let Some(h) = hosts.get_untracked().iter().find(|h| h.id == id) {
            editing_id.set(Some(h.id.clone()));
            label.set(h.label.clone());
            host.set(h.host.clone());
            port.set(h.port.to_string());
            host_enabled.set(h.is_enabled);
            form_status.set(String::new());
            form_state.set("idle".to_string());
            if let Some(body) = crate::utils::window::document_body() {
                let _ = body.set_attribute("data-mode", "edit");
            }
        }
    };

    let delete_host = move |id: String| {
        let row = hosts.get_untracked();
        let name = row
            .iter()
            .find(|h| h.id == id)
            .map(|h| h.label.clone())
            .unwrap_or_else(|| "this connection".to_string());
        if !confirm(&format!("Remove {name}? Presenter will stop reconnecting.")) {
            return;
        }
        leptos::task::spawn_local(async move {
            match settings::delete_resolume_host(&id).await {
                Ok(()) => {
                    if let Ok(list) = settings::list_resolume_hosts().await {
                        hosts.set(list);
                    }
                    if editing_id.get_untracked().as_deref() == Some(id.as_str()) {
                        reset_form();
                    }
                    toast.show("Deleted Resolume connection.", "success");
                }
                Err(err) => toast.show(&format!("Unable to delete connection. {err}"), "error"),
            }
        });
    };

    let test_host = move |id: String| {
        leptos::task::spawn_local(async move {
            match settings::test_resolume_host(&id).await {
                Ok(result) => {
                    if result.success {
                        let latency = result
                            .latency_ms
                            .map(|ms| format!("{ms:.1} ms"))
                            .unwrap_or_else(|| "—".to_string());
                        toast.show(&format!("Connection OK ({latency})"), "success");
                    } else {
                        let err = result.error.unwrap_or_else(|| "unknown error".to_string());
                        toast.show(&format!("Connection failed: {err}"), "error");
                    }
                }
                Err(err) => toast.show(&format!("Test failed: {err}"), "error"),
            }
            if let Ok(list) = settings::list_resolume_hosts().await {
                hosts.set(list);
            }
        });
    };

    view! {
        <section class="settings__card">
            <header class="settings__card-header">
                <div>
                    <h2>"Resolume Arena Connections"</h2>
                    <p>"Define Resolume web servers Presenter should control."</p>
                </div>
                <div class="settings__badge-group">
                    <span class="settings__badge" data-role="host-count">
                        {move || hosts.get().len().to_string()}
                    </span>
                    <span class="settings__badge-label">"Hosts"</span>
                </div>
            </header>
            <form class="settings__form" data-role="host-form" autocomplete="off" on:submit=on_submit>
                <div class="settings__form-header">
                    <div>
                        <h3 data-role="form-title">
                            {move || if editing_id.get().is_some() { "Edit Resolume Connection" } else { "Add Resolume Connection" }}
                        </h3>
                        <p data-role="form-subtitle">
                            {move || if editing_id.get().is_some() { "Update host details or toggle availability." } else { "Specify hostname, port, and availability." }}
                        </p>
                    </div>
                </div>
                <div class="settings__form-row">
                    <label>
                        <span>"Label"</span>
                        <input type="text" data-role="host-label" placeholder="Main Arena" required
                            prop:value=move || label.get()
                            on:input=move |ev| label.set(event_target_value(&ev)) />
                    </label>
                    <label>
                        <span>"Hostname or DNS"</span>
                        <input type="text" data-role="host-host" placeholder="resolume.lan" required
                            prop:value=move || host.get()
                            on:input=move |ev| host.set(event_target_value(&ev)) />
                    </label>
                    <label class="settings__form-control--small">
                        <span>"Port"</span>
                        <input type="number" data-role="host-port" min="1" max="65535" required
                            prop:value=move || port.get()
                            on:input=move |ev| port.set(event_target_value(&ev)) />
                    </label>
                </div>
                <div class="settings__form-row settings__form-row--single">
                    <label class="settings__form-checkbox settings__form-checkbox--block">
                        <input type="checkbox" data-role="host-enabled"
                            prop:checked=move || host_enabled.get()
                            on:change=move |ev| host_enabled.set(event_target_checked(&ev)) />
                        <span>"Enabled"</span>
                    </label>
                </div>
                <div class="settings__form-actions">
                    <button type="submit" class="settings__button settings__button--primary"
                        data-role="host-submit" prop:disabled=move || busy.get()>
                        {move || if editing_id.get().is_some() { "Save Changes" } else { "Add Connection" }}
                    </button>
                    <button type="button" class="settings__button settings__button--ghost"
                        data-role="host-reset" on:click=move |_| reset_form()>"Cancel"</button>
                </div>
                <p class="settings__form-status" data-role="form-status" data-state=move || form_state.get()>
                    {move || form_status.get()}
                </p>
            </form>
            <ul class="settings__list" data-role="resolume-host-list">
                <Show
                    when=move || !hosts.get().is_empty()
                    fallback=|| view! {
                        <li class="settings__list-empty" data-role="host-empty">"No Resolume connections defined yet."</li>
                    }
                >
                    <For
                        each=move || hosts.get()
                        // Key encodes the rendered status so the 5s poll re-renders
                        // a row when its connection state/latency/error changes —
                        // not only when `updated_at` bumps on an edit.
                        key=|h: &ResolumeHostDto| {
                            let s = h.status.as_ref();
                            format!(
                                "{}-{}-{}-{}-{}-{}",
                                h.id,
                                h.updated_at,
                                s.map(|s| s.state.as_str()).unwrap_or(""),
                                s.map(|s| s.consecutive_failures).unwrap_or(0),
                                s.and_then(|s| s.last_latency_ms).unwrap_or(0.0),
                                s.and_then(|s| s.last_error.as_deref()).unwrap_or(""),
                            )
                        }
                        children=move |h: ResolumeHostDto| {
                            let status = h.status.clone().unwrap_or(settings::ResolumeStatusDto {
                                state: if h.is_enabled { "connecting".into() } else { "disabled".into() },
                                last_latency_ms: None,
                                last_error: None,
                                consecutive_failures: 0,
                                error_since: None,
                            });
                            let raw_state = if status.state.is_empty() {
                                if h.is_enabled { "connecting".to_string() } else { "disabled".to_string() }
                            } else {
                                status.state.to_lowercase()
                            };
                            let status_class = format!("settings__status settings__status--{raw_state}");
                            let status_label = capitalize(&raw_state);
                            let latency = status.last_latency_ms
                                .map(|ms| format!("{ms:.1} ms"))
                                .unwrap_or_else(|| "—".to_string());
                            let updated = format_timestamp(&h.updated_at);
                            let created = format_timestamp(&h.created_at);
                            let detail = if let Some(err) = status.last_error.clone() {
                                if raw_state == "error" {
                                    let failures = status.consecutive_failures;
                                    let plural = if failures != 1 { "s" } else { "" };
                                    let since = status.error_since.as_ref()
                                        .map(|s| format!(" since {}", format_timestamp(s)))
                                        .unwrap_or_default();
                                    Some(view! {
                                        <p class="settings__list-meta settings__list-meta--warning" data-role="host-error-detail">
                                            {format!("⚠ Retrying… ({failures} failure{plural}{since})")}
                                        </p>
                                    }.into_any())
                                } else {
                                    Some(view! {
                                        <p class="settings__list-meta settings__list-meta--warning">{format!("⚠ {err}")}</p>
                                    }.into_any())
                                }
                            } else { None };
                            let id_test = h.id.clone();
                            let id_edit = h.id.clone();
                            let id_delete = h.id.clone();
                            view! {
                                <li class="settings__list-item" data-id=h.id.clone() data-enabled=h.is_enabled.to_string()>
                                    <div class="settings__list-primary">
                                        <div class="settings__list-title">
                                            <span class="settings__host-label">{h.label.clone()}</span>
                                            <span class=status_class>{status_label}</span>
                                        </div>
                                        <p class="settings__list-line">
                                            <code>{h.host.clone()}</code>
                                            <span class="settings__host-port">{format!(":{}", h.port)}</span>
                                        </p>
                                        <p class="settings__list-meta">{format!("Updated {updated}")}</p>
                                        <p class="settings__list-meta">{format!("Created {created}")}</p>
                                        <p class="settings__list-meta">{format!("Latency {latency}")}</p>
                                        {detail}
                                    </div>
                                    <div class="settings__list-actions">
                                        <button type="button" class="settings__button settings__button--ghost"
                                            data-role="host-test" data-id=id_test.clone()
                                            on:click=move |_| test_host(id_test.clone())>"Test"</button>
                                        <button type="button" class="settings__button settings__button--ghost"
                                            data-role="host-edit" data-id=id_edit.clone()
                                            on:click=move |_| start_edit(id_edit.clone())>"Edit"</button>
                                        <button type="button" class="settings__button settings__button--danger"
                                            data-role="host-delete" data-id=id_delete.clone()
                                            on:click=move |_| delete_host(id_delete.clone())>"Delete"</button>
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
