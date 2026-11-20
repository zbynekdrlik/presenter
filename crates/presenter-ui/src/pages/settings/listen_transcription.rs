//! Settings → "Počúvanie" → "Prepis hosťa" (#852 C3-9, spec §22.5 / §6.4): the
//! owner's controls of the paid live transcript of the guest's microphone —
//! the switch, which mic is the guest's (every published mic, VBAN and Dante,
//! by name), the daily cap in minutes, today's choice "Spustiť teraz" /
//! "Zastaviť" / "Automaticky (Kázeň)" (`PUT /listen/session`), a warning when
//! the box has no Soniox key, and ONE status line (state, reason, minutes
//! against the cap, today's cost, latency). Everything it shows comes from the
//! card's one `GET /listen/status` poll (no second poll); a change is a
//! `PUT /listen/transcription` of the whole setting, its answer is shown at
//! once and the poll confirms. A refused PUT (the server's Slovak 422 rule)
//! toasts and puts the controls back to what is stored. All copy is Slovak.

use leptos::prelude::*;
use presenter_core::listen::status::ListenStatus;
use presenter_core::listen::transcription::{
    MicTranscription, OffReason, SessionMode, TranscriptionSetting, TranscriptionState,
    TranscriptionStatus, DAILY_CAP_MINUTES_RANGE,
};

use super::list_card::failure_text;
use super::row_status::state_label;
use super::ToastHandle;
use crate::api::{listen, ApiError};

const MS_PER_MINUTE: u64 = 60_000;

/// The server's token of a state (core's camelCase serde name).
fn state_token(state: TranscriptionState) -> &'static str {
    match state {
        TranscriptionState::Off => "off",
        TranscriptionState::Armed => "armed",
        TranscriptionState::Connecting => "connecting",
        TranscriptionState::Streaming => "streaming",
        TranscriptionState::Gated => "gated",
        TranscriptionState::Parked => "parked",
        TranscriptionState::Reconnecting => "reconnecting",
        TranscriptionState::Stopped => "stopped",
    }
}

/// The badge's existing `settings__status--<modifier>` (no new colour).
fn badge_modifier(state: TranscriptionState) -> &'static str {
    match state {
        TranscriptionState::Streaming => "live",
        TranscriptionState::Connecting | TranscriptionState::Reconnecting => "connecting",
        TranscriptionState::Armed | TranscriptionState::Gated | TranscriptionState::Parked => {
            "parked"
        }
        TranscriptionState::Stopped => "error",
        TranscriptionState::Off => "unknown",
    }
}

/// The guest mic's session, or — with no guest session in the status (no mic
/// chosen) — an `off` one whose reason the setting gives.
fn guest_session(t: &TranscriptionStatus) -> MicTranscription {
    if let Some(mic) = t.mics.first() {
        return mic.clone();
    }
    let reason = if t.setting.enabled {
        OffReason::NoGuestMic
    } else {
        OffReason::Disabled
    };
    MicTranscription {
        mic_id: t.setting.guest_mic.clone().unwrap_or_default(),
        reason: Some(reason),
        cap_ms: u64::from(t.setting.daily_cap_minutes) * MS_PER_MINUTE,
        ..MicTranscription::default()
    }
}

/// Seconds with one decimal, Slovak comma.
fn seconds(ms: u32) -> String {
    format!("{:.1}", f64::from(ms) / 1000.0).replace('.', ",")
}

/// The status line after the badge: reason, minutes, cost, latency.
fn status_line(mic: &MicTranscription) -> String {
    let mut parts = Vec::new();
    if let Some(reason) = mic.reason {
        parts.push(reason.slovak().to_string());
    }
    parts.push(format!(
        "pripojené {} / {} min dnes",
        mic.connected_ms_today / MS_PER_MINUTE,
        mic.cap_ms / MS_PER_MINUTE
    ));
    parts.push(format!(
        "≈ ${} dnes",
        format!("{:.2}", mic.cost_usd_today).replace('.', ",")
    ));
    if let (Some(p50), Some(p90)) = (mic.latency_p50_ms, mic.latency_p90_ms) {
        parts.push(format!(
            "oneskorenie {} / {} s (p50 / p90)",
            seconds(p50),
            seconds(p90)
        ));
    }
    parts.join(" · ")
}

