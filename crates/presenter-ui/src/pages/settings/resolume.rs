//! Resolume Arena connections card for the settings page (#347).
//!
//! #819: the list, its inline editor (Edit in the row, "+ Add connection" at the top),
//! the fetch / save / delete flow and the keyed rows are the shared
//! `list_card::ListCard`. This file supplies the Resolume API and copy, the row's
//! own parts (status badge, latency, retry warning, Test / Refresh mapping) and the
//! clip-name legend. Rows are keyed on identity plus the fields they show or edit
//! ([`row_key`]), never on the live status, so the 5 s poll updates the badge in
//! place and never rebuilds a row or touches an open editor.

use leptos::prelude::*;

use super::host_editor::{EditorSpec, Submission};
use super::list_card::{CardItem, CardText, ListCard, RowParts};
use super::row_status::{latency_text, resolume_state, resolume_warning, HostWarning};
use super::{capitalize, ToastHandle};
use crate::api::settings::{self, ResolumeHostDraft, ResolumeHostDto, ResolumeStatusDto};
use crate::api::ApiError;

static TEXT: CardText = CardText {
    editor: EditorSpec {
        role: "host",
        message_id: "resolume-form-status",
        message_role: "form-status",
        label_placeholder: "Main Arena",
        host_placeholder: "resolume.lan",
        new_title: "New Resolume connection",
        new_submit: "Add connection",
        edit_title: "Edit Resolume connection",
        extra: None,
    },
    // Arena's default web-server port.
    default_port: 8090,
    list_role: "resolume-host-list",
    add_text: "+ Add connection",
    empty_text: "No Resolume connections defined yet.",
    fallback_name: "this connection",
    saving: "Saving changes…",
    creating: "Creating connection…",
    updated: "Updated Resolume connection.",
    added: "Added Resolume connection.",
    deleted: "Deleted Resolume connection.",
    save_failed: "Unable to save connection.",
    delete_failed: "Unable to delete connection.",
    removed_elsewhere: "This connection was removed elsewhere.",
};

/// A row's `<For>` key: its id plus every field the row shows or edits. Never the
/// live status (a poll must not rebuild a row) and never `updated_at` (#564 port-drift
/// discovery bumps it in the background, mid-edit).
fn row_key(h: &ResolumeHostDto) -> (String, String, String, u16, bool) {
    (
        h.id.clone(),
        h.label.clone(),
        h.host.clone(),
        h.port,
        h.is_enabled,
    )
}

impl CardItem for ResolumeHostDto {
    type Status = ResolumeStatusDto;
    type Key = (String, String, String, u16, bool);

    fn id(&self) -> &str {
        &self.id
    }

    fn key(&self) -> Self::Key {
        row_key(self)
    }

    fn label(&self) -> &str {
        &self.label
    }

    fn host(&self) -> &str {
        &self.host
    }

    fn port(&self) -> u16 {
        self.port
    }

    fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    fn status(&self) -> Option<ResolumeStatusDto> {
        self.status.clone()
    }

    fn created_at(&self) -> &str {
        &self.created_at
    }

    fn updated_at(&self) -> &str {
        &self.updated_at
    }

    async fn list() -> Result<Vec<Self>, ApiError> {
        settings::list_resolume_hosts().await
    }

    async fn save(id: Option<String>, submission: Submission) -> Result<(), ApiError> {
        let draft = ResolumeHostDraft {
            label: submission.fields.label,
            host: submission.fields.host,
            port: submission.fields.port,
            is_enabled: submission.enabled,
        };
        match id {
            Some(id) => settings::update_resolume_host(&id, &draft).await.map(drop),
            None => settings::create_resolume_host(&draft).await.map(drop),
        }
    }

    async fn delete(id: String) -> Result<(), ApiError> {
        settings::delete_resolume_host(&id).await
    }
}

