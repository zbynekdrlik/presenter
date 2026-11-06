//! Resolume Arena connections card for the settings page (#347).
//!
//! #819: the list, its inline editor (Edit in the row, "+ Pridať pripojenie" at the top),
//! the fetch / save / delete flow and the keyed rows are the shared
//! `list_card::ListCard`. This file supplies the Resolume API and copy, the row's
//! own parts (status badge, latency, retry warning, Test / Refresh mapping) and the
//! clip-name legend. Rows are keyed on identity plus the fields they show or edit
//! ([`row_key`]), never on the live status, so the 5 s poll updates the badge in
//! place and never rebuilds a row or touches an open editor.

use leptos::prelude::*;

use super::connection_form::{
    ConnectionForm, ConnectionSpec, DraftValues, Submission, SAVING_TITLE, UNSAVED_TITLE,
};
use super::host_editor::EditorSpec;
use super::list_card::{CardItem, CardText, ListCard, RowParts};
use super::row_status::{
    latency_text, resolume_state, resolume_warning, state_label, updated_created, HostWarning,
};
use super::ToastHandle;
use crate::api::settings::{self, ResolumeHostDraft, ResolumeHostDto, ResolumeStatusDto};
use crate::api::ApiError;

static TEXT: CardText<ConnectionForm> = CardText {
    editor: EditorSpec {
        role: "host",
        message_id: "resolume-form-status",
        message_role: "form-status",
        new_title: "Nové pripojenie Resolume",
        new_submit: "Pridať pripojenie",
        edit_title: "Upraviť pripojenie Resolume",
        save_text: "Uložiť",
        cancel_text: "Zrušiť",
        unsaved_title: UNSAVED_TITLE,
        saving_title: SAVING_TITLE,
    },
    form: ConnectionSpec {
        label_placeholder: "Hlavná Arena",
        host_placeholder: "resolume.lan",
        // Arena's default web-server port.
        default_port: 8090,
    },
    list_role: "resolume-host-list",
    add_text: "+ Pridať pripojenie",
    empty_text: "Zatiaľ žiadne pripojenie Resolume.",
    fallback_name: "toto pripojenie",
    edit_text: "Upraviť",
    delete_text: "Odstrániť",
    delete_question: ("Odstrániť „", "“? Presenter sa naň prestane pripájať."),
    saving: "Ukladám zmeny…",
    creating: "Pridávam pripojenie…",
    updated: "Pripojenie Resolume je uložené.",
    added: "Pripojenie Resolume je pridané.",
    deleted: "Pripojenie Resolume je odstránené.",
    save_failed: "Pripojenie sa nepodarilo uložiť.",
    delete_failed: "Pripojenie sa nepodarilo odstrániť.",
    removed_elsewhere: "Toto pripojenie medzitým niekto odstránil.",
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
    type Form = ConnectionForm;
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

    fn address(&self) -> (String, String) {
        (self.host.clone(), format!(":{}", self.port))
    }

    fn values(&self) -> DraftValues {
        DraftValues {
            label: self.label.clone(),
            host: self.host.clone(),
            port: self.port.to_string(),
            enabled: self.is_enabled,
        }
    }

    fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    fn status(&self) -> Option<ResolumeStatusDto> {
        self.status.clone()
    }

    fn timestamps(&self) -> Option<String> {
        Some(updated_created(&self.updated_at, &self.created_at))
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
                        toast.show(&format!("Pripojenie funguje ({latency})"), "success");
                    } else {
                        let err = result.error.unwrap_or_else(|| "neznáma chyba".to_string());
                        toast.show(&format!("Pripojenie zlyhalo: {err}"), "error");
                    }
                }
                Err(err) => toast.show(&format!("Test zlyhal: {err}"), "error"),
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
                    let err = result.error.unwrap_or_else(|| "neznáma chyba".to_string());
                    toast.show(&format!("Načítanie klipov zlyhalo: {err}"), "error");
                }
                Err(err) => toast.show(&format!("Načítanie klipov zlyhalo: {err}"), "error"),
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
        let status_label = move || state_label(&state());
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
                        title="Posledný čas odozvy">{latency}</span>
                </>
            }
            .into_any(),
            meta: None,
            warning: warning.into_any(),
            actions: view! {
                <>
                    <button type="button" class="ui-button"
                        data-role="host-test" data-id=id_test.clone()
                        on:click=move |_| test_host(id_test.clone())>"Otestovať"</button>
                    <button type="button" class="ui-button"
                        data-role="host-refresh-mapping" data-id=id_refresh.clone()
                        title="Znova načítať kompozíciu z Areny po úprave klipov v Arene"
                        prop:disabled=move || mapping_refreshing.get()
                        on:click=move |_| refresh_mapping(id_refresh.clone())>"Načítať klipy"</button>
                </>
            }
            .into_any(),
        }
    };
    let hosts = card.items();

    view! {
        <section class="settings__card">
            <header class="settings__card-header">
                <div>
                    <h2>"Pripojenia Resolume Arena"</h2>
                    <p>"Webové servery Resolume, ktoré Presenter ovláda."</p>
                    <p data-role="resolume-mapping-hint">
                        "Presenter načíta kompozíciu každej Areny pri pripojení. Keď v Arene pridáš, premenuješ alebo zmažeš klip, klikni na Načítať klipy."
                    </p>
                </div>
                <div class="settings__badge-group">
                    <span class="settings__badge" data-role="host-count">
                        {move || hosts.with(Vec::len).to_string()}
                    </span>
                    <span class="settings__badge-label">"Pripojenia"</span>
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
                <h3>"Podporované názvy klipov"</h3>
                <p class="settings__legend-note">
                    "Presenter sám aktualizuje každý klip Resolume, ktorého názov obsahuje jednu z týchto značiek (napríklad #main-a alebo #main-a-2), a strieda pruhy A / B, aby bol ďalší obraz vždy pripravený. Zoznam je zabudovaný v Presenteri — je rovnaký, či je Resolume práve pripojené, alebo nie."
                </p>
                <dl>
                    <div><dt>"#main-a / #main-b"</dt><dd>"Hlavný text piesne, strieda A a B pre plynulé prechody."</dd></div>
                    <div><dt>"#translate-a / #translate-b"</dt><dd>"Text prekladu piesne k príslušnému pruhu."</dd></div>
                    <div><dt>"#bible-a / #bible-b"</dt><dd>"Text verša s číslami veršov."</dd></div>
                    <div><dt>"#bible-reference-a / #bible-reference-b"</dt><dd>"Odkaz na verš s kódom prekladu (napríklad „1 Samuel 1:4-5 (ROH)“)."</dd></div>
                    <div><dt>"#bible-translate-a / #bible-translate-b"</dt><dd>"Text verša v druhom preklade, alebo prázdny, keď druhý preklad nie je nastavený."</dd></div>
                    <div><dt>"#bible-translate-reference-a / #bible-translate-reference-b"</dt><dd>"Odkaz v druhom preklade s jeho kódom prekladu."</dd></div>
                    <div data-role="resolume-bible-clear-note"><dt>"#bible-clear (zmazanie verša)"</dt><dd>"Zmazanie verša nespustí žiadny klip: priehľadnosť každého klipu Biblie stlmí na 0 a klip so značkou #bible-clear (napríklad názov kázne) vráti na jeho priehľadnosť (predvolene plnú), počas času prechodu vrstvy každého klipu. Ďalší verš klipy Biblie obnoví a #bible-clear znova stlmí na 0."</dd></div>
                    <div><dt>"#timer"</dt><dd>"Dostáva bežiaci text odpočtu / časomiery kázne."</dd></div>
                    <div><dt>"#song-name"</dt><dd>"Názov aktuálnej piesne (číselné predpony ako „001 “ sa odstránia samy)."</dd></div>
                    <div><dt>"#band-name"</dt><dd>"Knižnica, do ktorej patrí aktuálna pieseň."</dd></div>
                    <div><dt>"Prípony: -u / -re"</dt><dd>"Prípona -u vynúti veľké písmená, -re spojí viacriadkový text do jedného riadku."</dd></div>
                    <div><dt>"Prijíma aj"</dt><dd>"Aliasy: #translation (= #translate); prípony -upper (= -u) a -noenter / -singleline (= -re). Nové klipy pomenúvaj tvarmi vyššie."</dd></div>
                </dl>
            </section>
        </section>
    }
}

