use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use presenter_core::{OscSettings, VelocityMode};
use rosc::{decoder, OscMessage, OscPacket, OscType};
use tokio::sync::oneshot;
use tokio::{
    net::UdpSocket,
    sync::{Mutex, RwLock},
    task::JoinHandle,
};
use tracing::{debug, info, warn};

use crate::state::AppState;

#[derive(Clone)]
pub struct OscBridge {
    inner: Arc<OscBridgeInner>,
}

struct OscBridgeInner {
    status: RwLock<OscStatusInner>,
    listener: Mutex<Option<ListenerGuard>>,
}

struct ListenerGuard {
    shutdown: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

#[derive(Debug, Clone)]
struct OscStatusInner {
    enabled: bool,
    listening: bool,
    listen_port: u16,
    host_port: Option<u16>,
    address_pattern: String,
    velocity_mode: VelocityMode,
    last_message_at: Option<DateTime<Utc>>,
    last_note: Option<u8>,
    last_velocity: Option<u8>,
    last_error: Option<String>,
}

impl Default for OscStatusInner {
    fn default() -> Self {
        Self {
            enabled: false,
            listening: false,
            listen_port: 39051,
            host_port: std::env::var("PRESENTER_OSC_HOST_PORT")
                .ok()
                .and_then(|raw| raw.parse().ok()),
            address_pattern: "/note".to_string(),
            velocity_mode: VelocityMode::OneBased,
            last_message_at: None,
            last_note: None,
            last_velocity: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OscStatusSnapshot {
    pub enabled: bool,
    pub listening: bool,
    pub listen_port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_port: Option<u16>,
    pub address_pattern: String,
    pub velocity_mode: VelocityMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_note: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_velocity: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl From<&OscStatusInner> for OscStatusSnapshot {
    fn from(inner: &OscStatusInner) -> Self {
        Self {
            enabled: inner.enabled,
            listening: inner.listening,
            listen_port: inner.listen_port,
            host_port: inner.host_port,
            address_pattern: inner.address_pattern.clone(),
            velocity_mode: inner.velocity_mode,
            last_message_at: inner.last_message_at,
            last_note: inner.last_note,
            last_velocity: inner.last_velocity,
            last_error: inner.last_error.clone(),
        }
    }
}

const DEDUPE_WINDOW: Duration = Duration::from_millis(75);
const NOTE_OFF_WINDOW: Duration = Duration::from_millis(300);

impl OscBridge {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(OscBridgeInner {
                status: RwLock::new(OscStatusInner::default()),
                listener: Mutex::new(None),
            }),
        }
    }

    pub async fn apply_settings(
        &self,
        settings: OscSettings,
        app_state: AppState,
    ) -> anyhow::Result<()> {
        {
            let mut status = self.inner.status.write().await;
            status.enabled = settings.enabled;
            status.listen_port = settings.listen_port;
            status.host_port = std::env::var("PRESENTER_OSC_HOST_PORT")
                .ok()
                .and_then(|raw| raw.parse().ok());
            status.address_pattern = settings.address_pattern.clone();
            status.velocity_mode = settings.velocity_mode;
            status.last_error = None;
        }

        self.stop_listener().await;

        if !settings.enabled {
            return Ok(());
        }

        match self.start_listener(settings, app_state).await {
            Ok(()) => Ok(()),
            Err(err) => {
                let mut status = self.inner.status.write().await;
                status.listening = false;
                status.last_error = Some(err.to_string());
                Err(err)
            }
        }
    }

    pub async fn status(&self) -> OscStatusSnapshot {
        let status = self.inner.status.read().await;
        OscStatusSnapshot::from(&*status)
    }

    async fn start_listener(
        &self,
        settings: OscSettings,
        app_state: AppState,
    ) -> anyhow::Result<()> {
        let socket = UdpSocket::bind(("0.0.0.0", settings.listen_port))
            .await
            .with_context(|| {
                format!("failed to bind OSC listener port {}", settings.listen_port)
            })?;
        socket
            .set_broadcast(true)
            .with_context(|| "failed to enable broadcast for OSC listener")?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let inner = self.inner.clone();
        let config = ListenerConfig::from_settings(&settings);
        let handle = tokio::spawn(async move {
            run_listener(socket, shutdown_rx, app_state, inner.clone(), config).await;
        });

        {
            let mut guard = self.inner.listener.lock().await;
            *guard = Some(ListenerGuard {
                shutdown: shutdown_tx,
                handle,
            });
        }

        let mut status = self.inner.status.write().await;
        status.listening = true;
        status.last_error = None;
        Ok(())
    }

    async fn stop_listener(&self) {
        let mut guard = self.inner.listener.lock().await;
        if let Some(listener) = guard.take() {
            if listener.shutdown.send(()).is_err() {
                debug!("osc listener already stopped");
            }
            if let Err(err) = listener.handle.await {
                debug!(?err, "osc listener join failed");
            }
        }
        let mut status = self.inner.status.write().await;
        status.listening = false;
    }
}

struct ListenerConfig {
    address_pattern: String,
    velocity_mode: VelocityMode,
}

impl ListenerConfig {
    fn from_settings(settings: &OscSettings) -> Self {
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

const ACCUMULATOR_WINDOW: Duration = Duration::from_millis(500);

#[derive(Debug, Default)]
struct PendingPair {
    note: Option<(u8, Instant)>,
    velocity: Option<(u8, Instant)>,
}

#[derive(Debug)]
enum Extracted {
    Complete(u8, u8),
    Pending {
        channel: String,
        note: Option<u8>,
        velocity: Option<u8>,
    },
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SplitKind {
    Note,
    Velocity,
}

async fn run_listener(
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

fn parse_compound_message(message: &OscMessage, config: &ListenerConfig) -> Option<(u8, u8)> {
    let addr = message.addr.to_ascii_lowercase();
    let pattern = config.address_pattern.to_ascii_lowercase();
    if addr != pattern {
        return None;
    }
    let mut numeric = Vec::new();
    for arg in &message.args {
        match arg {
            OscType::Int(value) => numeric.push(*value as f64),
            OscType::Float(value) => numeric.push(*value as f64),
            _ => {}
        }
    }
    if numeric.len() >= 3 {
        let note = numeric[numeric.len() - 2];
        let velocity = numeric[numeric.len() - 1];
        return normalise_note_velocity(note, velocity);
    }
    if numeric.len() >= 2 {
        return normalise_note_velocity(numeric[0], numeric[1]);
    }
    None
}

fn parse_split_message(message: &OscMessage) -> Option<(String, SplitKind, u8)> {
    if message.args.is_empty() {
        return None;
    }
    let value = match &message.args[0] {
        OscType::Int(value) => *value as f64,
        OscType::Float(value) => *value as f64,
        _ => return None,
    };
    let value = normalise_single(value)?;

    let addr = message.addr.to_ascii_lowercase();
    if !addr.starts_with('/') {
        return None;
    }
    let mut prefix = String::new();
    let mut digits = String::new();
    for ch in addr.chars().skip(1) {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            prefix.push(ch);
        }
    }
    let kind = match prefix.as_str() {
        "note" => SplitKind::Note,
        "velocity" => SplitKind::Velocity,
        _ => return None,
    };
    let channel = if digits.is_empty() {
        "0".to_string()
    } else {
        digits
    };
    Some((channel, kind, value))
}

fn extract_note_velocity(message: &OscMessage, config: &ListenerConfig) -> Extracted {
    if let Some((note, velocity)) = parse_compound_message(message, config) {
        return Extracted::Complete(note, velocity);
    }
    if let Some((channel, kind, value)) = parse_split_message(message) {
        return match kind {
            SplitKind::Note => Extracted::Pending {
                channel,
                note: Some(value),
                velocity: None,
            },
            SplitKind::Velocity => Extracted::Pending {
                channel,
                note: None,
                velocity: Some(value),
            },
        };
    }
    Extracted::Ignore
}

async fn store_partial_event(
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

fn normalise_note_velocity(note: f64, velocity: f64) -> Option<(u8, u8)> {
    let note = normalise_single(note)?;
    let velocity = normalise_single(velocity)?;
    Some((note, velocity))
}

fn normalise_single(value: f64) -> Option<u8> {
    let rounded = value.round();
    if !(0.0..=127.0).contains(&rounded) {
        return None;
    }
    Some(rounded as u8)
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

fn compute_index(velocity: u8, mode: VelocityMode) -> Option<usize> {
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
        .update_stage_state(presentation_id, current.id, next_id)
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

#[cfg(test)]
mod tests {
    use super::*;
    use rosc::{OscMessage, OscType};
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::Mutex;

    #[test]
    fn extract_two_arguments_returns_note_and_velocity() {
        let config = ListenerConfig {
            address_pattern: "/note".to_string(),
            velocity_mode: VelocityMode::OneBased,
        };
        let message = OscMessage {
            addr: "/note".to_string(),
            args: vec![OscType::Int(60), OscType::Int(90)],
        };
        match extract_note_velocity(&message, &config) {
            Extracted::Complete(60, 90) => {}
            other => panic!("unexpected extraction result: {:?}", other),
        }
    }

    #[test]
    fn extract_three_arguments_uses_last_two_values() {
        let config = ListenerConfig {
            address_pattern: "/note".to_string(),
            velocity_mode: VelocityMode::OneBased,
        };
        let message = OscMessage {
            addr: "/note".to_string(),
            args: vec![OscType::Int(1), OscType::Int(62), OscType::Int(45)],
        };
        match extract_note_velocity(&message, &config) {
            Extracted::Complete(62, 45) => {}
            other => panic!("unexpected extraction result: {:?}", other),
        }
    }

    #[test]
    fn compute_index_zero_based_mapping() {
        assert_eq!(compute_index(0, VelocityMode::ZeroBased), Some(0));
        assert_eq!(compute_index(5, VelocityMode::ZeroBased), Some(5));
    }

    #[test]
    fn compute_index_one_based_mapping() {
        assert_eq!(compute_index(0, VelocityMode::OneBased), None);
        assert_eq!(compute_index(1, VelocityMode::OneBased), Some(0));
        assert_eq!(compute_index(7, VelocityMode::OneBased), Some(6));
    }

    #[tokio::test]
    async fn split_messages_merge_into_single_event() {
        let config = ListenerConfig {
            address_pattern: "/note".to_string(),
            velocity_mode: VelocityMode::OneBased,
        };
        let accumulator = Arc::new(Mutex::new(HashMap::new()));

        let note_message = OscMessage {
            addr: "/Note1".to_string(),
            args: vec![OscType::Int(19)],
        };
        if let Extracted::Pending {
            channel,
            note,
            velocity,
        } = extract_note_velocity(&note_message, &config)
        {
            assert_eq!(note, Some(19));
            assert!(velocity.is_none());
            assert!(store_partial_event(&accumulator, channel, note, velocity)
                .await
                .is_none());
        } else {
            panic!("expected pending note extraction");
        }

        let velocity_message = OscMessage {
            addr: "/Velocity1".to_string(),
            args: vec![OscType::Int(7)],
        };
        if let Extracted::Pending {
            channel,
            note,
            velocity,
        } = extract_note_velocity(&velocity_message, &config)
        {
            assert!(note.is_none());
            assert_eq!(velocity, Some(7));
            let combined = store_partial_event(&accumulator, channel, note, velocity).await;
            assert_eq!(combined, Some((19, 7)));
        } else {
            panic!("expected pending velocity extraction");
        }
    }
}
