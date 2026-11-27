//! One language's translation session (#999 L1, DESIGN D1): ONE task per
//! ACTIVE language, shared by every guest listening to it. It reads the source
//! mic's 16 kHz frames, sends them to Gemini Live only while the preacher
//! speaks (a frame above −50 dBFS opens the gate, [`HANGOVER`] of quiet after
//! the last one closes it; the last [`PRE_ROLL_FRAMES`] are sent first on
//! every onset), and hands the translated audio and text to the listeners.
//!
//! Connections: the first one opens with the session; Gemini cuts an idle one
//! after 120 s ("client inactivity") and it is reopened at once with the
//! resumption handle, so the next phrase never waits for a connect. A `goAway`
//! opens the next connection with the newest handle; when it is ready the
//! audio moves there and the old one drains its last output for [`DRAIN`].
//! Any other end backs off 1, 2, 4 … 30 s. A refused key / model, or the
//! day's minute cap, stops the session for good (the listeners are told).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use presenter_core::audio::level::{is_voiced, rms_dbfs};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::AbortHandle;
use tokio::time::{Instant, MissedTickBehavior};

use super::config::GeminiEnv;
use super::conn::{self, ConnEvent};
use super::stats::{DayMeter, MinuteStats, STALL_AFTER};
use super::wire::{audio_message, classify_close, setup_message, CloseClass, ServerMessage};
use super::{GuestPacket, Language, SessionState, StopReason, TARGET};
use crate::listen::soniox::conn::Connector;
use crate::listen::{should_log_event, MicFrame};

/// Quiet after the last voiced frame before the send pauses: the model hears
/// the end of the phrase (and its pause) before nothing more arrives.
pub const HANGOVER: Duration = Duration::from_secs(2);
/// A voiced frame after this much quiet is a new onset (latency is measured
/// from it).
pub const ONSET_GAP: Duration = Duration::from_millis(1_500);
/// The frames before an onset sent with it (1 s).
pub const PRE_ROLL_FRAMES: usize = 10;
/// Frames held while a connection is not ready yet (5 s), oldest dropped.
pub const PENDING_FRAMES: usize = 50;
/// How long a replaced connection may still deliver its last output.
pub const DRAIN: Duration = Duration::from_secs(4);
const RETRY_CAP: Duration = Duration::from_secs(30);
/// A connection that answers nothing this long while audio goes out is dead
/// (Gemini sends a resumption update about every second while it hears
/// audio): dropped and reopened after the back-off. This also bounds what its
/// unbounded message queue can hold.
pub const NO_ANSWER: Duration = Duration::from_secs(10);
/// An idle cut of a connection younger than this (or never ready) is not
/// trusted as idle: it backs off like any other end, so a server that cuts at
/// once can never make the session loop on connects.
pub const IDLE_CUT_MIN_AGE: Duration = Duration::from_secs(60);
const FRAME_MS: u64 = 100;

/// Everything a session needs besides its language.
#[derive(Clone)]
pub struct SessionDeps {
    pub env: GeminiEnv,
    pub connector: Arc<dyn Connector>,
    pub meter: DayMeter,
    /// Today's local date (the cap's day); a test sets its own.
    pub today: fn() -> chrono::NaiveDate,
}

/// The session's channels to its listeners.
pub struct SessionIo {
    pub frames: broadcast::Receiver<MicFrame>,
    pub out: broadcast::Sender<GuestPacket>,
    pub state: watch::Sender<SessionState>,
    pub stop: watch::Receiver<bool>,
    pub listeners: Arc<AtomicUsize>,
}

