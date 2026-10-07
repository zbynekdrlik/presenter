//! Resolume Arena connections card for the settings page (#347).
//!
//! #819: **Edit** opens the editor IN the clicked row and "+ Add connection" opens
//! it at the top of the list (`host_editor`). Rows are keyed on identity (id +
//! `updated_at`, which only an edit changes) and read their live status through a
//! `Memo`, so the 5 s poll updates the badge / latency / warning in place and never
//! rebuilds a row or touches an open editor.

use leptos::prelude::*;

use super::host_editor::{render_connection_editor, ConnectionDraft, EditTarget, EditorSpec};
use super::row_status::{
    latency_text, resolume_state, resolume_warning, updated_created, HostWarning,
};
use super::{capitalize, ToastHandle, STATUS_REFRESH_MS};
use crate::api::settings::{self, ResolumeHostDraft, ResolumeHostDto};
use crate::components::modal::confirm;

/// The port a new connection starts with (Arena's default web-server port).
const DEFAULT_PORT: u16 = 8090;

const EDITOR: EditorSpec = EditorSpec {
    role: "host",
    message_id: "resolume-form-status",
    message_role: "form-status",
    label_placeholder: "Main Arena",
    host_placeholder: "resolume.lan",
    new_title: "New Resolume connection",
    new_submit: "Add connection",
    edit_title: "Edit Resolume connection",
    extra: None,
};