/// The guest-mic choices: every published mic by name; a stored guest mic that
/// is not published (any more) stays listed so the select can show it (spec
/// gap 7: the setting is kept, the mic comes back).
fn guest_options(status: &ListenStatus) -> Vec<(String, String)> {
    let mut options: Vec<(String, String)> = status
        .mics
        .iter()
        .map(|mic| (mic.config.id.clone(), mic.config.name.clone()))
        .collect();
    if let Some(id) = status.transcription.setting.guest_mic.as_deref() {
        if !options.iter().any(|(listed, _)| listed == id) {
            options.push((id.to_string(), format!("{id} — nie je v zozname")));
        }
    }
    options
}

/// The cap field's text as minutes, `None` when it is not a whole number in
/// range (the server would refuse it).
fn parse_cap(text: &str) -> Option<u32> {
    text.trim()
        .parse::<u32>()
        .ok()
        .filter(|minutes| DAILY_CAP_MINUTES_RANGE.contains(minutes))
}

/// The section's state: the card's status signal plus the write in flight.
#[derive(Clone, Copy)]
struct Section {
    status: RwSignal<Option<ListenStatus>>,
    busy: RwSignal<bool>,
    toast: ToastHandle,
    switch_ref: NodeRef<leptos::html::Input>,
    mic_ref: NodeRef<leptos::html::Select>,
    cap_ref: NodeRef<leptos::html::Input>,
}

impl Section {
    fn setting(self) -> Option<TranscriptionSetting> {
        self.status
            .with_untracked(|s| s.as_ref().map(|s| s.transcription.setting.clone()))
    }

    /// Put every control back to the stored setting (after a refused write; a
    /// `Memo` that did not change would leave the operator's choice showing).
    fn restore(self) {
        let Some(setting) = self.setting() else {
            return;
        };
        if let Some(el) = self.switch_ref.get_untracked() {
            el.set_checked(setting.enabled);
        }
        if let Some(el) = self.mic_ref.get_untracked() {
            el.set_value(setting.guest_mic.as_deref().unwrap_or_default());
        }
        if let Some(el) = self.cap_ref.get_untracked() {
            el.set_value(&setting.daily_cap_minutes.to_string());
        }
    }

    /// Show a write's answer at once (the next poll confirms it).
    fn apply(self, answer: TranscriptionStatus) {
        self.status.update(|s| {
            if let Some(s) = s {
                s.transcription = answer;
            }
        });
    }

    fn finish(self, answer: Result<TranscriptionStatus, ApiError>, failed: &str) {
        match answer {
            Ok(answer) => self.apply(answer),
            Err(err) => {
                self.restore();
                self.toast.show(&failure_text(failed, &err), "error");
            }
        }
        self.busy.set(false);
    }

    /// `PUT /listen/transcription` of the stored setting with `change` applied.
    fn save(self, change: impl FnOnce(&mut TranscriptionSetting) + 'static) {
        let Some(mut setting) = self.setting() else {
            return;
        };
        change(&mut setting);
        self.busy.set(true);
        leptos::task::spawn_local(async move {
            let answer = listen::put_transcription(&setting).await;
            if answer.is_ok() {
                self.toast.show("Prepis hosťa je uložený.", "success");
            }
            self.finish(answer, "Prepis hosťa sa nepodarilo uložiť.");
        });
    }

    /// `PUT /listen/session`: today's choice.
    fn choose(self, mode: SessionMode) {
        self.busy.set(true);
        leptos::task::spawn_local(async move {
            let answer = listen::put_session(mode).await;
            if answer.is_ok() {
                let text = match mode {
                    SessionMode::On => "Prepis hosťa sa spúšťa.",
                    SessionMode::Off => "Prepis hosťa je dnes zastavený.",
                    SessionMode::Auto => "Prepis hosťa sa spustí s rozložením Kázeň.",
                };
                self.toast.show(text, "success");
            }
            self.finish(answer, "Voľbu prepisu sa nepodarilo uložiť.");
        });
    }

