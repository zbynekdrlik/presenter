//! Settings → "Počúvanie" (#852 C1b, spec §5.6): the listening microphones on
//! the shared `list_card::ListCard`, edited in place by `listen_form::MicForm`
//! (the editor, its option captions and the new mic's id line). The API is
//! the whole list (`GET|PUT /listen/mics`): a save or a delete reads the list,
//! changes one mic and writes the list back ([`apply_draft`]) — one round trip
//! apart, in one Settings card, for at most 4 mics; the server validates the
//! whole list and its Slovak 422 rule shows in the editor. A new mic's id comes
//! from its role (`mic_id_for_role`: `guest`, `interp`, `preacher`, the spec
//! §5.6 ids) and never changes: it names the mic's recordings. Each row's
//! health and recorder line, the socket line and the recorder's per-day switch
//! come from `GET /listen/status`, polled every 2 s while the card is on screen
//! (C1b-7). All copy is Slovak.

use leptos::prelude::*;
use presenter_core::listen::mics::{mic_id_for_role, MicChannel, MicConfig, MicLanguage, MicRole};
use presenter_core::listen::recorder::{
    MicRecording, RecorderStatus, RecordingState, RECORDING_RETENTION_DAYS,
};
use presenter_core::listen::status::{
    ListenStatus, MicState, MicStatus, SocketState, SocketStatus,
};

use super::host_editor::EditorSpec;
use super::list_card::{failure_text, CardItem, CardText, ListCard, RowParts};
use super::list_sync::ResponseOrder;
use super::listen_form::{mic_aside, MicDraft, MicForm, TakenIds};
use super::video_sources::is_stale;
use super::ToastHandle;
use crate::api::{listen, ApiError};

/// The health poll: fast enough to watch a level move; a quiet GET (#850 O1).
const LISTEN_STATUS_REFRESH_MS: u32 = 2_000;
/// Audio bytes per second of a recording (16 kHz mono s16).
const RECORDING_BYTES_PER_SEC: u64 = 32_000;

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
    /// Health comes from the card's own `/listen/status` poll, not the list.
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

/// A mic's badge: (CSS state modifier, Slovak text, `data-state`).
fn mic_badge(state: Option<MicState>) -> (&'static str, &'static str, &'static str) {
    match state {
        Some(MicState::Receiving) => ("live", "prijíma", "receiving"),
        Some(MicState::NoAudio) => ("not-broadcasting", "žiadny zvuk", "no_audio"),
        Some(MicState::Error) => ("error", "chyba", "error"),
        Some(MicState::Waiting) => ("checking", "čaká na zvuk", "waiting"),
        None => ("unknown", "—", "unknown"),
    }
}

/// One decimal, Slovak comma.
fn decimal(value: f32) -> String {
    format!("{value:.1}").replace('.', ",")
}

/// A mic's health line (spec §5.4).
fn health_line(mic: &MicStatus) -> String {
    if mic.packets == 0 {
        return "Zatiaľ žiadny paket VBAN.".to_string();
    }
    let mut parts = Vec::new();
    if mic.packets_per_sec > 0.0 {
        parts.push(format!("{} paketov/s", decimal(mic.packets_per_sec)));
    }
    if let Some(level) = mic.level_dbfs {
        let voice = if mic.voiced { "● hlas" } else { "○ ticho" };
        parts.push(format!("{} dBFS {voice}", decimal(level)));
    }
    parts.push(format!(
        "stratené {} · neskoré {} · resync {}",
        mic.lost_packets, mic.late_packets, mic.resyncs
    ));
    if mic.dropped_blocks > 0 {
        parts.push(format!("zahodené bloky {}", mic.dropped_blocks));
    }
    if mic.other_sender_packets > 0 {
        parts.push(format!("iný odosielateľ {}", mic.other_sender_packets));
    }
    parts.push(format!("rámce {}", mic.frames_out));
    if let Some(age) = mic.last_packet_age_ms {
        parts.push(format!(
            "posledný paket pred {} s",
            decimal(age as f32 / 1000.0)
        ));
    }
    if let Some(format) = mic.format {
        parts.push(format!(
            "{} Hz · {} kan. · {}",
            format.rate_hz,
            format.channels,
            format.sample_type.as_str()
        ));
    }
    if let Some(sender) = &mic.sender {
        parts.push(format!("od {sender}"));
    }
    parts.join(" · ")
}

