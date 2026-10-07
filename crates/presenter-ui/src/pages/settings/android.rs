//! Android stage launchers card for the settings page (#347).
//!
//! #819: **Edit** opens the editor IN the clicked row and "+ Add display" opens it
//! at the top of the list (`host_editor`, shared with the Resolume card). Rows are
//! keyed on identity (id + `updated_at`) and read their live launch status through a
//! `Memo`, so the 5 s poll never rebuilds a row or touches an open editor.

use leptos::prelude::*;

use super::host_editor::{
    render_connection_editor, ConnectionDraft, EditTarget, EditorSpec, ExtraField,
};
use super::row_status::{android_attempts, android_state, updated_created};
use super::{ToastHandle, STATUS_REFRESH_MS};
use crate::api::settings::{self, AndroidDisplayDraft, AndroidDisplayDto};
use crate::components::modal::confirm;

/// The adb port a new display starts with.
const DEFAULT_PORT: u16 = 5555;
/// The launch package a new display starts with.
const DEFAULT_COMPONENT: &str = "com.tcl.browser";

fn editor_spec(component: RwSignal<String>) -> EditorSpec {
    EditorSpec {
        role: "android",
        message_id: "android-form-status",
        message_role: "android-form-status",
        label_placeholder: "Stage Left",
        host_placeholder: "sd1l.lan",
        new_title: "New Android stage display",
        new_submit: "Add display",
        edit_title: "Edit Android stage display",
        extra: Some(ExtraField {
            caption: "Launch Package",
            role: "android-component",
            placeholder: DEFAULT_COMPONENT,
            value: component,
        }),
    }
}

