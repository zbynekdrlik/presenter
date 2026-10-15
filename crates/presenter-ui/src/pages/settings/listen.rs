//! Settings → "Počúvanie" (#852 C1b, spec §5.6): the listening microphones on
//! the shared `list_card::ListCard`, edited in place by `listen_form::MicForm`
//! (the editor, its option captions and the new mic's id line). The API is
//! the whole list (`GET|PUT /listen/mics`): a save or a delete reads the list,
//! changes one mic and writes the list back ([`apply_draft`]) — one round trip
//! apart, in one Settings card, for at most 4 mics; the server validates the
//! whole list and its Slovak 422 rule shows in the editor. A new mic's id comes
//! from its role (`mic_id_for_role`: `guest`, `interp`, `preacher`, the spec
//! §5.6 ids) and never changes: it names the mic's recordings. All copy is
//! Slovak.

use leptos::prelude::*;
use presenter_core::listen::mics::{mic_id_for_role, MicChannel, MicConfig, MicLanguage, MicRole};

use super::host_editor::EditorSpec;
use super::list_card::{CardItem, CardText, ListCard, RowParts};
use super::listen_form::{mic_aside, MicDraft, MicForm, TakenIds};
use super::ToastHandle;
use crate::api::{listen, ApiError};

static TEXT: CardText<MicForm> = CardText {
    editor: EditorSpec {
        role: "listen",
        message_id: "listen-form-status",
        message_role: "listen-form-status",
        new_title: "Nový mikrofón",
        new_submit: "Pridať mikrofón",
        edit_title: "Upraviť mikrofón",
        save_text: "Uložiť",
        cancel_text: "Zrušiť",
        unsaved_title: "Najprv ulož alebo zruš otvorenú úpravu",
        saving_title: "Počkaj, kým sa uloženie dokončí",
    },
    form: (),
    list_role: "listen-mic-list",
    add_text: "+ Pridať mikrofón",
    empty_text: "Zatiaľ žiadny mikrofón — Presenter nepočúva a neotvorí žiadny port.",
    fallback_name: "tento mikrofón",
    edit_text: "Upraviť",
    delete_text: "Odstrániť",
    delete_question: ("Odstrániť mikrofón „", "“? Presenter ho prestane počúvať."),
    saving: "Ukladám mikrofón…",
    creating: "Pridávam mikrofón…",
    updated: "Mikrofón je uložený.",
    added: "Mikrofón je pridaný.",
    deleted: "Mikrofón je odstránený.",
    save_failed: "Mikrofón sa nepodarilo uložiť.",
    delete_failed: "Mikrofón sa nepodarilo odstrániť.",
    removed_elsewhere: "Tento mikrofón medzitým niekto odstránil.",
};

type MicKey = (
    String,
    String,
    String,
    Option<String>,
    MicChannel,
    MicLanguage,
    MicRole,
);

/// A row's `<For>` key: the id plus every field the row shows or edits — never
/// live health (a poll must not rebuild a row).
fn row_key(mic: &MicConfig) -> MicKey {
    (
        mic.id.clone(),
        mic.name.clone(),
        mic.stream_name.clone(),
        mic.sender_ip.clone(),
        mic.channel,
        mic.language,
        mic.role,
    )
}

impl CardItem for MicConfig {
    type Form = MicForm;
    /// Health is the card's own business (C1b-7), not the list's.
    type Status = ();
    type Key = MicKey;

    fn id(&self) -> &str {
        &self.id
    }

    fn key(&self) -> MicKey {
        row_key(self)
    }

    fn label(&self) -> &str {
        &self.name
    }

    fn address(&self) -> (String, String) {
        let from = match &self.sender_ip {
            Some(ip) => format!(" · od {ip}"),
            None => " · od ktoréhokoľvek odosielateľa".to_string(),
        };
        (self.stream_name.clone(), from)
    }

    fn values(&self) -> MicDraft {
        MicDraft::of(self)
    }

    fn status(&self) -> Option<()> {
        None
    }

    fn timestamps(&self) -> Option<String> {
        None
    }

    async fn list() -> Result<Vec<Self>, ApiError> {
        listen::get_mics().await
    }

    async fn save(id: Option<String>, draft: MicDraft) -> Result<(), ApiError> {
        let mics = listen::get_mics().await?;
        let mics = apply_draft(mics, id.as_deref(), draft)
            .ok_or_else(|| ApiError::NotFound(TEXT.removed_elsewhere.to_string()))?;
        listen::put_mics(&mics).await.map(drop)
    }

    async fn delete(id: String) -> Result<(), ApiError> {
        let mut mics = listen::get_mics().await?;
        mics.retain(|mic| mic.id != id);
        listen::put_mics(&mics).await.map(drop)
    }
}