/// Minutes:seconds and megabytes of audio in `bytes`.
fn audio_size(bytes: u64) -> String {
    let seconds = bytes / RECORDING_BYTES_PER_SEC;
    let megabytes = decimal(bytes as f32 / 1_000_000.0);
    format!("{}:{:02} · {megabytes} MB", seconds / 60, seconds % 60)
}

/// A mic's recorder line; empty when there is nothing to say.
fn recording_line(rec: &MicRecording) -> String {
    let file = rec
        .file
        .as_deref()
        .and_then(|path| path.rsplit('/').next())
        .unwrap_or_default();
    match rec.state {
        RecordingState::Recording => format!(
            "Nahráva: {file} · {} · zahodené rámce {}",
            audio_size(rec.bytes),
            rec.dropped_frames
        ),
        RecordingState::Waiting => "Nahrávanie: čaká na zvuk.".to_string(),
        RecordingState::Error => {
            "Nahrávanie zlyhalo — ďalší pokus o pár minút (najneskôr o hodinu).".to_string()
        }
        RecordingState::Off if rec.file.is_some() => {
            format!("Posledná nahrávka: {file} · {}", audio_size(rec.bytes))
        }
        RecordingState::Off => String::new(),
    }
}

/// The socket line (spec §5.1, C-S8).
fn socket_line(socket: &SocketStatus) -> String {
    match socket.state {
        SocketState::NotStarted => "Príjem VBAN nebeží.".to_string(),
        SocketState::Idle => {
            "Príjem VBAN je nečinný — bez mikrofónu nie je otvorený žiadny port.".to_string()
        }
        SocketState::Bound => format!(
            "Počúva na {} (UDP).",
            socket
                .local_addr
                .as_deref()
                .or(socket.bind_addr.as_deref())
                .unwrap_or("?")
        ),
        SocketState::Retrying => format!(
            "Port sa nedá otvoriť — skúšam znova. {}",
            socket.error.as_deref().unwrap_or_default()
        ),
    }
}

/// The line under the recorder switch.
fn recorder_line(recorder: &RecorderStatus) -> String {
    match (recorder.enabled, recorder.enabled_for) {
        (true, Some(day)) => format!(
            "Nahráva sa do {}/{} — vypne sa samo o polnoci. \
             Nahrávky sa mažú po {RECORDING_RETENTION_DAYS} dňoch.",
            recorder.root,
            day.format("%Y-%m-%d")
        ),
        _ if recorder.mics.is_empty() => {
            "Nahrávanie je vypnuté. Najprv pridaj mikrofón — bez neho niet čo nahrávať.".to_string()
        }
        _ => format!(
            "Nahrávanie je vypnuté. Zapína sa vždy len na dnešný deň; \
             nahrávky sa mažú po {RECORDING_RETENTION_DAYS} dňoch."
        ),
    }
}

/// The line under the switch: the recorder's own, or that its state is unknown
/// (no status yet, or the polls failed).
fn recorder_state_line(recorder: Option<&RecorderStatus>) -> String {
    recorder.map_or_else(
        || "Stav nahrávania nie je známy.".to_string(),
        recorder_line,
    )
}

/// What the card shows after a status poll whose answer is `fresh`: the fresh
/// status; the last one after a single failure (a blip — a page torn down
/// mid-fetch); nothing after two in a row, so the badges read "—" rather than
/// stale numbers (`video_sources::is_stale`). Also the failures in a row.
fn after_poll(
    shown: Option<ListenStatus>,
    failures: u32,
    fresh: Option<ListenStatus>,
) -> (Option<ListenStatus>, u32) {
    match fresh {
        Some(fresh) => (Some(fresh), 0),
        None => {
            let failures = failures.saturating_add(1);
            (if is_stale(failures) { None } else { shown }, failures)
        }
    }
}