struct Conn {
    id: u32,
    tx: mpsc::UnboundedSender<String>,
    abort: AbortHandle,
    ready: bool,
    opened_at: Instant,
    /// The last message from Gemini on this connection.
    last_heard: Instant,
    /// It delivered translated speech (a draining connection's speech is
    /// dropped from then on, so two voices never overlap).
    spoke: bool,
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

struct Session {
    language: Language,
    deps: SessionDeps,
    io_out: broadcast::Sender<GuestPacket>,
    state: watch::Sender<SessionState>,
    listeners: Arc<AtomicUsize>,
    events_tx: mpsc::Sender<ConnEvent>,
    next_id: u32,
    current: Option<Conn>,
    /// Opened on `goAway`, takes over when ready.
    next: Option<Conn>,
    draining: Option<(Conn, Instant)>,
    handle: Option<String>,
    failures: u32,
    retry_at: Option<Instant>,
    stopped: Option<StopReason>,
    pre_roll: VecDeque<MicFrame>,
    pending: VecDeque<MicFrame>,
    last_voiced: Option<Instant>,
    gate_open: bool,
    /// When the gate last opened (the "no answer" rule counts from here, so a
    /// long quiet stretch never reads as a dead connection).
    gate_opened_at: Option<Instant>,
    onset_pending: Option<Instant>,
    last_output: Option<Instant>,
    in_stall: bool,
    stats: MinuteStats,
}

pub async fn run(language: Language, deps: SessionDeps, io: SessionIo) {
    let SessionIo {
        mut frames,
        out,
        state,
        mut stop,
        listeners,
    } = io;
    let (events_tx, mut events) = mpsc::channel(256);
    let mut session = Session::new(language, deps, out, state, listeners, events_tx);
    tracing::info!(target: TARGET, language = language.code, model = session.deps.env.model.as_str(), source_mic = session.deps.env.source_mic.as_str(), "guest translation session started");
    session.connect();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut summary = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(60),
        Duration::from_secs(60),
    );
    summary.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        let retry_at = session.retry_at;
        let drain_at = session.draining.as_ref().map(|(_, at)| *at);
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            event = events.recv() => match event {
                Some(event) => session.on_event(event),
                None => break,
            },
            frame = frames.recv() => match frame {
                Ok(frame) => session.on_frame(frame),
                Err(RecvError::Lagged(missed)) => session.on_lagged(missed),
                Err(RecvError::Closed) => break,
            },
            () = sleep_until(retry_at) => {
                session.retry_at = None;
                session.connect();
            }
            () = sleep_until(drain_at) => {
                if let Some((conn, _)) = session.draining.take() {
                    tracing::debug!(target: TARGET, language = language.code, conn = conn.id, "replaced Gemini connection drained");
                }
            }
            _ = tick.tick() => session.on_tick(),
            _ = summary.tick() => session.summary(),
        }
    }
    session.summary();
    tracing::info!(target: TARGET, language = language.code, reason = ?session.stopped, "guest translation session ended");
}

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// The pause before reconnect attempt `failures + 1`: 1, 2, 4 … s, ≤ 30 s.
pub fn retry_delay(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(16);
    Duration::from_secs(1_u64 << shift).min(RETRY_CAP)
}

impl Session {
    fn new(
        language: Language,
        deps: SessionDeps,
        io_out: broadcast::Sender<GuestPacket>,
        state: watch::Sender<SessionState>,
        listeners: Arc<AtomicUsize>,
        events_tx: mpsc::Sender<ConnEvent>,
    ) -> Self {
        Self {
            language,
            deps,
            io_out,
            state,
            listeners,
            events_tx,
            next_id: 0,
            current: None,
            next: None,
            draining: None,
            handle: None,
            failures: 0,
            retry_at: None,
            stopped: None,
            pre_roll: VecDeque::with_capacity(PRE_ROLL_FRAMES),
            pending: VecDeque::new(),
            last_voiced: None,
            gate_open: false,
            gate_opened_at: None,
            onset_pending: None,
            last_output: None,
            in_stall: false,
            stats: MinuteStats::default(),
        }
    }

    fn set_state(&self, next: SessionState) {
        self.state.send_if_modified(|state| {
            let changed = *state != next;
            if changed {
                tracing::info!(target: TARGET, language = self.language.code, from = ?state, to = ?next, "guest translation state");
                *state = next.clone();
            }
            changed
        });
    }

    fn open(&mut self) -> Option<Conn> {
        let req = match conn::request(&self.deps.env) {
            Ok(req) => req,
            Err(error) => {
                tracing::warn!(target: TARGET, language = self.language.code, error = %error, "no Gemini request can be made");
                self.stop(StopReason::Key);
                return None;
            }
        };
        self.next_id += 1;
        let id = self.next_id;
        let setup = setup_message(
            &self.deps.env.model,
            self.language.code,
            self.handle.as_deref(),
        );
        let (tx, abort) = conn::spawn(
            id,
            Arc::clone(&self.deps.connector),
            req,
            setup,
            self.events_tx.clone(),
        );
        tracing::info!(target: TARGET, language = self.language.code, conn = id, resumed = self.handle.is_some(), "Gemini connection opening");
        let now = Instant::now();
        Some(Conn {
            id,
            tx,
            abort,
            ready: false,
            opened_at: now,
            last_heard: now,
            spoke: false,
        })
    }

