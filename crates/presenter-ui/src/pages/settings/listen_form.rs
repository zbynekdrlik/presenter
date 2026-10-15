//! The microphone editor of the Počúvanie card (#852 C1b): [`MicDraft`] (what the
//! editor holds) and [`MicForm`], the card's `host_editor::EditorForm` — name,
//! VBAN stream, optional sender IP, channel / language / role selects — and the
//! option captions the row's aside shows. Split out of `listen.rs`, which keeps
//! the card, its API calls and its rows.
//!
//! A new mic's id comes from its role (`mic_id_for_role` over the listed ids) and
//! never changes: it names the mic's recordings. The editor shows it ([`id_line`]),
//! so the server's refusal ("Mikrofón „guest-2“: …") names an id the operator has
//! seen. All copy is Slovak.

use leptos::prelude::*;
use presenter_core::listen::mics::{mic_id_for_role, MicChannel, MicConfig, MicLanguage, MicRole};

use super::host_editor::{EditorForm, EditorSpec};

/// The ids of the listed mics: the card provides it as a context (`ListenCard`)
/// and keeps it current, [`MicForm`] reads it for a new mic's id line.
#[derive(Clone, Copy)]
pub(super) struct TakenIds(pub(super) RwSignal<Vec<String>>);

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
    /// The edited mic's id; `None` for a new mic (its id comes from its role).
    pub(super) id: Option<String>,
    pub(super) name: String,
    pub(super) stream_name: String,
    /// As typed; blank = any sender.
    pub(super) sender_ip: String,
    pub(super) channel: MicChannel,
    pub(super) language: MicLanguage,
    pub(super) role: MicRole,
}

impl Default for MicDraft {
    fn default() -> Self {
        Self {
            id: None,
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
    pub(super) fn of(mic: &MicConfig) -> Self {
        Self {
            id: Some(mic.id.clone()),
            name: mic.name.clone(),
            stream_name: mic.stream_name.clone(),
            sender_ip: mic.sender_ip.clone().unwrap_or_default(),
            channel: mic.channel,
            language: mic.language,
            role: mic.role,
        }
    }

    /// The mic with this draft's values; `id` names its recordings and stays.
    pub(super) fn into_config(self, id: String) -> MicConfig {
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

/// The editor's id line: an existing mic's own id, a new mic's from its role over
/// the `taken` ids (the id the save will give it, unless the list changes first).
fn id_line(id: Option<&str>, role: MicRole, taken: &[String]) -> String {
    match id {
        Some(id) => format!("Identifikátor: {id} (nemení sa)"),
        None => {
            let taken: Vec<&str> = taken.iter().map(String::as_str).collect();
            let new_id = mic_id_for_role(role, &taken);
            format!("Identifikátor: {new_id} (podľa roly, nemení sa)")
        }
    }
}

/// The microphone editor's fields. `Copy` (signals only).
#[derive(Clone, Copy)]
pub(super) struct MicForm {
    id: RwSignal<Option<String>>,
    name: RwSignal<String>,
    stream_name: RwSignal<String>,
    sender_ip: RwSignal<String>,
    channel: RwSignal<MicChannel>,
    language: RwSignal<MicLanguage>,
    role: RwSignal<MicRole>,
    /// The listed ids ([`TakenIds`]; empty when no card provided them).
    taken: RwSignal<Vec<String>>,
}

impl EditorForm for MicForm {
    type Values = MicDraft;
    type Submission = MicDraft;
    type Config = ();

    fn new(_config: ()) -> Self {
        let blank = MicDraft::default();
        Self {
            id: RwSignal::new(blank.id),
            name: RwSignal::new(blank.name),
            stream_name: RwSignal::new(blank.stream_name),
            sender_ip: RwSignal::new(blank.sender_ip),
            channel: RwSignal::new(blank.channel),
            language: RwSignal::new(blank.language),
            role: RwSignal::new(blank.role),
            taken: use_context::<TakenIds>().map_or_else(|| RwSignal::new(Vec::new()), |ids| ids.0),
        }
    }

    fn new_values(self) -> MicDraft {
        MicDraft::default()
    }

    fn load(self, values: &MicDraft) {
        self.id.set(values.id.clone());
        self.name.set(values.name.clone());
        self.stream_name.set(values.stream_name.clone());
        self.sender_ip.set(values.sender_ip.clone());
        self.channel.set(values.channel);
        self.language.set(values.language);
        self.role.set(values.role);
    }

    fn values(self) -> MicDraft {
        MicDraft {
            id: self.id.get(),
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
            id: self.id.get_untracked(),
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
        let id_text = move || {
            let (id, mic_role) = (self.id.get(), self.role.get());
            self.taken
                .with(|taken| id_line(id.as_deref(), mic_role, taken))
        };
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
                <p class="settings__list-meta settings__list-meta--muted" data-role=role("id")>
                    {id_text}
                </p>
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
pub(super) fn mic_aside(mic: &MicConfig) -> String {
    format!(
        "{} · {} · kanál {} · id {}",
        caption(LANGUAGES, mic.language),
        caption(ROLES, mic.role),
        caption(CHANNELS, mic.channel),
        mic.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str, stream: &str, ip: &str) -> MicDraft {
        MicDraft {
            name: name.into(),
            stream_name: stream.into(),
            sender_ip: ip.into(),
            ..MicDraft::default()
        }
    }

    #[test]
    fn the_editor_needs_a_name_and_a_stream_and_trims_them() {
        assert_eq!(
            validate_mic(&draft("   ", "GUEST_EN", "")),
            Err("Zadaj názov mikrofónu.")
        );
        assert_eq!(
            validate_mic(&draft("Hosť", "  ", "")),
            Err("Zadaj názov VBAN streamu.")
        );
        let valid = validate_mic(&draft("  Hosť ", " GUEST_EN ", " 192.0.2.10 ")).unwrap();
        assert_eq!(
            (
                valid.name.as_str(),
                valid.stream_name.as_str(),
                valid.sender_ip.as_str()
            ),
            ("Hosť", "GUEST_EN", "192.0.2.10")
        );
        let saved = valid.clone().into_config("guest".into());
        assert_eq!(
            MicDraft::of(&saved),
            MicDraft {
                id: Some("guest".into()),
                ..valid
            },
            "the draft of a saved mic is the validated one, with its id"
        );
    }

    #[test]
    fn a_new_mics_id_line_follows_its_role_and_the_listed_ids() {
        let none: Vec<String> = Vec::new();
        assert_eq!(
            id_line(None, MicRole::Guest, &none),
            "Identifikátor: guest (podľa roly, nemení sa)"
        );
        assert_eq!(
            id_line(None, MicRole::Interpreter, &none),
            "Identifikátor: interp (podľa roly, nemení sa)"
        );
        let listed = vec!["guest".to_string(), "interp".to_string()];
        assert_eq!(
            id_line(None, MicRole::Guest, &listed),
            "Identifikátor: guest-2 (podľa roly, nemení sa)",
            "the role's id is taken: the one the save will give"
        );
        assert_eq!(
            id_line(None, MicRole::Preacher, &listed),
            "Identifikátor: preacher (podľa roly, nemení sa)"
        );
    }

    #[test]
    fn an_edited_mics_id_line_is_its_own_id_whatever_its_role() {
        let listed = vec!["interp".to_string()];
        assert_eq!(
            id_line(Some("interp"), MicRole::Preacher, &listed),
            "Identifikátor: interp (nemení sa)"
        );
    }
}