/// What one tick of the poll does.
#[derive(Debug, PartialEq, Eq)]
enum Tick {
    /// The card is on screen: fetch the status.
    Fetch,
    /// The card is hidden and still holds a status: drop it.
    Forget,
    /// The card is hidden and holds nothing.
    Idle,
}

/// The operator keeps the Settings panel mounted under `display: none`: a hidden
/// card fetches nothing and forgets its last status, so an hours-old "prijíma" or
/// a checked switch never greets whoever opens Settings (the badges read "—"
/// until the first visible tick).
fn tick(visible: bool, holds_status: bool) -> Tick {
    match (visible, holds_status) {
        (true, _) => Tick::Fetch,
        (false, true) => Tick::Forget,
        (false, false) => Tick::Idle,
    }
}

/// Fetch `/listen/status` now and every 2 s while `section` is on screen (see
/// [`tick`]); returns the fetch for an immediate refresh. Numbered: an older
/// answer never replaces a newer one, nor, once the card was hidden, one that
/// was still in flight.
fn start_status_poll(
    status: RwSignal<Option<ListenStatus>>,
    section: NodeRef<leptos::html::Section>,
) -> impl Fn() + Copy + 'static {
    let order = ResponseOrder::default();
    let failures = StoredValue::new(0u32);
    let fetch = move || {
        leptos::task::spawn_local(async move {
            let seq = order.begin();
            let answer = listen::status().await;
            if !order.accept(seq) {
                return;
            }
            let fresh = answer.as_ref().ok().cloned();
            let (shown, count) = after_poll(status.get_untracked(), failures.get_value(), fresh);
            if let Err(err) = &answer {
                if is_stale(count) && !is_stale(count - 1) {
                    leptos::logging::warn!(
                        "listen status poll failed {count}x in a row — showing it as unknown: {err}"
                    );
                }
            }
            failures.set_value(count);
            status.set(shown);
        });
    };
    fetch();
    gloo_timers::callback::Interval::new(LISTEN_STATUS_REFRESH_MS, move || {
        let visible = section
            .get_untracked()
            .is_some_and(|el| el.offset_parent().is_some());
        match tick(visible, status.with_untracked(Option::is_some)) {
            Tick::Fetch => fetch(),
            Tick::Forget => {
                // numbering a request nobody sends drops any answer still in flight
                let seq = order.begin();
                order.accept(seq);
                failures.set_value(0);
                status.set(None);
            }
            Tick::Idle => {}
        }
    })
    .forget();
    fetch
}

/// The switch's box shows `checked`.
fn set_switch(switch_ref: NodeRef<leptos::html::Input>, checked: bool) {
    if let Some(el) = switch_ref.get_untracked() {
        el.set_checked(checked);
    }
}

/// The recorder switch: `PUT /listen/recorder`, a toast, a fresh status. Once the
/// PUT is answered the box shows what the server stored; when it fails, the state
/// from before the click.
fn recorder_switch<R>(
    toast: ToastHandle,
    switching: RwSignal<bool>,
    switch_ref: NodeRef<leptos::html::Input>,
    refresh: R,
) -> impl Fn(leptos::ev::Event) + 'static
where
    R: Fn() + Copy + 'static,
{
    move |ev| {
        let enabled = event_target_checked(&ev);
        switching.set(true);
        leptos::task::spawn_local(async move {
            match listen::set_recorder(enabled).await {
                Ok(setting) => {
                    set_switch(switch_ref, setting.enabled_for.is_some());
                    toast.show(
                        if enabled {
                            "Nahrávanie je zapnuté na dnešný deň."
                        } else {
                            "Nahrávanie je vypnuté."
                        },
                        "success",
                    );
                }
                Err(err) => {
                    set_switch(switch_ref, !enabled);
                    let message = failure_text("Nahrávanie sa nepodarilo prepnúť.", &err);
                    toast.show(&message, "error");
                }
            }
            switching.set(false);
            refresh();
        });
    }
}

