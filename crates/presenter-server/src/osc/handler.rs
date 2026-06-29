use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use chrono::Utc;
use presenter_core::{OscSettings, VelocityMode};
use rosc::{decoder, OscPacket};
use tokio::{
    net::UdpSocket,
    sync::{oneshot, Mutex, RwLock},
};
use tracing::{debug, info, warn};

use crate::state::AppState;

use super::parser::{extract_note_velocity, Extracted};
use super::OscBridgeInner;

pub(super) const DEDUPE_WINDOW: Duration = Duration::from_millis(75);
const NOTE_OFF_WINDOW: Duration = Duration::from_millis(300);
const ACCUMULATOR_WINDOW: Duration = Duration::from_millis(500);

pub(super) struct ListenerConfig {
    pub(super) address_pattern: String,
    pub(super) velocity_mode: VelocityMode,
}

impl ListenerConfig {
    pub(super) fn from_settings(settings: &OscSettings) -> Self {
        Self {
            address_pattern: settings.address_pattern.clone(),
            velocity_mode: settings.velocity_mode,
        }
    }
}

#[derive(Default)]
struct ControlContext {
    presentation_id: Option<presenter_core::PresentationId>,
    last_prefix: Option<String>,
    last_slide_velocity: Option<u8>,
    last_slide_index: Option<usize>,
    last_slide_time: Option<Instant>,
}

#[derive(Debug, Default)]
pub(super) struct PendingPair {
    note: Option<(u8, Instant)>,
    velocity: Option<(u8, Instant)>,
}

pub(super) fn compute_index(velocity: u8, mode: VelocityMode) -> Option<usize> {
    match mode {
        VelocityMode::ZeroBased => Some(velocity as usize),
        VelocityMode::OneBased => {
            if velocity == 0 {
                None
            } else {
                Some((velocity.saturating_sub(1)) as usize)
            }
        }
    }
}

pub(super) async fn store_partial_event(
    accumulator: &Arc<Mutex<HashMap<String, PendingPair>>>,
    channel: String,
    note: Option<u8>,
    velocity: Option<u8>,
) -> Option<(u8, u8)> {
    let mut guard = accumulator.lock().await;
    let entry = guard.entry(channel.clone()).or_default();
    let now = Instant::now();

    if let Some(note) = note {
        entry.note = Some((note, now));
    }
    if let Some(velocity) = velocity {
        entry.velocity = Some((velocity, now));
    }

    if let Some((_, timestamp)) = entry.note {
        if now.duration_since(timestamp) > ACCUMULATOR_WINDOW {
            entry.note = None;
        }
    }
    if let Some((_, timestamp)) = entry.velocity {
        if now.duration_since(timestamp) > ACCUMULATOR_WINDOW {
            entry.velocity = None;
        }
    }

    if let (Some((note, _)), Some((velocity, _))) = (entry.note, entry.velocity) {
        guard.remove(&channel);
        Some((note, velocity))
    } else {
        None
    }
}

pub(super) async fn run_listener(
    socket: UdpSocket,
    mut shutdown: oneshot::Receiver<()>,
    app_state: AppState,
    inner: Arc<OscBridgeInner>,
    config: ListenerConfig,
) {
    let control = Arc::new(RwLock::new(ControlContext::default()));
    let accumulator = Arc::new(Mutex::new(HashMap::<String, PendingPair>::new()));
    let mut buf = vec![0u8; 4096];
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                debug!("osc listener received shutdown signal");
                break;
            }
            result = socket.recv_from(&mut buf) => {
                match result {
                    Ok((len, _remote)) => {
                        if let Err(err) = handle_packet(&buf[..len], &app_state, &inner, &config, &control, &accumulator).await {
                            warn!(?err, "failed to handle osc packet");
                            let mut status = inner.status.write().await;
                            status.last_error = Some(err.to_string());
                        }
                    }
                    Err(err) => {
                        let mut status = inner.status.write().await;
                        status.last_error = Some(err.to_string());
                        warn!(?err, "osc listener socket error");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        }
    }

    let mut status = inner.status.write().await;
    status.listening = false;
}

async fn handle_packet(
    data: &[u8],
    app_state: &AppState,
    inner: &Arc<OscBridgeInner>,
    config: &ListenerConfig,
    control: &Arc<RwLock<ControlContext>>,
    accumulator: &Arc<Mutex<HashMap<String, PendingPair>>>,
) -> anyhow::Result<()> {
    match decoder::decode_udp(data) {
        Ok((_, packet)) => {
            process_packet(packet, app_state, inner, config, control, accumulator).await
        }
        Err(err) => Err(anyhow!(err)),
    }
}