    fn on_cap(self, text: String) {
        match parse_cap(&text) {
            Some(minutes) => self.save(move |s| s.daily_cap_minutes = minutes),
            None => {
                self.restore();
                let text = format!(
                    "Denný limit prepisu môže byť {} až {} minút.",
                    DAILY_CAP_MINUTES_RANGE.start(),
                    DAILY_CAP_MINUTES_RANGE.end()
                );
                self.toast.show(&text, "error");
            }
        }
    }
}

/// One of today's three choices: a `ui-toggle`, ON for the current mode.
fn mode_button(
    section: Section,
    mode: SessionMode,
    current: Memo<Option<SessionMode>>,
    locked: Memo<bool>,
) -> impl IntoView {
    let (role, label) = match mode {
        SessionMode::On => ("listen-session-on", "Spustiť teraz"),
        SessionMode::Off => ("listen-session-off", "Zastaviť"),
        SessionMode::Auto => ("listen-session-auto", "Automaticky (Kázeň)"),
    };
    let on = move || current.get() == Some(mode);
    view! {
        <button type="button" class="ui-toggle" data-role=role
            data-state=move || if on() { "on" } else { "off" }
            aria-pressed=move || on().to_string()
            prop:disabled=move || locked.get()
            on:click=move |_| section.choose(mode)>{label}</button>
    }
}