    /// (Re)open the current connection (never after a stop).
    fn connect(&mut self) {
        if self.stopped.is_some() || self.current.is_some() {
            return;
        }
        if let Some(conn) = self.open() {
            self.current = Some(conn);
            if self.state.borrow().is_ready() || self.failures > 0 {
                self.set_state(SessionState::Reconnecting);
            } else {
                self.set_state(SessionState::Connecting);
            }
        }
    }

    fn stop(&mut self, reason: StopReason) {
        if self.stopped.is_some() {
            return;
        }
        tracing::warn!(target: TARGET, language = self.language.code, reason = ?reason, minutes_today = self.deps.meter.minutes((self.deps.today)()), "guest translation stopped");
        self.stopped = Some(reason);
        self.current = None;
        self.next = None;
        self.draining = None;
        self.retry_at = None;
        self.pending.clear();
        self.set_state(SessionState::Stopped { reason });
    }

    fn on_lagged(&mut self, missed: u64) {
        self.stats.dropped_frames += missed;
        if should_log_event(self.stats.dropped_frames) {
            tracing::warn!(target: TARGET, language = self.language.code, missed, "guest translation fell behind the mic — frames lost");
        }
    }

    fn on_frame(&mut self, frame: MicFrame) {
        if self.stopped.is_some() {
            return;
        }
        let now = Instant::now();
        if is_voiced(rms_dbfs(&frame)) {
            if self.last_voiced.is_none_or(|at| now - at >= ONSET_GAP) {
                self.stats.onsets += 1;
                self.onset_pending = Some(now);
                self.in_stall = false;
                self.last_output = None;
            }
            self.last_voiced = Some(now);
        }
        let open = self.last_voiced.is_some_and(|at| now - at <= HANGOVER);
        if open && !self.gate_open {
            self.gate_opened_at = Some(now);
            for held in std::mem::take(&mut self.pre_roll) {
                self.send(held);
            }
        }
        self.gate_open = open;
        if open {
            self.send(frame);
        } else {
            if self.pre_roll.len() == PRE_ROLL_FRAMES {
                self.pre_roll.pop_front();
            }
            self.pre_roll.push_back(frame);
            self.refresh_live_state();
        }
    }

    /// One frame to the ready connection, counted against the day's cap
    /// (charged only when it really goes out). `false` = the cap is reached.
    fn deliver(&mut self, frame: &MicFrame) -> bool {
        let Some(conn) = self.current.as_ref().filter(|conn| conn.ready) else {
            return true;
        };
        if !self.deps.meter.spend((self.deps.today)(), FRAME_MS) {
            return false;
        }
        if conn.tx.send(audio_message(frame)).is_ok() {
            self.stats.sent_ms += FRAME_MS;
        }
        true
    }

    fn send(&mut self, frame: MicFrame) {
        match self.current.as_ref().filter(|conn| conn.ready) {
            Some(_) => {
                if !self.deliver(&frame) {
                    self.stop(StopReason::DailyCap);
                    return;
                }
            }
            None => {
                if self.pending.len() == PENDING_FRAMES {
                    self.pending.pop_front();
                    self.stats.dropped_frames += 1;
                }
                self.pending.push_back(frame);
                // A scheduled retry keeps its back-off; it sends what waits here.
                if self.retry_at.is_none() {
                    self.connect();
                }
            }
        }
        self.refresh_live_state();
    }

    fn flush_pending(&mut self) {
        if !self.current.as_ref().is_some_and(|conn| conn.ready) {
            return;
        }
        let held = std::mem::take(&mut self.pending);
        let count = held.len();
        for frame in held {
            if !self.deliver(&frame) {
                self.stop(StopReason::DailyCap);
                return;
            }
        }
        if count > 0 {
            tracing::debug!(target: TARGET, language = self.language.code, frames = count, "held audio sent to the new connection");
        }
    }

    /// `live` while the gate is open on a ready connection, `waiting` when
    /// the mic is quiet.
    fn refresh_live_state(&self) {
        if self.stopped.is_some() {
            return;
        }
        if self.current.as_ref().is_some_and(|conn| conn.ready) {
            if self.gate_open {
                self.set_state(SessionState::Live);
            } else {
                self.set_state(SessionState::Waiting);
            }
        }
    }

    fn on_event(&mut self, event: ConnEvent) {
        match event {
            ConnEvent::Message { conn, message } => self.on_message(conn, message),
            ConnEvent::Closed { conn, code, reason } => self.on_closed(conn, code, &reason),
            ConnEvent::Failed {
                conn,
                status,
                error,
            } => self.on_failed(conn, status, &error),
        }
    }