/// `mics` with `draft` applied: the mic `id` changed in place (its id kept), or
/// a new mic appended with an id from its role; `None` when `id` is gone (a mic
/// removed meanwhile is never added back).
fn apply_draft(
    mut mics: Vec<MicConfig>,
    id: Option<&str>,
    draft: MicDraft,
) -> Option<Vec<MicConfig>> {
    match id {
        Some(id) => {
            let mic = mics.iter_mut().find(|mic| mic.id == id)?;
            *mic = draft.into_config(id.to_string());
        }
        None => {
            let taken: Vec<&str> = mics.iter().map(|mic| mic.id.as_str()).collect();
            let new_id = mic_id_for_role(draft.role, &taken);
            mics.push(draft.into_config(new_id));
        }
    }
    Some(mics)
}

/// A mic row's own part: its language, role, channel and id.
fn mic_row(mic: &MicConfig) -> RowParts {
    RowParts {
        line: view! {
            <span class="settings__list-aside" data-role="listen-config">{mic_aside(mic)}</span>
        }
        .into_any(),
        meta: None,
        warning: ().into_any(),
        actions: ().into_any(),
    }
}

/// The Počúvanie card.
#[component]
pub fn ListenCard(toast: ToastHandle) -> impl IntoView {
    // The ids of the listed mics: the editor's id line for a new mic reads them
    // (`MicForm::new` takes the signal from this context).
    let taken = RwSignal::new(Vec::<String>::new());
    provide_context(TakenIds(taken));
    let card = ListCard::<MicConfig>::new(toast, &TEXT);
    let row = move |mic: &MicConfig, _: Memo<Option<()>>| mic_row(mic);
    let mics = card.items();
    Effect::new(move || {
        taken.set(mics.with(|list| list.iter().map(|mic| mic.id.clone()).collect()));
    });

    view! {
        <section class="settings__card" data-role="listen-card">
            <header class="settings__card-header">
                <div>
                    <h2>"Počúvanie"</h2>
                    <p>"Mikrofóny hosťa a tlmočníka cez VBAN (prepis kázne). Bez mikrofónu Presenter nepočúva a neotvorí žiadny port."</p>
                </div>
                <div class="settings__badge-group">
                    <span class="settings__badge" data-role="listen-count">
                        {move || mics.with(Vec::len).to_string()}
                    </span>
                    <span class="settings__badge-label">"Mikrofóny"</span>
                </div>
            </header>
            {card.render_list(row)}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mic(id: &str, name: &str, role: MicRole) -> MicConfig {
        MicConfig {
            id: id.into(),
            name: name.into(),
            stream_name: "GUEST_EN".into(),
            sender_ip: None,
            channel: MicChannel::Left,
            language: MicLanguage::En,
            role,
        }
    }

    fn draft(name: &str, ip: &str, role: MicRole) -> MicDraft {
        MicDraft {
            id: None,
            name: name.into(),
            stream_name: "INTERP_SK".into(),
            sender_ip: ip.into(),
            channel: MicChannel::Right,
            language: MicLanguage::Sk,
            role,
        }
    }

    #[test]
    fn a_save_changes_one_mic_in_place_and_a_new_one_gets_its_roles_id() {
        let mics = vec![
            mic("guest", "Hosť", MicRole::Guest),
            mic("interp", "Tlmočník", MicRole::Interpreter),
        ];
        let renamed = draft("Tlmočník booth", "192.0.2.10", MicRole::Interpreter);
        let changed = apply_draft(mics.clone(), Some("interp"), renamed).unwrap();
        assert_eq!(changed[0], mics[0], "the other mic is untouched");
        assert_eq!(changed[1].id, "interp", "the id never changes on a rename");
        assert_eq!(changed[1].name, "Tlmočník booth");
        assert_eq!(changed[1].sender_ip.as_deref(), Some("192.0.2.10"));
        let added = apply_draft(mics, None, draft("Druhý hosť", "", MicRole::Guest)).unwrap();
        assert_eq!(added.len(), 3);
        assert_eq!(
            added[2].id, "guest-2",
            "the role's id is taken: the next one"
        );
        assert_eq!(added[2].sender_ip, None, "a blank IP = any sender");
        let preacher = apply_draft(Vec::new(), None, draft("Kazateľ", "", MicRole::Preacher));
        assert_eq!(preacher.unwrap()[0].id, "preacher");
    }

    #[test]
    fn a_save_of_a_mic_removed_meanwhile_never_adds_it_back() {
        let mics = vec![mic("guest", "Hosť", MicRole::Guest)];
        assert_eq!(
            apply_draft(mics, Some("gone"), draft("X", "", MicRole::Guest)),
            None
        );
    }

    #[test]
    fn the_row_shows_the_id_and_its_key_every_field_it_shows_or_edits() {
        let base = mic("interp", "Tlmočník", MicRole::Interpreter);
        assert_eq!(
            mic_aside(&base),
            "angličtina · tlmočník · kanál ľavý (1) · id interp"
        );
        let mut renamed = base.clone();
        renamed.name = "Tlmočník 2".into();
        let mut mixed = base.clone();
        mixed.channel = MicChannel::Mix;
        assert_ne!(row_key(&base), row_key(&renamed));
        assert_ne!(row_key(&base), row_key(&mixed));
        assert_eq!(row_key(&base), row_key(&base.clone()));
    }
}