#[component]
pub fn AndroidCard(toast: ToastHandle) -> impl IntoView {
    let displays = RwSignal::new(Vec::<AndroidDisplayDto>::new());
    let editing = RwSignal::new(EditTarget::Closed);
    let draft = ConnectionDraft::default();
    let component = RwSignal::new(String::from(DEFAULT_COMPONENT));
    let spec = editor_spec(component);

    let reload = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = settings::list_android_displays().await {
                displays.set(list);
            }
        });
    };
    // Initial load + 5s status poll.
    reload();
    gloo_timers::callback::Interval::new(STATUS_REFRESH_MS, reload).forget();

    let close_editor = move || {
        editing.set(EditTarget::Closed);
        draft.clear_message();
    };
    let open_new = move || {
        draft.reset(DEFAULT_PORT);
        component.set(DEFAULT_COMPONENT.to_string());
        editing.set(EditTarget::New);
    };
    let open_edit = move |id: String| {
        if let Some(d) = displays.with_untracked(|list| list.iter().find(|d| d.id == id).cloned()) {
            draft.load(&d.label, &d.host, d.port, d.is_enabled);
            component.set(d.launch_component.clone());
            editing.set(EditTarget::Item(d.id));
        }
    };

    let save = move || {
        let target = editing.get_untracked();
        if target == EditTarget::Closed || draft.busy.get_untracked() {
            return;
        }
        let fields = match draft.validated() {
            Ok(fields) => fields,
            Err(message) => {
                draft.show("error", message);
                return;
            }
        };
        let component_val = component.get_untracked().trim().to_string();
        if component_val.is_empty() {
            draft.show("error", "Launch component cannot be empty.");
            return;
        }
        let payload = AndroidDisplayDraft {
            label: fields.label,
            host: fields.host,
            port: fields.port,
            launch_component: component_val,
            is_enabled: draft.enabled.get_untracked(),
        };
        let updating = target.item_id().map(str::to_string);
        draft.busy.set(true);
        draft.show(
            "info",
            if updating.is_some() {
                "Updating display…"
            } else {
                "Creating display…"
            },
        );
        leptos::task::spawn_local(async move {
            let result = match &updating {
                Some(id) => settings::update_android_display(id, &payload).await,
                None => settings::create_android_display(&payload).await,
            };
            draft.busy.set(false);
            // The operator may have opened another editor meanwhile: leave that one alone.
            let still_open = editing.get_untracked() == target;
            match result {
                Ok(_) => {
                    if let Ok(list) = settings::list_android_displays().await {
                        displays.set(list);
                    }
                    if still_open {
                        close_editor();
                    }
                    toast.show(
                        if updating.is_some() {
                            "Saved Android stage display."
                        } else {
                            "Added Android stage display."
                        },
                        "success",
                    );
                }
                Err(err) => {
                    let message = format!("Unable to save display. {err}");
                    if still_open {
                        draft.show("error", &message);
                    }
                    toast.show(&message, "error");
                }
            }
        });
    };

    let delete_display = move |id: String| {
        let name = displays
            .with_untracked(|list| list.iter().find(|d| d.id == id).map(|d| d.label.clone()))
            .unwrap_or_else(|| "this display".to_string());
        if !confirm(&format!("Remove {name}? Presenter will stop reconnecting.")) {
            return;
        }
        leptos::task::spawn_local(async move {
            match settings::delete_android_display(&id).await {
                Ok(()) => {
                    if editing.with_untracked(|t| t.is_item(&id)) {
                        close_editor();
                    }
                    if let Ok(list) = settings::list_android_displays().await {
                        displays.set(list);
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

    let adding = move || editing.with(EditTarget::is_new);
    let each_display = move || displays.get();

    view! {
        <section class="settings__card">
            <header class="settings__card-header">
                <div>
                    <h2>"Android Stage Launchers"</h2>
                    <p>"Keep each Android TV pinned to the stage display: Presenter reconnects and reopens the stage URL in the launch package whenever the device appears."</p>
                </div>
                <div class="settings__badge-group">
                    <span class="settings__badge" data-role="android-count">
                        {move || displays.get().len().to_string()}
                    </span>
                    <span class="settings__badge-label">"Displays"</span>
                </div>
            </header>
            <div class="settings__list-toolbar">
                <button type="button" class="settings__button settings__button--primary"
                    data-role="android-add" prop:disabled=adding
                    on:click=move |_| open_new()>"+ Add display"</button>
            </div>
            <ul class="settings__list" data-role="android-display-list">
                <Show when=adding>
                    <li class="settings__list-item" data-role="android-new-item" data-editing="true">
                        {render_connection_editor(spec, draft, true, save, close_editor)}
                    </li>
                </Show>
                <Show when=move || displays.with(Vec::is_empty) && !adding()>
                    <li class="settings__list-empty" data-role="android-empty">"No Android stage displays configured yet."</li>
                </Show>
                <For
                    each=each_display
                    // Identity only: `updated_at` changes on an edit, never on a launch
                    // status change, so the poll keeps every row (and an open editor) in place.
                    key=|d: &AndroidDisplayDto| (d.id.clone(), d.updated_at.clone())
                    children=move |d: AndroidDisplayDto| {
                        let edit_id = d.id.clone();
                        let editing_this = Memo::new(move |_| editing.with(|t| t.is_item(&edit_id)));
                        // This row's live launch status, re-read on every poll.
                        let status_id = d.id.clone();
                        let status = Memo::new(move |_| {
                            displays.with(|list| {
                                list.iter().find(|x| x.id == status_id).and_then(|x| x.status.clone())
                            })
                        });
                        let is_enabled = d.is_enabled;
                        let badge = move || status.with(|s| android_state(s.as_ref(), is_enabled));
                        let status_class = move || format!("settings__status settings__status--{}", badge().0);
                        let status_state = move || badge().0;
                        let status_label = move || badge().1;
                        let attempts = move || status.with(|s| android_attempts(s.as_ref()));
                        let warning = move || {
                            status.with(|s| s.as_ref().and_then(|s| s.last_error.clone())).map(|err| view! {
                                <p class="settings__list-meta settings__list-meta--warning"
                                    data-role="android-error">{format!("⚠ {err}")}</p>
                            })
                        };
                        let id = d.id.clone();
                        let label = d.label.clone();
                        let host = d.host.clone();
                        let port = d.port;
                        let launch_component = d.launch_component.clone();
                        let timestamps = updated_created(&d.updated_at, &d.created_at);
                        let summary = move || {
                            let (id_test, id_edit, id_delete) = (id.clone(), id.clone(), id.clone());
                            view! {
                                <div class="settings__list-summary">
                                    <div class="settings__list-primary">
                                        <div class="settings__list-title">
                                            <span class="settings__host-label">{label.clone()}</span>
                                        </div>
                                        <p class="settings__list-line">
                                            <span class="settings__host-addr">
                                                <code>{host.clone()}</code>
                                                <span class="settings__host-port">{format!(":{port}")}</span>
                                            </span>
                                            <span class=status_class data-role="android-status" data-state=status_state>
                                                {status_label}
                                            </span>
                                            <span class="settings__list-aside" data-role="android-component-value"
                                                title="Launch package">{launch_component.clone()}</span>
                                        </p>
                                        <p class="settings__list-meta settings__list-meta--muted"
                                            data-role="android-attempts">{attempts}</p>
                                        <p class="settings__list-meta settings__list-meta--muted">
                                            {timestamps.clone()}
                                        </p>
                                        {warning}
                                    </div>
                                    <div class="settings__list-actions">
                                        <button type="button" class="settings__button settings__button--ghost settings__button--small"
                                            data-role="android-test" data-id=id_test.clone()
                                            on:click=move |_| test_display(id_test.clone())>"Test"</button>
                                        <button type="button" class="settings__button settings__button--ghost settings__button--small"
                                            data-role="android-edit" data-id=id_edit.clone()
                                            on:click=move |_| open_edit(id_edit.clone())>"Edit"</button>
                                        <button type="button" class="settings__button settings__button--danger settings__button--small"
                                            data-role="android-delete" data-id=id_delete.clone()
                                            on:click=move |_| delete_display(id_delete.clone())>"Delete"</button>
                                    </div>
                                </div>
                            }
                        };
                        view! {
                            <li class="settings__list-item" data-id=d.id.clone()
                                data-enabled=d.is_enabled.to_string()
                                data-editing=move || editing_this.get().to_string()>
                                {move || if editing_this.get() {
                                    render_connection_editor(spec, draft, false, save, close_editor)
                                } else {
                                    summary().into_any()
                                }}
                            </li>
                        }
                    }
                />
            </ul>
        </section>
    }
}
