//! Settings → "Počúvanie": the two fixed Dante mics (#933). Presenter's own
//! Dante device has two mono RX channels, `mic 1` and `mic 2`, on every box
//! that names a Dante interface; what feeds them is Dante patching, never a
//! setting, so this part of the card only SHOWS them: the device line, then per
//! mic its name, RX channel, state badge, health and recorder lines — all from
//! the card's one `GET /listen/status` poll (the status lists them after the
//! saved VBAN mics). Nothing here edits anything. All copy is Slovak.

use leptos::prelude::*;
use presenter_core::listen::status::{DanteState, DanteStatus, ListenStatus, MicStatus};

use super::listen::{health_line, mic_badge, recording_line};
use super::listen_form::{language_caption, role_caption};

/// The Dante device's line: what runs, or why nothing does.
pub(super) fn dante_line(dante: &DanteStatus) -> String {
    let device = dante.device.as_deref().unwrap_or("?");
    match dante.state {
        DanteState::NotStarted | DanteState::Idle => String::new(),
        DanteState::NoInterface => format!("Dante: {}", dante.error.as_deref().unwrap_or_default()),
        DanteState::Running => format!(
            "Dante: zariadenie {device} beží na {} — kanály „mic 1“ a „mic 2“, zdroj sa zapája v Dante.",
            dante.interface.as_deref().unwrap_or("?")
        ),
        DanteState::Retrying => format!(
            "Dante: zariadenie {device} nebeží — skúšam znova. {}",
            dante.error.as_deref().unwrap_or_default()
        ),
    }
}

/// The ids of the status's Dante mics, in order (the row keys).
pub(super) fn dante_mic_ids(status: &ListenStatus) -> Vec<String> {
    status
        .mics
        .iter()
        .filter(|mic| mic.config.is_dante())
        .map(|mic| mic.config.id.clone())
        .collect()
}

/// A Dante mic's address line: its RX channel.
fn channel_text(mic: &MicStatus) -> String {
    mic.config
        .dante
        .as_ref()
        .map(|dante| format!("kanál Dante „{}“", dante.channel))
        .unwrap_or_default()
}

/// The row's aside: language · role · the id (its recordings' name).
fn dante_aside(mic: &MicStatus) -> String {
    format!(
        "{} · {} · id {}",
        language_caption(mic.config.language),
        role_caption(mic.config.role),
        mic.config.id
    )
}

/// One fixed Dante mic, read-only: everything through `Memo`s over the one
/// status signal, so a poll never rebuilds the row.
fn dante_row(id: String, status: RwSignal<Option<ListenStatus>>) -> impl IntoView {
    let health = {
        let id = id.clone();
        Memo::new(move |_| {
            status.with(|s| {
                s.as_ref()
                    .and_then(|s| s.mics.iter().find(|m| m.config.id == id).cloned())
            })
        })
    };
    let recording = {
        let id = id.clone();
        Memo::new(move |_| {
            status.with(|s| {
                s.as_ref()
                    .and_then(|s| s.recorder.mics.iter().find(|r| r.id == id).cloned())
            })
        })
    };
    let badge = move || mic_badge(health.with(|h| h.as_ref().map(|h| h.state)));
    let text =
        move |f: fn(&MicStatus) -> String| health.with(|h| h.as_ref().map(f).unwrap_or_default());
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
    view! {
        <li class="settings__list-item" data-role="listen-dante-mic" data-id=id>
            <div class="settings__list-summary">
                <div class="settings__list-primary">
                    <div class="settings__list-title">
                        <span class="settings__host-label">{move || text(|m| m.config.name.clone())}</span>
                    </div>
                    <p class="settings__list-line">
                        <span class="settings__host-addr" data-role="listen-dante-channel">
                            {move || text(channel_text)}
                        </span>
                        <span class=move || format!("settings__status settings__status--{}", badge().0)
                            data-role="listen-state" data-state=move || badge().2>{move || badge().1}</span>
                        <span class="settings__list-aside" data-role="listen-config">{move || text(dante_aside)}</span>
                    </p>
                    <p class="settings__list-meta settings__list-meta--muted" data-role="listen-health">
                        {move || text(health_line)}
                    </p>
                    <p class="settings__list-meta settings__list-meta--muted" data-role="listen-recording">
                        {move || recording.with(|r| r.as_ref().map(recording_line).unwrap_or_default())}
                    </p>
                    {warning}
                </div>
            </div>
        </li>
    }
}

/// The Dante part of the card: the device line and the two fixed mics.
#[component]
pub(super) fn DanteMics(status: RwSignal<Option<ListenStatus>>) -> impl IntoView {
    let ids =
        Memo::new(move |_| status.with(|s| s.as_ref().map(dante_mic_ids).unwrap_or_default()));
    view! {
        <p class="settings__list-meta" data-role="listen-dante">
            {move || status.with(|s| s.as_ref().map(|s| dante_line(&s.dante)).unwrap_or_default())}
        </p>
        <Show when=move || ids.with(|ids| !ids.is_empty())>
            <ul class="settings__list" data-role="listen-dante-mics">
                <For each=move || ids.get() key=|id| id.clone() let:id>
                    {dante_row(id, status)}
                </For>
            </ul>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::listen::mics::fixed_dante_mics;
    use presenter_core::listen::recorder::RecorderStatus;
    use presenter_core::listen::status::{MicState, SocketState, SocketStatus};

    fn waiting(config: presenter_core::listen::mics::MicConfig) -> MicStatus {
        MicStatus {
            config,
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

    #[test]
    fn the_dante_rows_are_the_status_dante_mics_and_name_their_channel() {
        let mut vban = fixed_dante_mics()[0].clone();
        vban.id = "guest".into();
        vban.transport = presenter_core::listen::mics::MicTransport::Vban;
        vban.dante = None;
        let mut mics = vec![waiting(vban)];
        mics.extend(fixed_dante_mics().into_iter().map(waiting));
        let status = ListenStatus {
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
        };
        assert_eq!(dante_mic_ids(&status), ["dante-mic-1", "dante-mic-2"]);
        assert_eq!(channel_text(&status.mics[2]), "kanál Dante „mic 2“");
        assert_eq!(
            dante_aside(&status.mics[1]),
            "neprepisovať · moderátor · id dante-mic-1"
        );
        assert_eq!(health_line(&status.mics[1]), "Zatiaľ žiadny zvuk z Dante.");
    }

    #[test]
    fn the_dante_line_names_the_device_and_why_it_does_not_run() {
        let mut dante = DanteStatus::default();
        assert_eq!(dante_line(&dante), "", "not started: nothing to say");
        dante = DanteStatus {
            state: DanteState::Running,
            device: Some("presenter-listen".into()),
            interface: Some("enp2s0.70".into()),
            error: None,
        };
        assert_eq!(
            dante_line(&dante),
            "Dante: zariadenie presenter-listen beží na enp2s0.70 — kanály „mic 1“ a „mic 2“, zdroj sa zapája v Dante."
        );
        dante.state = DanteState::Retrying;
        dante.error = Some("no such interface".into());
        assert!(dante_line(&dante).ends_with("skúšam znova. no such interface"));
        dante.state = DanteState::NoInterface;
        dante.error = Some("Na tomto počítači nie je nastavené rozhranie Dante.".into());
        assert_eq!(
            dante_line(&dante),
            "Dante: Na tomto počítači nie je nastavené rozhranie Dante."
        );
    }
}