    fn is(&self, slot: Option<&Conn>, id: u32) -> bool {
        slot.is_some_and(|conn| conn.id == id)
    }

    fn on_message(&mut self, id: u32, message: ServerMessage) {
        let from_current = self.is(self.current.as_ref(), id);
        let from_next = self.is(self.next.as_ref(), id);
        let from_draining = self
            .draining
            .as_ref()
            .is_some_and(|(conn, _)| conn.id == id);
        if !(from_current || from_next || from_draining) {
            return;
        }
        let now = Instant::now();
        for conn in [self.current.as_mut(), self.next.as_mut()]
            .into_iter()
            .flatten()
        {
            if conn.id == id {
                conn.last_heard = now;
            }
        }
        if message.setup_complete {
            if from_next {
                self.switch_to_next();
            } else if let Some(conn) = self.current.as_mut().filter(|conn| conn.id == id) {
                conn.ready = true;
                tracing::info!(target: TARGET, language = self.language.code, conn = id, "Gemini connection ready");
                self.flush_pending();
                self.refresh_live_state();
            }
        }
        if let Some(rate) = &message.unexpected_rate {
            tracing::warn!(target: TARGET, language = self.language.code, mime = rate.as_str(), "Gemini audio at an unexpected rate");
        }
        self.forward(&message, from_draining);
        if from_current || from_next {
            if let Some(handle) = message.handle {
                self.handle = Some(handle);
            }
        }
        if from_current && message.go_away.is_some() && self.next.is_none() {
            tracing::info!(target: TARGET, language = self.language.code, conn = id, time_left = message.go_away.as_deref().unwrap_or(""), resumable = self.handle.is_some(), "Gemini goAway — opening the next connection");
            self.next = self.open();
            self.stats.reconnects += 1;
        }
    }

    fn switch_to_next(&mut self) {
        let Some(mut next) = self.next.take() else {
            return;
        };
        next.ready = true;
        if let Some(old) = self.current.take() {
            self.draining = Some((old, Instant::now() + DRAIN));
        }
        tracing::info!(target: TARGET, language = self.language.code, conn = next.id, "audio moved to the next Gemini connection");
        self.current = Some(next);
        self.flush_pending();
        self.refresh_live_state();
    }

    /// Hands a message's speech and text to the listeners. A draining
    /// connection's speech is dropped once the new one speaks (no two voices).
    fn forward(&mut self, message: &ServerMessage, from_draining: bool) {
        let now = Instant::now();
        let new_spoke = self.current.as_ref().is_some_and(|conn| conn.spoke);
        if from_draining && new_spoke && !message.audio.is_empty() {
            self.stats.overlap_dropped_ms += message
                .audio
                .iter()
                .map(|c| c.len() as u64 / 48)
                .sum::<u64>();
        }
        let audio: &[Vec<u8>] = if from_draining && new_spoke {
            &[]
        } else {
            &message.audio
        };
        if !from_draining && !audio.is_empty() {
            if let Some(conn) = self.current.as_mut() {
                conn.spoke = true;
            }
        }
        for chunk in audio {
            if let Some(onset) = self.onset_pending.take() {
                let ms = u64::try_from((now - onset).as_millis()).unwrap_or(u64::MAX);
                self.stats.first_voiced_ms.push(ms);
                tracing::debug!(target: TARGET, language = self.language.code, first_voiced_ms = ms, "first translated audio after an onset");
            }
            self.last_output = Some(now);
            self.in_stall = false;
            self.failures = 0;
            self.stats.output_ms += chunk.len() as u64 / 48;
            let _ = self
                .io_out
                .send(GuestPacket::Audio(Arc::from(chunk.as_slice())));
        }
        if let Some(text) = &message.output_text {
            self.stats.output_chars += text.chars().count() as u64;
            let _ = self.io_out.send(GuestPacket::Text(text.clone()));
        }
        if let Some(text) = &message.input_text {
            let _ = self.io_out.send(GuestPacket::Original(text.clone()));
        }
        if message.turn_complete {
            let _ = self.io_out.send(GuestPacket::TurnComplete);
        }
    }