/// A mic row's own parts: the state badge and the mic's language / role /
/// channel / id, the health and recorder lines, the mic's or the recorder's error.
fn mic_row(mic: &MicConfig, status: RwSignal<Option<ListenStatus>>) -> RowParts {
    let health = {
        let id = mic.id.clone();
        Memo::new(move |_| {
            status.with(|s| {
                s.as_ref()
                    .and_then(|s| s.mics.iter().find(|m| m.config.id == id).cloned())
            })
        })
    };
    let recording = {
        let id = mic.id.clone();
        Memo::new(move |_| {
            status.with(|s| {
                s.as_ref()
                    .and_then(|s| s.recorder.mics.iter().find(|r| r.id == id).cloned())
            })
        })
    };
    let badge = move || mic_badge(health.with(|h| h.as_ref().map(|h| h.state)));
    let warning = move || {
        let mic_error = health.with(|h| h.as_ref().and_then(|h| h.error.clone()));
        let recorder_error = recording.with(|r| r.as_ref().and_then(|r| r.error.clone()));
        mic_error.or(recorder_error).map(|error| {
            view! {
                <p class="settings__list-meta settings__list-meta--warning"
                    data-role="listen-error">{format!("⚠ {error}")}</p>
            }
        })
    };
    let aside = mic_aside(mic);
    RowParts {
        line: view! {
            <>
                <span class=move || format!("settings__status settings__status--{}", badge().0)
                    data-role="listen-state" data-state=move || badge().2>{move || badge().1}</span>
                <span class="settings__list-aside" data-role="listen-config">{aside}</span>
            </>
        }
        .into_any(),
        meta: Some(
            view! {
                <>
                    <p class="settings__list-meta settings__list-meta--muted" data-role="listen-health">
                        {move || health.with(|h| h.as_ref().map(health_line).unwrap_or_default())}
                    </p>
                    <p class="settings__list-meta settings__list-meta--muted" data-role="listen-recording">
                        {move || recording.with(|r| r.as_ref().map(recording_line).unwrap_or_default())}
                    </p>
                </>
            }
            .into_any(),
        ),
        warning: warning.into_any(),
        actions: ().into_any(),
    }
}

/// The ids of the listed mics, for the editor's id line.
fn listed_ids(mics: &[MicConfig]) -> Vec<String> {
    mics.iter().map(|mic| mic.id.clone()).collect()
}