/// #808: the success toast after "Načítať klipy" (Refresh mapping): names the clips the
/// refreshed composition still lacks, so the operator sees at once whether an
/// edit in Arena was picked up.
fn mapping_refreshed_message(missing_clips: &[String]) -> String {
    match missing_clips.len() {
        0 => "Klipy sú načítané — všetky nájdené.".to_string(),
        1 => format!("Klipy sú načítané — chýba 1 klip: {}", missing_clips[0]),
        n @ 2..=4 => format!(
            "Klipy sú načítané — chýbajú {n} klipy: {}",
            missing_clips.join(", ")
        ),
        n => format!(
            "Klipy sú načítané — chýba {n} klipov: {}",
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
            "Klipy sú načítané — všetky nájdené."
        );
    }

    #[test]
    fn missing_clips_are_counted_and_named() {
        assert_eq!(
            mapping_refreshed_message(&["#timer".to_string()]),
            "Klipy sú načítané — chýba 1 klip: #timer"
        );
        assert_eq!(
            mapping_refreshed_message(&["#timer".to_string(), "#song-name".to_string()]),
            "Klipy sú načítané — chýbajú 2 klipy: #timer, #song-name"
        );
        let five: Vec<String> = ["#a", "#b", "#c", "#d", "#e"].map(String::from).to_vec();
        assert_eq!(
            mapping_refreshed_message(&five),
            "Klipy sú načítané — chýba 5 klipov: #a, #b, #c, #d, #e"
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