async fn process_packet(
    packet: OscPacket,
    app_state: &AppState,
    inner: &Arc<OscBridgeInner>,
    config: &ListenerConfig,
    control: &Arc<RwLock<ControlContext>>,
    accumulator: &Arc<Mutex<HashMap<String, PendingPair>>>,
) -> anyhow::Result<()> {
    let mut queue = vec![packet];
    while let Some(packet) = queue.pop() {
        match packet {
            OscPacket::Message(message) => match extract_note_velocity(&message, config) {
                Extracted::Complete(note, velocity) => {
                    handle_note(note, velocity, app_state, inner, config, control).await?;
                }
                Extracted::Pending {
                    channel,
                    note,
                    velocity,
                } => {
                    if let Some((note, velocity)) =
                        store_partial_event(accumulator, channel, note, velocity).await
                    {
                        handle_note(note, velocity, app_state, inner, config, control).await?;
                    }
                }
                Extracted::Ignore => {}
            },
            OscPacket::Bundle(bundle) => {
                for sub in bundle.content.into_iter().rev() {
                    queue.push(sub);
                }
            }
        }
    }
    Ok(())
}

async fn handle_note(
    note: u8,
    velocity: u8,
    app_state: &AppState,
    inner: &Arc<OscBridgeInner>,
    config: &ListenerConfig,
    control: &Arc<RwLock<ControlContext>>,
) -> anyhow::Result<()> {
    update_status_for_message(inner, note, velocity).await;

    match note {
        19 | 20 => trigger_slide(velocity, app_state, config, control).await,
        _ => Ok(()),
    }
}

async fn update_status_for_message(inner: &Arc<OscBridgeInner>, note: u8, velocity: u8) {
    let mut status = inner.status.write().await;
    // Always track when we last received any OSC message.
    status.last_message_at = Some(Utc::now());
    // Product decision: expose only the last note-on pair to the UI so
    // rapid note-off (velocity 0) events do not overwrite the slide index.
    if velocity > 0 {
        status.last_note = Some(note);
        status.last_velocity = Some(velocity);
    }
    status.last_error = None;
}

async fn trigger_slide(
    velocity: u8,
    app_state: &AppState,
    config: &ListenerConfig,
    control: &Arc<RwLock<ControlContext>>,
) -> anyhow::Result<()> {
    let now = Instant::now();
    let Some(song) = app_state.current_ableset_song().await else {
        debug!("AbleSet automation idle; skipping OSC trigger");
        return Ok(());
    };

    let Some(presentation_id) = app_state.resolve_ableset_presentation(&song.prefix).await? else {
        debug!(prefix = %song.prefix, "AbleSet prefix missing in library");
        return Ok(());
    };

    let detail = app_state
        .presentation_detail(presentation_id)
        .await?
        .ok_or_else(|| anyhow!("presentation not found for AbleSet trigger"))?;
    let slides = detail.2.slides;
    if slides.is_empty() {
        return Ok(());
    }

    let Some(index) = compute_index(velocity, config.velocity_mode) else {
        return Ok(());
    };
    let clamped_index = index.min(slides.len() - 1);

    let (prev_presentation, prev_velocity, prev_time, prev_index, prev_prefix) = {
        let ctx = control.read().await;
        (
            ctx.presentation_id,
            ctx.last_slide_velocity,
            ctx.last_slide_time,
            ctx.last_slide_index,
            ctx.last_prefix.clone(),
        )
    };

    let song_changed = prev_presentation != Some(presentation_id)
        || prev_prefix.as_deref() != Some(song.prefix.as_str());

    if !song_changed {
        if let Some(last) = prev_time {
            if prev_velocity == Some(velocity) {
                if velocity == 0 {
                    if last.elapsed() < NOTE_OFF_WINDOW {
                        return Ok(());
                    }
                } else if last.elapsed() < DEDUPE_WINDOW {
                    return Ok(());
                }
            }
            if velocity == 0 && last.elapsed() < NOTE_OFF_WINDOW {
                return Ok(());
            }
        }

        if let Some(prev_index) = prev_index {
            if prev_index == clamped_index {
                if let Some(last) = prev_time {
                    if last.elapsed() < DEDUPE_WINDOW {
                        return Ok(());
                    }
                }
            }
        }
    }

    info!(prefix = %song.prefix, presentation = ?presentation_id, slide_index = clamped_index, velocity = velocity, "AbleSet resolved presentation");
    let current = &slides[clamped_index];
    let next = slides.get(clamped_index + 1);
    let next_id = next.map(|slide| slide.id);

    app_state
        .update_stage_state(presentation_id, current.id, next_id, None, None)
        .await
        .map_err(|err| anyhow!(err))?;

    {
        let mut ctx = control.write().await;
        ctx.presentation_id = Some(presentation_id);
        ctx.last_prefix = Some(song.prefix.clone());
        ctx.last_slide_velocity = Some(velocity);
        ctx.last_slide_index = Some(clamped_index);
        ctx.last_slide_time = Some(now);
    }

    info!(
        slide_index = clamped_index,
        slide = %current.id,
        prefix = %song.prefix,
        "OSC triggered slide"
    );
    Ok(())
}
