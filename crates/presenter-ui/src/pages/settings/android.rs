//! Android stage launchers card for the settings page (#347).
//!
//! #819: **Edit** opens the editor IN the clicked row and "+ Add display" opens it
//! at the top of the list (`host_editor`, shared with the Resolume card). Rows are
//! keyed on identity plus the fields they show or edit ([`row_key`]) and read their
//! live launch status and timestamps through a `Memo`, so the 5 s poll never rebuilds
//! a row or touches an open editor.

use leptos::prelude::*;

use super::host_editor::{
    focus_on_close, render_connection_editor, EditTarget, EditorSpec, ExtraField, ListEditor,
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

/// A row's `<For>` key: its id plus every field the row shows or edits — never the
/// live launch status, so a poll never rebuilds a row.
fn row_key(d: &AndroidDisplayDto) -> (String, String, String, u16, String, bool) {
    (
        d.id.clone(),
        d.label.clone(),
        d.host.clone(),
        d.port,
        d.launch_component.clone(),
        d.is_enabled,
    )
}

#[component]
pub fn AndroidCard(toast: ToastHandle) -> impl IntoView {
    let displays = RwSignal::new(Vec::<AndroidDisplayDto>::new());
    let editor = ListEditor::default();
    let component = RwSignal::new(String::from(DEFAULT_COMPONENT));
    let spec = editor_spec(component);

    // Every list refresh goes through here, so an editor left open on a display that
    // was deleted meanwhile (here or in another tab) closes with it.
    let apply = move |list: Vec<AndroidDisplayDto>| {
        editor.forget_missing(list.iter().map(|d| d.id.as_str()));
        displays.set(list);
    };
    let reload = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = settings::list_android_displays().await {
                apply(list);
            }
        });
    };
    // Initial load + 5s status poll.
    reload();
    gloo_timers::callback::Interval::new(STATUS_REFRESH_MS, reload).forget();

    let open_new = move || {
        component.set(DEFAULT_COMPONENT.to_string());
        editor.open_new(DEFAULT_PORT);
    };
    let open_edit = move |id: String| {
        if let Some(d) = displays.with_untracked(|list| list.iter().find(|d| d.id == id).cloned()) {
            component.set(d.launch_component.clone());
            editor.open_item(d.id, &d.label, &d.host, d.port, d.is_enabled);
        }
    };

    let save = move || {
        let Some((ticket, fields)) = editor.begin_save() else {
            return;
        };
        let component_val = component.get_untracked().trim().to_string();
        if component_val.is_empty() {
            editor
                .draft
                .show("error", "Launch component cannot be empty.");
            return;
        }
        let payload = AndroidDisplayDraft {
            label: fields.label,
            host: fields.host,
            port: fields.port,
            launch_component: component_val,
            is_enabled: editor.draft.enabled.get_untracked(),
        };
        let updating = ticket.updating();
        editor.mark_saving(if updating.is_some() {
            "Updating display…"
        } else {
            "Creating display…"
        });
        leptos::task::spawn_local(async move {
            let result = match &updating {
                Some(id) => settings::update_android_display(id, &payload).await,
                None => settings::create_android_display(&payload).await,
            };
            match result {
                Ok(_) => {
                    if let Ok(list) = settings::list_android_displays().await {
                        apply(list);
                    }
                    toast.show(
                        if updating.is_some() {
                            "Saved Android stage display."
                        } else {
                            "Added Android stage display."
                        },
                        "success",
                    );
                    editor.finish_save(&ticket, None);
                }
                Err(err) => {
                    let message = format!("Unable to save display. {err}");
                    if !editor.finish_save(&ticket, Some(&message)) {
                        toast.show(&message, "error");
                    }
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
                    editor.discard_if_open_on(&id);
                    if let Ok(list) = settings::list_android_displays().await {
                        apply(list);
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
                        apply(list);
                    }
                }
                Err(err) => toast.show(&format!("Unable to trigger launch. {err}"), "error"),
            }
        });
    };

    let adding = move || editor.is_new();
    let each_display = move || displays.get();
    let add_ref = NodeRef::<leptos::html::Button>::new();
    focus_on_close(editor, add_ref, EditTarget::New);

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
                    node_ref=add_ref data-role="android-add" prop:disabled=adding
                    on:click=move |_| open_new()>"+ Add display"</button>
            </div>
            <ul class="settings__list" data-role="android-display-list">
                <Show when=adding>
                    <li class="settings__list-item" data-role="android-new-item" data-editing="true">
                        {render_connection_editor(spec, editor, true, save)}
                    </li>
                </Show>
                <Show when=move || displays.with(Vec::is_empty) && !adding()>
                    <li class="settings__list-empty" data-role="android-empty">"No Android stage displays configured yet."</li>
                </Show>
                <For
                    each=each_display
                    key=row_key
                    children=move |d: AndroidDisplayDto| {
                        let edit_id = d.id.clone();
                        let editing_this = Memo::new(move |_| editor.is_open_on(&edit_id));
                        // This row's live launch status and timestamps, re-read on every poll.
                        let status_id = d.id.clone();
                        let status = Memo::new(move |_| {
                            displays.with(|list| {
                                list.iter().find(|x| x.id == status_id).and_then(|x| x.status.clone())
                            })
                        });
                        let meta_id = d.id.clone();
                        let timestamps = Memo::new(move |_| {
                            displays.with(|list| {
                                list.iter()
                                    .find(|x| x.id == meta_id)
                                    .map(|x| updated_created(&x.updated_at, &x.created_at))
                                    .unwrap_or_default()
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
                        let summary = move || {
                            let (id_test, id_edit, id_delete) = (id.clone(), id.clone(), id.clone());
                            let edit_ref = NodeRef::<leptos::html::Button>::new();
                            focus_on_close(editor, edit_ref, EditTarget::Item(id.clone()));
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
                                            {move || timestamps.get()}
                                        </p>
                                        {warning}
                                    </div>
                                    <div class="settings__list-actions">
                                        <button type="button" class="settings__button settings__button--ghost settings__button--small"
                                            data-role="android-test" data-id=id_test.clone()
                                            on:click=move |_| test_display(id_test.clone())>"Test"</button>
                                        <button type="button" class="settings__button settings__button--ghost settings__button--small"
                                            node_ref=edit_ref data-role="android-edit" data-id=id_edit.clone()
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
                                    render_connection_editor(spec, editor, false, save)
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

#[cfg(test)]
mod tests {
    use super::row_key;
    use crate::api::settings::{AndroidDisplayDto, AndroidStatusDto};

    fn display(state: &str, updated_at: &str) -> AndroidDisplayDto {
        AndroidDisplayDto {
            id: "d1".into(),
            label: "Stage Left".into(),
            host: "sd1l.lan".into(),
            port: 5555,
            launch_component: "com.tcl.browser".into(),
            is_enabled: true,
            created_at: "c".into(),
            updated_at: updated_at.into(),
            status: Some(AndroidStatusDto {
                state: state.into(),
                last_attempt: None,
                last_success: None,
                last_error: None,
            }),
        }
    }

    #[test]
    fn a_launch_status_change_keeps_the_row() {
        assert_eq!(
            row_key(&display("connecting", "t1")),
            row_key(&display("running", "t2"))
        );
    }

    #[test]
    fn every_shown_or_edited_field_re_keys_the_row() {
        let base = display("running", "t1");
        let changed = [
            AndroidDisplayDto {
                label: "Stage Right".into(),
                ..base.clone()
            },
            AndroidDisplayDto {
                host: "sd1r.lan".into(),
                ..base.clone()
            },
            AndroidDisplayDto {
                port: 5556,
                ..base.clone()
            },
            AndroidDisplayDto {
                launch_component: "com.example/.Main".into(),
                ..base.clone()
            },
            AndroidDisplayDto {
                is_enabled: false,
                ..base.clone()
            },
        ];
        for edited in &changed {
            assert_ne!(row_key(&base), row_key(edited));
        }
    }
}