/// The Počúvanie card.
#[component]
pub fn ListenCard(toast: ToastHandle) -> impl IntoView {
    // The editor's form is built INSIDE `ListCard::new` and reads this context
    // there, before the card's list exists: the id line's signal derives from a
    // cell filled right after, before anything reads it — no Effect, no tick of lag.
    let listed = StoredValue::new(None::<ReadSignal<Vec<MicConfig>>>);
    provide_context(TakenIds(Signal::derive(move || {
        listed
            .get_value()
            .map(|mics| mics.with(|list| listed_ids(list)))
            .unwrap_or_default()
    })));
    let card = ListCard::<MicConfig>::new(toast, &TEXT);
    let mics = card.items();
    listed.set_value(Some(mics));
    let status = RwSignal::new(None::<ListenStatus>);
    let section = NodeRef::<leptos::html::Section>::new();
    let refresh = start_status_poll(status, section);
    let recorder = Memo::new(move |_| status.with(|s| s.as_ref().map(|s| s.recorder.clone())));
    let recording_on = Memo::new(move |_| recorder.with(|r| r.as_ref().is_some_and(|r| r.enabled)));
    // no status: the box is neither on nor off (indeterminate, never "not recording")
    let unknown = Memo::new(move |_| recorder.with(Option::is_none));
    // no switch without a status, nor ON without a mic (it stays switchable OFF)
    let locked = Memo::new(move |_| {
        recorder.with(|r| r.as_ref().is_none_or(|r| !r.enabled && r.mics.is_empty()))
    });
    let switching = RwSignal::new(false);
    let switch_ref = NodeRef::<leptos::html::Input>::new();
    let on_switch = recorder_switch(toast, switching, switch_ref, refresh);
    let row = move |mic: &MicConfig, _: Memo<Option<()>>| mic_row(mic, status);

    view! {
        <section class="settings__card" data-role="listen-card" node_ref=section>
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
            <p class="settings__list-meta" data-role="listen-socket">
                {move || status.with(|s| s.as_ref().map(|s| socket_line(&s.socket)).unwrap_or_default())}
            </p>
            <div class="settings__form-row settings__form-row--inline">
                <label class="settings__form-checkbox">
                    <input type="checkbox" node_ref=switch_ref data-role="listen-recorder-toggle"
                        prop:checked=move || recording_on.get()
                        prop:indeterminate=move || unknown.get()
                        prop:disabled=move || switching.get() || locked.get()
                        on:change=on_switch />
                    <span>"Nahrávať dnes"</span>
                </label>
            </div>
            <p class="settings__list-meta settings__list-meta--muted" data-role="listen-recorder-state">
                {move || recorder.with(|r| recorder_state_line(r.as_ref()))}
            </p>
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

    fn receiving() -> MicStatus {
        MicStatus {
            config: mic("guest", "Hosť", MicRole::Guest),
            state: MicState::Receiving,
            sender: Some("192.0.2.10:6980".into()),
            format: Some(presenter_core::audio::vban::VbanAudioFormat {
                rate_hz: 48_000,
                channels: 1,
                sample_type: presenter_core::audio::vban::VbanSampleType::Int16,
            }),
            frames_per_packet: Some(256),
            packets: 936,
            packets_per_sec: 187.5,
            last_packet_age_ms: Some(120),
            lost_packets: 1,
            late_packets: 0,
            resyncs: 0,
            dropped_blocks: 0,
            other_sender_packets: 0,
            frames_out: 49,
            level_dbfs: Some(-9.03),
            voiced: true,
            error: None,
        }
    }

    fn status_with(mics: Vec<MicStatus>, recorder: RecorderStatus) -> ListenStatus {
        ListenStatus {
            socket: SocketStatus {
                state: SocketState::Bound,
                bind_addr: Some("0.0.0.0:6980".into()),
                local_addr: Some("0.0.0.0:6980".into()),
                error: None,
            },
            mics,
            unmatched: Vec::new(),
            rejected: Vec::new(),
            recorder,
        }
    }

    #[test]
    fn the_listed_ids_follow_the_list_in_order() {
        let mics = vec![
            mic("guest", "Hosť", MicRole::Guest),
            mic("interp", "Tlmočník", MicRole::Interpreter),
        ];
        assert_eq!(listed_ids(&mics), ["guest", "interp"]);
        assert!(listed_ids(&[]).is_empty());
    }

    #[test]
    fn the_health_line_reads_the_status_in_slovak() {
        assert_eq!(
            health_line(&receiving()),
            "187,5 paketov/s · -9,0 dBFS ● hlas · stratené 1 · neskoré 0 · resync 0 · rámce 49 · \
             posledný paket pred 0,1 s · 48000 Hz · 1 kan. · int16 · od 192.0.2.10:6980"
        );
        let waiting = MicStatus {
            packets: 0,
            ..receiving()
        };
        assert_eq!(health_line(&waiting), "Zatiaľ žiadny paket VBAN.");
        assert_eq!(
            mic_badge(Some(MicState::NoAudio)),
            ("not-broadcasting", "žiadny zvuk", "no_audio")
        );
        assert_eq!(mic_badge(None).1, "—", "no status: no claim");
    }

    #[test]
    fn the_recorder_lines_follow_the_state() {
        let mut rec = MicRecording {
            id: "guest".into(),
            state: RecordingState::Recording,
            file: Some("/opt/presenter/recordings/2026-10-18/guest_194205.wav".into()),
            bytes: 2_400_000,
            dropped_frames: 0,
            error: None,
        };
        assert_eq!(
            recording_line(&rec),
            "Nahráva: guest_194205.wav · 1:15 · 2,4 MB · zahodené rámce 0"
        );
        rec.state = RecordingState::Off;
        assert_eq!(
            recording_line(&rec),
            "Posledná nahrávka: guest_194205.wav · 1:15 · 2,4 MB"
        );
        rec.state = RecordingState::Waiting;
        assert_eq!(recording_line(&rec), "Nahrávanie: čaká na zvuk.");
        rec.state = RecordingState::Error;
        assert!(recording_line(&rec).starts_with("Nahrávanie zlyhalo"));
        rec.state = RecordingState::Off;
        rec.file = None;
        assert_eq!(recording_line(&rec), "");
        let on = RecorderStatus {
            enabled: true,
            enabled_for: Some("2026-10-18".parse().unwrap()),
            root: "/opt/presenter/recordings".into(),
            mics: vec![MicRecording::off("guest")],
        };
        assert!(
            recorder_line(&on).starts_with("Nahráva sa do /opt/presenter/recordings/2026-10-18 —")
        );
        let off = RecorderStatus {
            enabled: false,
            ..on.clone()
        };
        assert!(recorder_line(&off).starts_with("Nahrávanie je vypnuté. Zapína sa"));
        assert!(
            recorder_line(&RecorderStatus::default()).contains("Najprv pridaj mikrofón"),
            "no mic: the hint the disabled switch needs"
        );
        // the retention is the core's number, not a copy of it
        let days = format!("po {RECORDING_RETENTION_DAYS} dňoch.");
        assert!(recorder_line(&on).ends_with(&days));
        assert!(recorder_line(&off).ends_with(&days));
    }

    #[test]
    fn an_unknown_status_says_so_instead_of_reading_as_not_recording() {
        assert_eq!(
            recorder_state_line(None),
            "Stav nahrávania nie je známy.",
            "no status: not a claim about the recorder"
        );
        let known = RecorderStatus::default();
        assert_eq!(recorder_state_line(Some(&known)), recorder_line(&known));
    }

    #[test]
    fn a_hidden_card_fetches_nothing_and_forgets_what_it_held() {
        assert_eq!(tick(true, false), Tick::Fetch, "on screen: the first fetch");
        assert_eq!(tick(true, true), Tick::Fetch);
        assert_eq!(
            tick(false, true),
            Tick::Forget,
            "hidden: an old status must not greet whoever opens Settings"
        );
        assert_eq!(
            tick(false, false),
            Tick::Idle,
            "hidden, nothing held: nothing to do"
        );
    }

    #[test]
    fn the_socket_line_names_the_state() {
        let status = status_with(Vec::new(), RecorderStatus::default());
        assert_eq!(socket_line(&status.socket), "Počúva na 0.0.0.0:6980 (UDP).");
        let idle = SocketStatus {
            state: SocketState::Idle,
            local_addr: None,
            ..status.socket.clone()
        };
        assert!(socket_line(&idle).contains("nečinný"));
    }

    #[test]
    fn one_failed_poll_keeps_the_last_status_and_two_show_nothing() {
        let first = status_with(vec![receiving()], RecorderStatus::default());
        assert_eq!(
            after_poll(None, 3, Some(first.clone())),
            (Some(first.clone()), 0)
        );
        let (kept, failures) = after_poll(Some(first.clone()), 0, None);
        assert_eq!((kept, failures), (Some(first.clone()), 1), "a blip");
        let (shown, failures) = after_poll(Some(first.clone()), 1, None);
        assert_eq!(
            (shown, failures),
            (None, 2),
            "two in a row: the badges read —"
        );
        assert_eq!(
            after_poll(None, 2, Some(first.clone())),
            (Some(first), 0),
            "back"
        );
    }
}