    fn on_closed(&mut self, id: u32, code: Option<u16>, reason: &str) {
        if self
            .draining
            .as_ref()
            .is_some_and(|(conn, _)| conn.id == id)
        {
            self.draining = None;
            return;
        }
        if self.is(self.next.as_ref(), id) {
            tracing::warn!(target: TARGET, language = self.language.code, conn = id, code, reason, "the next Gemini connection closed before it was ready");
            self.next = None;
            return;
        }
        if !self.is(self.current.as_ref(), id) {
            return;
        }
        let young = self
            .current
            .take()
            .is_some_and(|conn| !conn.ready || conn.opened_at.elapsed() < IDLE_CUT_MIN_AGE);
        let class = match classify_close(code, reason) {
            CloseClass::Idle if young => CloseClass::Retry,
            class => class,
        };
        tracing::info!(target: TARGET, language = self.language.code, conn = id, code, reason, class = ?class, "Gemini connection closed");
        match class {
            CloseClass::Fatal if code == Some(1007) => self.stop(StopReason::Key),
            CloseClass::Fatal => self.stop(StopReason::Model),
            CloseClass::Idle => {
                if let Some(next) = self.next.take() {
                    self.promote(next);
                } else {
                    self.connect();
                }
            }
            CloseClass::Retry => {
                if let Some(next) = self.next.take() {
                    self.promote(next);
                } else {
                    self.schedule_retry();
                }
            }
        }
    }

    /// The `goAway` successor takes over before it is ready: nothing is
    /// connected until its setup completes.
    fn promote(&mut self, next: Conn) {
        let ready = next.ready;
        self.current = Some(next);
        if !ready {
            self.set_state(SessionState::Reconnecting);
        }
    }

    fn on_failed(&mut self, id: u32, status: Option<u16>, error: &str) {
        if self.is(self.next.as_ref(), id) {
            tracing::warn!(target: TARGET, language = self.language.code, conn = id, status, error, "the next Gemini connection failed");
            self.next = None;
            return;
        }
        if !self.is(self.current.as_ref(), id) {
            return;
        }
        self.current = None;
        match status {
            Some(401 | 403) => self.stop(StopReason::Key),
            Some(400 | 404) => self.stop(StopReason::Model),
            _ => {
                tracing::warn!(target: TARGET, language = self.language.code, conn = id, status, error, "Gemini connection failed");
                self.schedule_retry();
            }
        }
    }

    fn schedule_retry(&mut self) {
        self.failures += 1;
        self.stats.reconnects += 1;
        let delay = retry_delay(self.failures);
        if should_log_event(u64::from(self.failures)) {
            tracing::warn!(target: TARGET, language = self.language.code, failures = self.failures, retry_in_s = delay.as_secs(), "Gemini reconnect scheduled");
        }
        self.retry_at = Some(Instant::now() + delay);
        self.set_state(SessionState::Reconnecting);
    }

    /// Stalls: the preacher spoke in the last second, the first translated
    /// audio of this stretch came, and nothing played for [`STALL_AFTER`].
    fn on_tick(&mut self) {
        let now = Instant::now();
        let sending = self
            .last_voiced
            .is_some_and(|at| now - at <= HANGOVER + Duration::from_secs(1));
        let deaf = match (self.current.as_ref(), self.gate_opened_at) {
            (Some(conn), Some(opened)) if conn.ready && sending => {
                now - conn.last_heard.max(opened) >= NO_ANSWER
            }
            _ => false,
        };
        if deaf {
            if let Some(conn) = self.current.take() {
                tracing::warn!(target: TARGET, language = self.language.code, conn = conn.id, silent_s = (now - conn.last_heard).as_secs(), "Gemini connection answers nothing while audio goes out — dropped");
            }
            self.schedule_retry();
        }
        let speaking = self
            .last_voiced
            .is_some_and(|at| now - at <= Duration::from_secs(1));
        if let Some(last) = self.last_output {
            if speaking && !self.in_stall && now - last >= STALL_AFTER {
                self.in_stall = true;
                self.stats.stalls += 1;
                tracing::warn!(target: TARGET, language = self.language.code, silent_ms = u64::try_from((now - last).as_millis()).unwrap_or(u64::MAX), "guest translation stalled while the preacher speaks");
            }
        }
        if self
            .onset_pending
            .is_some_and(|at| now - at > Duration::from_secs(20))
        {
            self.onset_pending = None;
        }
    }

    fn summary(&mut self) {
        let listeners = self.listeners.load(Ordering::SeqCst);
        let connected = self.current.as_ref().is_some_and(|conn| conn.ready);
        self.stats
            .log_and_reset(self.language.code, listeners, connected);
    }
}
