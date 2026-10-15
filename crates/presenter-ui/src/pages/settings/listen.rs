//! Settings → "Počúvanie" (#852 C1b, spec §5.6): the listening microphones on
//! the shared `list_card::ListCard`, edited in place by [`MicForm`]. The API is
//! the whole list (`GET|PUT /listen/mics`): a save or a delete reads the list,
//! changes one mic and writes the list back ([`apply_draft`]) — one round trip
//! apart, in one Settings card, for at most 4 mics; the server validates the
//! whole list and its Slovak 422 rule shows in the editor. A new mic's id comes
//! from its role (`mic_id_for_role`: `guest`, `interp`, `preacher`, the spec
//! §5.6 ids) and never changes: it names the mic's recordings. All copy is
//! Slovak.

use leptos::prelude::*;
use presenter_core::listen::mics::{mic_id_for_role, MicChannel, MicConfig, MicLanguage, MicRole};

use super::host_editor::{EditorForm, EditorSpec};
use super::list_card::{CardItem, CardText, ListCard, RowParts};
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

/// (value, the wire name = the `<option>` value, the caption).
const CHANNELS: &[(MicChannel, &str, &str)] = &[
    (MicChannel::Left, "left", "ľavý (1)"),
    (MicChannel::Right, "right", "pravý (2)"),
    (MicChannel::Mix, "mix", "zmes"),
];
const LANGUAGES: &[(MicLanguage, &str, &str)] = &[
    (MicLanguage::En, "en", "angličtina"),
    (MicLanguage::Sk, "sk", "slovenčina"),
    (MicLanguage::Off, "off", "neprepisovať"),
];
const ROLES: &[(MicRole, &str, &str)] = &[
    (MicRole::Guest, "guest", "hosť"),
    (MicRole::Interpreter, "interpreter", "tlmočník"),
    (MicRole::Preacher, "preacher", "kazateľ"),
];

/// What the microphone editor holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MicDraft {
    name: String,
    stream_name: String,
    /// As typed; blank = any sender.
    sender_ip: String,
    channel: MicChannel,
    language: MicLanguage,
    role: MicRole,
}

impl Default for MicDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            stream_name: String::new(),
            sender_ip: String::new(),
            channel: MicChannel::Left,
            language: MicLanguage::En,
            role: MicRole::Guest,
        }
    }
}

impl MicDraft {
    fn of(mic: &MicConfig) -> Self {
        Self {
            name: mic.name.clone(),
            stream_name: mic.stream_name.clone(),
            sender_ip: mic.sender_ip.clone().unwrap_or_default(),
            channel: mic.channel,
            language: mic.language,
            role: mic.role,
        }
    }

    /// The mic with this draft's values; `id` names its recordings and stays.
    fn into_config(self, id: String) -> MicConfig {
        MicConfig {
            id,
            name: self.name,
            stream_name: self.stream_name,
            sender_ip: Some(self.sender_ip).filter(|ip| !ip.is_empty()),
            channel: self.channel,
            language: self.language,
            role: self.role,
        }
    }
}

/// The editor's own checks: a name and a stream name. Every other rule is the
/// server's (its 422 text shows in the same line).
fn validate_mic(values: &MicDraft) -> Result<MicDraft, &'static str> {
    let name = values.name.trim();
    if name.is_empty() {
        return Err("Zadaj názov mikrofónu.");
    }
    let stream_name = values.stream_name.trim();
    if stream_name.is_empty() {
        return Err("Zadaj názov VBAN streamu.");
    }
    Ok(MicDraft {
        name: name.to_string(),
        stream_name: stream_name.to_string(),
        sender_ip: values.sender_ip.trim().to_string(),
        ..values.clone()
    })
}

/// The microphone editor's fields. `Copy` (signals only).
#[derive(Clone, Copy)]
pub(super) struct MicForm {
    name: RwSignal<String>,
    stream_name: RwSignal<String>,
    sender_ip: RwSignal<String>,
    channel: RwSignal<MicChannel>,
    language: RwSignal<MicLanguage>,
    role: RwSignal<MicRole>,
}

impl EditorForm for MicForm {
    type Values = MicDraft;
    type Submission = MicDraft;
    type Config = ();

    fn new(_config: ()) -> Self {
        let blank = MicDraft::default();
        Self {
            name: RwSignal::new(blank.name),
            stream_name: RwSignal::new(blank.stream_name),
            sender_ip: RwSignal::new(blank.sender_ip),
            channel: RwSignal::new(blank.channel),
            language: RwSignal::new(blank.language),
            role: RwSignal::new(blank.role),
        }
    }

    fn new_values(self) -> MicDraft {
        MicDraft::default()
    }

    fn load(self, values: &MicDraft) {
        self.name.set(values.name.clone());
        self.stream_name.set(values.stream_name.clone());
        self.sender_ip.set(values.sender_ip.clone());
        self.channel.set(values.channel);
        self.language.set(values.language);
        self.role.set(values.role);
    }

    fn values(self) -> MicDraft {
        MicDraft {
            name: self.name.get(),
            stream_name: self.stream_name.get(),
            sender_ip: self.sender_ip.get(),
            channel: self.channel.get(),
            language: self.language.get(),
            role: self.role.get(),
        }
    }