/// The "Prepis hosťa" part of the Počúvanie card.
#[component]
pub(super) fn GuestTranscription(
    status: RwSignal<Option<ListenStatus>>,
    toast: ToastHandle,
) -> impl IntoView {
    let section = Section {
        status,
        busy: RwSignal::new(false),
        toast,
        switch_ref: NodeRef::new(),
        mic_ref: NodeRef::new(),
        cap_ref: NodeRef::new(),
    };
    let transcription =
        Memo::new(move |_| status.with(|s| s.as_ref().map(|s| s.transcription.clone())));
    let setting = Memo::new(move |_| transcription.with(|t| t.as_ref().map(|t| t.setting.clone())));
    let enabled = Memo::new(move |_| setting.with(|s| s.as_ref().is_some_and(|s| s.enabled)));
    let guest = Memo::new(move |_| {
        setting.with(|s| {
            s.as_ref()
                .and_then(|s| s.guest_mic.clone())
                .unwrap_or_default()
        })
    });
    let cap = Memo::new(move |_| {
        setting.with(|s| {
            s.as_ref()
                .map(|s| s.daily_cap_minutes.to_string())
                .unwrap_or_default()
        })
    });
    let mode = Memo::new(move |_| transcription.with(|t| t.as_ref().map(|t| t.mode)));
    let options =
        Memo::new(move |_| status.with(|s| s.as_ref().map(guest_options).unwrap_or_default()));
    let locked = Memo::new(move |_| section.busy.get() || transcription.with(Option::is_none));
    let no_key =
        Memo::new(move |_| transcription.with(|t| t.as_ref().is_some_and(|t| !t.key_configured)));
    let session = Memo::new(move |_| transcription.with(|t| t.as_ref().map(guest_session)));
    let badge = move || {
        session.with(|m| match m {
            Some(m) => (badge_modifier(m.state), state_token(m.state)),
            None => ("unknown", "unknown"),
        })
    };
    let error = move || {
        session
            .with(|m| m.as_ref().and_then(|m| m.error.clone()))
            .map(|error| {
                view! {
                    <p class="settings__list-meta settings__list-meta--warning"
                        data-role="listen-transcription-error">{format!("⚠ {error}")}</p>
                }
            })
    };

    view! {
        <div class="settings__form" data-role="listen-transcription">
            <div class="settings__form-header"><h3>"Prepis hosťa"</h3></div>
            <p class="settings__list-meta settings__list-meta--muted">
                "Anglický prepis reči hosťa pre tlmočníka (služba Soniox, platená podľa minút pripojenia). Beží len pre mikrofón hosťa."
            </p>
            <Show when=move || no_key.get()>
                <p class="settings__list-meta settings__list-meta--warning" data-role="listen-transcription-key">
                    "⚠ Chýba kľúč služby prepisu (Soniox) — prepis sa nespustí. Kľúč sa nastavuje pri nasadení (tajomstvo LISTEN_SONIOX_API_KEY)."
                </p>
            </Show>
            <div class="settings__form-row settings__form-row--inline">
                <label class="settings__form-checkbox">
                    <input type="checkbox" node_ref=section.switch_ref data-role="listen-transcription-toggle"
                        prop:checked=move || enabled.get()
                        prop:disabled=move || locked.get()
                        on:change=move |ev| {
                            let on = event_target_checked(&ev);
                            section.save(move |s| s.enabled = on);
                        } />
                    <span>"Prepis hosťa (Soniox)"</span>
                </label>
                <label>
                    "Mikrofón hosťa"
                    <select node_ref=section.mic_ref data-role="listen-guest-mic"
                        prop:disabled=move || locked.get()
                        on:change=move |ev| {
                            let id = event_target_value(&ev);
                            section.save(move |s| s.guest_mic = (!id.is_empty()).then_some(id));
                        }>
                        <option value="" prop:selected=move || guest.with(String::is_empty)>"— nevybraný —"</option>
                        <For each=move || options.get() key=|option| option.clone() let:option>
                            {
                                let (id, name) = option;
                                let value = id.clone();
                                view! {
                                    <option value=value prop:selected=move || guest.with(|g| *g == id)>{name}</option>
                                }
                            }
                        </For>
                    </select>
                </label>
                <label class="settings__form-control--tiny">
                    "Denný limit (min)"
                    <input type="number" node_ref=section.cap_ref data-role="listen-transcription-cap"
                        min=*DAILY_CAP_MINUTES_RANGE.start() max=*DAILY_CAP_MINUTES_RANGE.end()
                        prop:value=move || cap.get()
                        prop:disabled=move || locked.get()
                        on:change=move |ev| section.on_cap(event_target_value(&ev)) />
                </label>
            </div>
            <div class="settings__form-actions">
                {mode_button(section, SessionMode::On, mode, locked)}
                {mode_button(section, SessionMode::Off, mode, locked)}
                {mode_button(section, SessionMode::Auto, mode, locked)}
            </div>
            <p class="settings__list-meta" data-role="listen-transcription-status">
                <span class=move || format!("settings__status settings__status--{}", badge().0)
                    data-role="listen-transcription-state" data-state=move || badge().1>
                    {move || session.with(|m| m.as_ref().map(|m| state_label(state_token(m.state))).unwrap_or_else(|| "—".to_string()))}
                </span>
                " "
                <span data-role="listen-transcription-line">
                    {move || session.with(|m| m.as_ref().map(status_line).unwrap_or_default())}
                </span>
            </p>
            {error}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::listen::mics::{MicChannel, MicConfig, MicLanguage, MicRole, MicTransport};
    use presenter_core::listen::recorder::RecorderStatus;
    use presenter_core::listen::status::{
        DanteStatus, MicState, MicStatus, SocketState, SocketStatus,
    };

    fn mic(id: &str, name: &str) -> MicStatus {
        MicStatus {
            config: MicConfig {
                id: id.into(),
                name: name.into(),
                stream_name: "E2E_GUEST".into(),
                sender_ip: None,
                channel: MicChannel::Left,
                language: MicLanguage::En,
                role: MicRole::Guest,
                transport: MicTransport::Vban,
                dante: None,
            },
            state: MicState::Waiting,
            sender: None,
            format: None,
            frames_per_packet: None,
            packets: 0,
            packets_per_sec: 0.0,
            last_packet_age_ms: None,
            lost_packets: 0,
            late_packets: 0,
            resyncs: 0,
            dropped_blocks: 0,
            other_sender_packets: 0,
            frames_out: 0,
            level_dbfs: None,
            voiced: false,
            error: None,
        }
    }

    fn status(mics: Vec<MicStatus>, guest: Option<&str>) -> ListenStatus {
        let mut status = ListenStatus {
            socket: SocketStatus {
                state: SocketState::Idle,
                bind_addr: None,
                local_addr: None,
                error: None,
            },
            mics,
            unmatched: Vec::new(),
            rejected: Vec::new(),
            recorder: RecorderStatus::default(),
            dante: DanteStatus::default(),
            transcription: TranscriptionStatus::default(),
        };
        status.transcription.setting.guest_mic = guest.map(str::to_string);
        status
    }

    #[test]
    fn the_guest_choices_are_the_published_mics_plus_a_stored_missing_one() {
        let listed = status(
            vec![mic("guest", "Hosť"), mic("dante-mic-1", "mic 1")],
            None,
        );
        assert_eq!(
            guest_options(&listed),
            [
                ("guest".to_string(), "Hosť".to_string()),
                ("dante-mic-1".to_string(), "mic 1".to_string())
            ]
        );
        let missing = status(vec![mic("guest", "Hosť")], Some("old"));
        assert_eq!(
            guest_options(&missing)[1],
            ("old".to_string(), "old — nie je v zozname".to_string()),
            "the stored mic stays visible in the select"
        );
        let chosen = status(vec![mic("guest", "Hosť")], Some("guest"));
        assert_eq!(
            guest_options(&chosen).len(),
            1,
            "a listed guest mic is not doubled"
        );
    }

    #[test]
    fn the_status_line_reads_reason_minutes_cost_and_latency_in_slovak() {
        let mic = MicTranscription {
            mic_id: "guest".into(),
            state: TranscriptionState::Streaming,
            connected_ms_today: 12 * 60_000 + 59_000,
            cap_ms: 240 * 60_000,
            cost_usd_today: 0.026,
            latency_p50_ms: Some(700),
            latency_p90_ms: Some(1_150),
            ..MicTranscription::default()
        };
        assert_eq!(
            status_line(&mic),
            "pripojené 12 / 240 min dnes · ≈ $0,03 dnes · oneskorenie 0,7 / 1,1 s (p50 / p90)"
        );
        let stopped = MicTranscription {
            state: TranscriptionState::Stopped,
            reason: Some(OffReason::Payment),
            ..MicTranscription::default()
        };
        assert!(status_line(&stopped).starts_with("Prepis vypnutý — problém s účtom služby · "));
        assert_eq!(state_label(state_token(stopped.state)), "Zastavené");
        assert_eq!(badge_modifier(stopped.state), "error");
    }

    #[test]
    fn without_a_guest_session_the_setting_says_why_nothing_runs() {
        let mut t = TranscriptionStatus::default();
        assert_eq!(guest_session(&t).reason, Some(OffReason::Disabled));
        assert_eq!(
            status_line(&guest_session(&t)),
            "Prepis je vypnutý · pripojené 0 / 240 min dnes · ≈ $0,00 dnes",
            "the cap is the setting's before any session"
        );
        t.setting.enabled = true;
        assert_eq!(guest_session(&t).reason, Some(OffReason::NoGuestMic));
        assert_eq!(guest_session(&t).state, TranscriptionState::Off);
        t.mics.push(MicTranscription {
            mic_id: "guest".into(),
            state: TranscriptionState::Gated,
            ..MicTranscription::default()
        });
        assert_eq!(
            guest_session(&t).state,
            TranscriptionState::Gated,
            "the server's own session wins"
        );
    }

    #[test]
    fn every_state_has_a_slovak_label_and_an_existing_badge_colour() {
        for state in [
            TranscriptionState::Off,
            TranscriptionState::Armed,
            TranscriptionState::Connecting,
            TranscriptionState::Streaming,
            TranscriptionState::Gated,
            TranscriptionState::Parked,
            TranscriptionState::Reconnecting,
            TranscriptionState::Stopped,
        ] {
            let token = state_token(state);
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::json!(token),
                "the token is the server's"
            );
            assert!(
                !state_label(token).eq_ignore_ascii_case(token),
                "{token}: translated, never the English token"
            );
            assert!(["live", "connecting", "parked", "error", "unknown"]
                .contains(&badge_modifier(state)));
        }
    }

    #[test]
    fn the_cap_field_takes_whole_minutes_in_range_only() {
        assert_eq!(parse_cap(" 30 "), Some(30));
        assert_eq!(parse_cap("600"), Some(600));
        assert_eq!(parse_cap("9"), None);
        assert_eq!(parse_cap("601"), None);
        assert_eq!(parse_cap(""), None);
        assert_eq!(parse_cap("1,5"), None);
    }
}