#[component]
pub fn ResolumeCard(toast: ToastHandle) -> impl IntoView {
    let hosts = RwSignal::new(Vec::<ResolumeHostDto>::new());
    let editing = RwSignal::new(EditTarget::Closed);
    let draft = ConnectionDraft::default();

    let reload = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = settings::list_resolume_hosts().await {
                hosts.set(list);
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
        editing.set(EditTarget::New);
    };
    let open_edit = move |id: String| {
        if let Some(h) = hosts.with_untracked(|list| list.iter().find(|h| h.id == id).cloned()) {
            draft.load(&h.label, &h.host, h.port, h.is_enabled);
            editing.set(EditTarget::Item(h.id));
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
        let payload = ResolumeHostDraft {
            label: fields.label,
            host: fields.host,
            port: fields.port,
            is_enabled: draft.enabled.get_untracked(),
        };
        let updating = target.item_id().map(str::to_string);
        draft.busy.set(true);
        draft.show(
            "info",
            if updating.is_some() {
                "Saving changes…"
            } else {
                "Creating connection…"
            },
        );
        leptos::task::spawn_local(async move {
            let result = match &updating {
                Some(id) => settings::update_resolume_host(id, &payload).await,
                None => settings::create_resolume_host(&payload).await,
            };
            draft.busy.set(false);
            // The operator may have opened another editor meanwhile: leave that one alone.
            let still_open = editing.get_untracked() == target;
            match result {
                Ok(_) => {
                    if let Ok(list) = settings::list_resolume_hosts().await {
                        hosts.set(list);
                    }
                    toast.show(
                        if updating.is_some() {
                            "Updated Resolume connection."
                        } else {
                            "Added Resolume connection."
                        },
                        "success",
                    );
                    if still_open {
                        close_editor();
                    }
                }
                Err(err) => {
                    let message = format!("Unable to save connection. {err}");
                    if still_open {
                        draft.show("error", &message);
                    } else {
                        toast.show(&message, "error");
                    }
                }
            }
        });
    };

    let delete_host = move |id: String| {
        let name = hosts
            .with_untracked(|list| list.iter().find(|h| h.id == id).map(|h| h.label.clone()))
            .unwrap_or_else(|| "this connection".to_string());
        if !confirm(&format!("Remove {name}? Presenter will stop reconnecting.")) {
            return;
        }
        leptos::task::spawn_local(async move {
            match settings::delete_resolume_host(&id).await {
                Ok(()) => {
                    if editing.with_untracked(|t| t.is_item(&id)) {
                        close_editor();
                    }
                    if let Ok(list) = settings::list_resolume_hosts().await {
                        hosts.set(list);
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
                        let latency = latency_text(result.latency_ms);
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

    // #808: the server no longer re-reads the composition on a timer, so a
    // clip edit in Arena is picked up through this button. One refresh at a
    // time: each one is a full composition fetch on Arena.
    let mapping_refreshing = RwSignal::new(false);
    let refresh_mapping = move |id: String| {
        if mapping_refreshing.get_untracked() {
            return;
        }
        mapping_refreshing.set(true);
        leptos::task::spawn_local(async move {
            match settings::refresh_resolume_mapping(&id).await {
                Ok(result) if result.success => {
                    toast.show(&mapping_refreshed_message(&result.missing_clips), "success");
                }
                Ok(result) => {
                    let err = result.error.unwrap_or_else(|| "unknown error".to_string());
                    toast.show(&format!("Mapping refresh failed: {err}"), "error");
                }
                Err(err) => toast.show(&format!("Mapping refresh failed: {err}"), "error"),
            }
            mapping_refreshing.set(false);
            if let Ok(list) = settings::list_resolume_hosts().await {
                hosts.set(list);
            }
        });
    };

    let adding = move || editing.with(EditTarget::is_new);
    let each_host = move || hosts.get();

    view! {
        <section class="settings__card">
            <header class="settings__card-header">
                <div>
                    <h2>"Resolume Arena Connections"</h2>
                    <p>"Define Resolume web servers Presenter should control."</p>
                    <p data-role="resolume-mapping-hint">
                        "Presenter reads each Arena composition when it connects. After you add, rename or delete clips in Arena, click Refresh mapping."
                    </p>
                </div>
                <div class="settings__badge-group">
                    <span class="settings__badge" data-role="host-count">
                        {move || hosts.get().len().to_string()}
                    </span>
                    <span class="settings__badge-label">"Hosts"</span>
                </div>
            </header>
            <div class="settings__list-toolbar">
                <button type="button" class="settings__button settings__button--primary"
                    data-role="host-add" prop:disabled=adding
                    on:click=move |_| open_new()>"+ Add connection"</button>
            </div>
            <ul class="settings__list" data-role="resolume-host-list">
                <Show when=adding>
                    <li class="settings__list-item" data-role="host-new-item" data-editing="true">
                        {render_connection_editor(EDITOR, draft, true, save, close_editor)}
                    </li>
                </Show>
                <Show when=move || hosts.with(Vec::is_empty) && !adding()>
                    <li class="settings__list-empty" data-role="host-empty">"No Resolume connections defined yet."</li>
                </Show>
                <For
                    each=each_host
                    // Identity only: `updated_at` changes on an edit, never on a status
                    // change, so the poll keeps every row (and an open editor) in place.
                    key=|h: &ResolumeHostDto| (h.id.clone(), h.updated_at.clone())
                    children=move |h: ResolumeHostDto| {
                        let edit_id = h.id.clone();
                        let editing_this = Memo::new(move |_| editing.with(|t| t.is_item(&edit_id)));
                        // This row's live status, re-read on every poll (ui skill: key on
                        // identity, read the state through a Memo).
                        let status_id = h.id.clone();
                        let status = Memo::new(move |_| {
                            hosts.with(|list| {
                                list.iter().find(|x| x.id == status_id).and_then(|x| x.status.clone())
                            })
                        });
                        let is_enabled = h.is_enabled;
                        let state = move || status.with(|s| resolume_state(s.as_ref(), is_enabled));
                        let status_class = move || format!("settings__status settings__status--{}", state());
                        let status_label = move || capitalize(&state());
                        let latency = move || {
                            status.with(|s| latency_text(s.as_ref().and_then(|s| s.last_latency_ms)))
                        };
                        let warning = move || {
                            status.with(|s| resolume_warning(s.as_ref(), is_enabled)).map(|w| match w {
                                HostWarning::Retrying(text) => view! {
                                    <p class="settings__list-meta settings__list-meta--warning"
                                        data-role="host-error-detail">{text}</p>
                                }.into_any(),
                                HostWarning::Error(text) => view! {
                                    <p class="settings__list-meta settings__list-meta--warning">{text}</p>
                                }.into_any(),
                            })
                        };
                        let id = h.id.clone();
                        let label = h.label.clone();
                        let host = h.host.clone();
                        let port = h.port;
                        let timestamps = updated_created(&h.updated_at, &h.created_at);
                        let summary = move || {
                            let (id_test, id_refresh) = (id.clone(), id.clone());
                            let (id_edit, id_delete) = (id.clone(), id.clone());
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
                                            <span class=status_class data-role="host-status" data-state=state>
                                                {status_label}
                                            </span>
                                            <span class="settings__list-aside" data-role="host-latency"
                                                title="Last response time">{latency}</span>
                                        </p>
                                        <p class="settings__list-meta settings__list-meta--muted">
                                            {timestamps.clone()}
                                        </p>
                                        {warning}
                                    </div>
                                    <div class="settings__list-actions">
                                        <button type="button" class="settings__button settings__button--ghost settings__button--small"
                                            data-role="host-test" data-id=id_test.clone()
                                            on:click=move |_| test_host(id_test.clone())>"Test"</button>
                                        <button type="button" class="settings__button settings__button--ghost settings__button--small"
                                            data-role="host-refresh-mapping" data-id=id_refresh.clone()
                                            title="Re-read the Arena composition after editing clips in Arena"
                                            prop:disabled=move || mapping_refreshing.get()
                                            on:click=move |_| refresh_mapping(id_refresh.clone())>"Refresh mapping"</button>
                                        <button type="button" class="settings__button settings__button--ghost settings__button--small"
                                            data-role="host-edit" data-id=id_edit.clone()
                                            on:click=move |_| open_edit(id_edit.clone())>"Edit"</button>
                                        <button type="button" class="settings__button settings__button--danger settings__button--small"
                                            data-role="host-delete" data-id=id_delete.clone()
                                            on:click=move |_| delete_host(id_delete.clone())>"Delete"</button>
                                    </div>
                                </div>
                            }
                        };
                        view! {
                            <li class="settings__list-item" data-id=h.id.clone()
                                data-enabled=h.is_enabled.to_string()
                                data-editing=move || editing_this.get().to_string()>
                                {move || if editing_this.get() {
                                    render_connection_editor(EDITOR, draft, false, save, close_editor)
                                } else {
                                    summary().into_any()
                                }}
                            </li>
                        }
                    }
                />
            </ul>
            // #697: static reference of the special clip-name conventions
            // Presenter supports and what each one does. Compiled-in (derived
            // from `presenter-server`'s resolume::clip_map), NOT a live fetch —
            // it renders identically whether a Resolume host is connected or
            // not, so an operator can always discover which magic clip names
            // exist (the discoverability the pre-WASM settings page had and the
            // #347 Leptos migration dropped). Reuses the existing
            // `.settings__legend` styling.
            <section class="settings__legend" data-role="resolume-clip-legend">
                <h3>"Supported Clip Names"</h3>
                <p class="settings__legend-note">
                    "Presenter automatically updates every Resolume clip whose name contains one of these tokens (for example, #main-a or #main-a-2) and alternates between the A/B lanes so the next look is always preloaded. This list is compiled into Presenter — it is the same whether or not a Resolume host is currently connected."
                </p>
                <dl>
                    <div><dt>"#main-a / #main-b"</dt><dd>"Main lyric text, alternating between A and B for seamless cuts."</dd></div>
                    <div><dt>"#translate-a / #translate-b"</dt><dd>"Translation lyric text matched to each lane."</dd></div>
                    <div><dt>"#bible-a / #bible-b"</dt><dd>"Bible verse text with verse numbers."</dd></div>
                    <div><dt>"#bible-reference-a / #bible-reference-b"</dt><dd>"Bible reference with translation code (e.g. \"1 Samuel 1:4-5 (ROH)\")."</dd></div>
                    <div><dt>"#bible-translate-a / #bible-translate-b"</dt><dd>"Secondary-translation verse text, or empty if no secondary translation is configured."</dd></div>
                    <div><dt>"#bible-translate-reference-a / #bible-translate-reference-b"</dt><dd>"Secondary-translation reference with its translation code."</dd></div>
                    <div><dt>"#bible-clear"</dt><dd>"Fired when a Bible verse is cleared — blanks the Bible clips and triggers this clip."</dd></div>
                    <div><dt>"#timer"</dt><dd>"Receives the running countdown / preach timer text."</dd></div>
                    <div><dt>"#song-name"</dt><dd>"The active song title (numeric prefixes like \"001 \" are removed automatically)."</dd></div>
                    <div><dt>"#band-name"</dt><dd>"The library the current song belongs to."</dd></div>
                    <div><dt>"Suffixes: -u / -re"</dt><dd>"Append -u to force uppercase, or -re to collapse multi-line text into a single line."</dd></div>
                    <div><dt>"Also accepted"</dt><dd>"Aliases: #translation (= #translate), #bibleclear (= #bible-clear); suffixes -upper (= -u) and -noenter / -singleline (= -re). Use the canonical forms above when naming new clips."</dd></div>
                </dl>
            </section>
        </section>
    }
}

/// #808: the success toast after "Refresh mapping": names the clips the
/// refreshed composition still lacks, so the operator sees at once whether an
/// edit in Arena was picked up.
fn mapping_refreshed_message(missing_clips: &[String]) -> String {
    match missing_clips.len() {
        0 => "Mapping refreshed — every clip found.".to_string(),
        1 => format!("Mapping refreshed — 1 clip missing: {}", missing_clips[0]),
        n => format!(
            "Mapping refreshed — {n} clips missing: {}",
            missing_clips.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::mapping_refreshed_message;

    #[test]
    fn a_complete_mapping_says_every_clip_was_found() {
        assert_eq!(
            mapping_refreshed_message(&[]),
            "Mapping refreshed — every clip found."
        );
    }

    #[test]
    fn missing_clips_are_counted_and_named() {
        assert_eq!(
            mapping_refreshed_message(&["#timer".to_string()]),
            "Mapping refreshed — 1 clip missing: #timer"
        );
        assert_eq!(
            mapping_refreshed_message(&["#timer".to_string(), "#song-name".to_string()]),
            "Mapping refreshed — 2 clips missing: #timer, #song-name"
        );
    }
}