    fn values_untracked(self) -> MicDraft {
        MicDraft {
            name: self.name.get_untracked(),
            stream_name: self.stream_name.get_untracked(),
            sender_ip: self.sender_ip.get_untracked(),
            channel: self.channel.get_untracked(),
            language: self.language.get_untracked(),
            role: self.role.get_untracked(),
        }
    }

    fn validate(self, values: &MicDraft) -> Result<MicDraft, &'static str> {
        validate_mic(values)
    }

    fn render_fields(
        self,
        spec: EditorSpec,
        first: NodeRef<leptos::html::Input>,
        invalid: Signal<bool>,
    ) -> AnyView {
        let role = move |field: &str| format!("{}-{field}", spec.role);
        let aria_invalid = move || invalid.get().to_string();
        view! {
            <>
                <div class="settings__form-row">
                    <label>
                        <span>"Názov"</span>
                        <input type="text" node_ref=first data-role=role("name") placeholder="Hosť"
                            required aria-required="true"
                            aria-describedby=spec.message_id aria-invalid=aria_invalid
                            prop:value=move || self.name.get()
                            on:input=move |ev| self.name.set(event_target_value(&ev)) />
                    </label>
                    <label>
                        <span>"VBAN stream"</span>
                        <input type="text" data-role=role("stream") placeholder="GUEST_EN"
                            required aria-required="true"
                            aria-describedby=spec.message_id aria-invalid=aria_invalid
                            prop:value=move || self.stream_name.get()
                            on:input=move |ev| self.stream_name.set(event_target_value(&ev)) />
                    </label>
                    <label>
                        <span>"IP odosielateľa (nepovinné)"</span>
                        <input type="text" data-role=role("sender-ip") placeholder="ktorýkoľvek"
                            aria-describedby=spec.message_id aria-invalid=aria_invalid
                            prop:value=move || self.sender_ip.get()
                            on:input=move |ev| self.sender_ip.set(event_target_value(&ev)) />
                    </label>
                </div>
                <div class="settings__form-row">
                    {render_select("Kanál", role("channel"), spec.message_id, invalid, self.channel, CHANNELS)}
                    {render_select("Jazyk", role("language"), spec.message_id, invalid, self.language, LANGUAGES)}
                    {render_select("Rola", role("role"), spec.message_id, invalid, self.role, ROLES)}
                </div>
            </>
        }
        .into_any()
    }

    fn render_action_lead(self, _spec: EditorSpec) -> AnyView {
        ().into_any()
    }
}

/// A `<select>` over `options`; the selection lives on the options
/// (`prop:selected`, the #827 rule).
fn render_select<T>(
    caption: &'static str,
    data_role: String,
    message_id: &'static str,
    invalid: Signal<bool>,
    value: RwSignal<T>,
    options: &'static [(T, &'static str, &'static str)],
) -> impl IntoView
where
    T: Copy + PartialEq + Send + Sync + 'static,
{
    let on_change = move |ev: leptos::ev::Event| {
        let picked = event_target_value(&ev);
        if let Some((option, _, _)) = options.iter().find(|(_, key, _)| *key == picked) {
            value.set(*option);
        }
    };
    view! {
        <label>
            <span>{caption}</span>
            <select data-role=data_role aria-describedby=message_id
                aria-invalid=move || invalid.get().to_string() on:change=on_change>
                {options
                    .iter()
                    .map(|(option, key, text)| {
                        let option = *option;
                        view! {
                            <option value=*key prop:selected=move || value.get() == option>{*text}</option>
                        }
                    })
                    .collect_view()}
            </select>
        </label>
    }
}

/// An option's caption.
fn caption<T: PartialEq>(options: &[(T, &'static str, &'static str)], value: T) -> &'static str {
    options
        .iter()
        .find(|(option, _, _)| *option == value)
        .map_or("?", |(_, _, text)| *text)
}

/// The row's aside: language · role · channel · the id (its recordings' name).
fn mic_aside(mic: &MicConfig) -> String {
    format!(
        "{} · {} · kanál {} · id {}",
        caption(LANGUAGES, mic.language),
        caption(ROLES, mic.role),
        caption(CHANNELS, mic.channel),
        mic.id
    )
}

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
    let card = ListCard::<MicConfig>::new(toast, &TEXT);
    let row = move |mic: &MicConfig, _: Memo<Option<()>>| mic_row(mic);
    let mics = card.items();

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
    fn the_editor_needs_a_name_and_a_stream_and_trims_them() {
        let blank_name = draft("   ", "", MicRole::Guest);
        assert_eq!(validate_mic(&blank_name), Err("Zadaj názov mikrofónu."));
        let mut blank_stream = draft("Hosť", "", MicRole::Guest);
        blank_stream.stream_name = "  ".into();
        assert_eq!(
            validate_mic(&blank_stream),
            Err("Zadaj názov VBAN streamu.")
        );
        let mut padded = draft("  Hosť ", " 192.0.2.10 ", MicRole::Guest);
        padded.stream_name = " GUEST_EN ".into();
        let valid = validate_mic(&padded).unwrap();
        assert_eq!(
            (
                valid.name.as_str(),
                valid.stream_name.as_str(),
                valid.sender_ip.as_str()
            ),
            ("Hosť", "GUEST_EN", "192.0.2.10")
        );
        let round_trip = MicDraft::of(&valid.clone().into_config("guest".into()));
        assert_eq!(round_trip, valid);
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