#[component]
pub fn ResolumeCard(toast: ToastHandle) -> impl IntoView {
    let card = ListCard::<ResolumeHostDto>::new(toast, &TEXT);

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
            card.fetch().await;
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
            card.fetch().await;
        });
    };

    // The Resolume parts of a row: status badge + latency, the retry warning, and
    // Test / Refresh mapping before the shared Edit / Delete.
    let row = move |h: &ResolumeHostDto, status: Memo<Option<ResolumeStatusDto>>| {
        let is_enabled = h.is_enabled;
        let state = move || status.with(|s| resolume_state(s.as_ref(), is_enabled));
        let status_class = move || format!("settings__status settings__status--{}", state());
        let status_label = move || capitalize(&state());
        let latency =
            move || status.with(|s| latency_text(s.as_ref().and_then(|s| s.last_latency_ms)));
        let warning = move || {
            status
                .with(|s| resolume_warning(s.as_ref(), is_enabled))
                .map(|w| match w {
                    HostWarning::Retrying(text) => view! {
                        <p class="settings__list-meta settings__list-meta--warning"
                            data-role="host-error-detail">{text}</p>
                    }
                    .into_any(),
                    HostWarning::Error(text) => view! {
                        <p class="settings__list-meta settings__list-meta--warning">{text}</p>
                    }
                    .into_any(),
                })
        };
        let (id_test, id_refresh) = (h.id.clone(), h.id.clone());
        RowParts {
            line: view! {
                <>
                    <span class=status_class data-role="host-status" data-state=state>
                        {status_label}
                    </span>
                    <span class="settings__list-aside" data-role="host-latency"
                        title="Last response time">{latency}</span>
                </>
            }
            .into_any(),
            meta: None,
            warning: warning.into_any(),
            actions: view! {
                <>
                    <button type="button" class="settings__button settings__button--ghost settings__button--small"
                        data-role="host-test" data-id=id_test.clone()
                        on:click=move |_| test_host(id_test.clone())>"Test"</button>
                    <button type="button" class="settings__button settings__button--ghost settings__button--small"
                        data-role="host-refresh-mapping" data-id=id_refresh.clone()
                        title="Re-read the Arena composition after editing clips in Arena"
                        prop:disabled=move || mapping_refreshing.get()
                        on:click=move |_| refresh_mapping(id_refresh.clone())>"Refresh mapping"</button>
                </>
            }
            .into_any(),
        }
    };
    let hosts = card.items;

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
            {card.render_list(row)}
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
    use super::{mapping_refreshed_message, row_key};
    use crate::api::settings::{ResolumeHostDto, ResolumeStatusDto};

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

    fn host(state: &str, updated_at: &str) -> ResolumeHostDto {
        ResolumeHostDto {
            id: "h1".into(),
            label: "Main Arena".into(),
            host: "resolume.lan".into(),
            port: 8090,
            is_enabled: true,
            created_at: "c".into(),
            updated_at: updated_at.into(),
            status: Some(ResolumeStatusDto {
                state: state.into(),
                last_latency_ms: None,
                last_error: None,
                consecutive_failures: 0,
                error_since: None,
            }),
        }
    }

    #[test]
    fn a_status_change_or_a_background_updated_at_bump_keeps_the_row() {
        // #819: the poll's status flip and #564's port-drift `updated_at` write must
        // not rebuild the row (and re-focus an open editor).
        assert_eq!(
            row_key(&host("connected", "t1")),
            row_key(&host("error", "t2"))
        );
    }

    #[test]
    fn every_shown_or_edited_field_re_keys_the_row() {
        let base = host("connected", "t1");
        let changed = [
            ResolumeHostDto {
                label: "Other".into(),
                ..base.clone()
            },
            ResolumeHostDto {
                host: "other.lan".into(),
                ..base.clone()
            },
            ResolumeHostDto {
                port: 8091,
                ..base.clone()
            },
            ResolumeHostDto {
                is_enabled: false,
                ..base.clone()
            },
        ];
        for edited in &changed {
            assert_ne!(row_key(&base), row_key(edited));
        }
    }
}
