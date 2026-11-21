//! The logical Soniox session as a sans-I/O state machine (#852 C3-4, plan
//! Decision 5, spec §22.5): [`SonioxCore`] turns mic frames, connection
//! reports and the clock into [`Action`]s; the provider task (C3-5) only
//! executes them. No tokio, no IO, time is injected — every rule is a unit
//! test on hand-made `Instant`s (`machine/tests.rs`).
//!
//! Rules:
//! - lazy connect: no connection until the first voiced frame; the 1 s
//!   pre-roll goes first, frames that arrive meanwhile are queued (≤ 5 s,
//!   older ones dropped and counted);
//! - silence gate: 10 s without a voiced frame (quiet frames or none) →
//!   `finalize` once, no audio, `keepalive` every 5 s; a voiced frame
//!   resumes with the pre-roll on the same connection; 5 min gated → park
//!   (empty text frame), the next voiced frame reconnects without back-off;
//! - reconnect after 1, 5, 20, 60, 60 … s on a lost connection, a failed
//!   connect, a retryable error or a stall (15 s without a response while
//!   ≥ 10 s of audio went out); the audio after the last FINAL word (≤ 5 s)
//!   is sent again first, then the queue;
//! - stop without retry on Auth / Payment / BadRequest and on more than 20
//!   connections in 10 min;
//! - rollover from 280 min of connection age at a gated moment (295 min
//!   regardless): `finalize`, wait ≤ 3 s for `<fin>`, end, connect anew;
//! - never a WebSocket ping (the connection task sends none).
//!
//! Session ms = the position in the audio SENT to Soniox: a frame gets its
//! ms when it is first sent (a gated or dropped frame never gets one), a
//! resend keeps them, so every connection's audio is contiguous from its
//! `offset_ms` and `offset_ms + token ms` is the session ms.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use presenter_core::listen::transcript::TokenBatch;
use presenter_core::listen::transcription::{OffReason, TranscriptionState};

use super::wire::{Control, SonioxResponse};
use crate::listen::MicFrame;

/// No voiced frame for this long → gate (spec §22.5).
pub const GATE_AFTER: Duration = Duration::from_secs(10);
/// Keepalives while gated (Soniox: ≤ 20 s apart; LiveKit's plugin: 5 s).
pub const KEEPALIVE_EVERY: Duration = Duration::from_secs(5);
/// Gated this long → the connection ends (bounds the billed time).
pub const PARK_AFTER: Duration = Duration::from_secs(5 * 60);
/// 10 frames of 100 ms = the 1 s pre-roll.
pub const PRE_ROLL_FRAMES: usize = 10;
/// A pre-roll frame older than this is stale (the mic paused meanwhile).
pub const PRE_ROLL: Duration = Duration::from_secs(1);
/// The sent audio kept for a resend after a lost connection.
pub const REPLAY_MAX_MS: u64 = 5_000;
/// Frames kept while no connection takes audio.
pub const QUEUE_MAX_MS: u64 = 5_000;
/// No response for this long while audio flows = a dead socket (measured
/// cadence p50 0.19 s, max 1.19 s).
pub const STALL_AFTER: Duration = Duration::from_secs(15);
pub const STALL_MIN_AUDIO_MS: u64 = 10_000;
/// The pauses before the 1st, 2nd, 3rd, 4th and every later retry.
pub const BACKOFF: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(20),
    Duration::from_secs(60),
];
/// Soniox ends a stream at 300 min: roll over before.
pub const ROLLOVER_FROM: Duration = Duration::from_secs(280 * 60);
pub const ROLLOVER_HARD: Duration = Duration::from_secs(295 * 60);
/// The longest wait for `<fin>` (rollover) or `finished` (stop, drain).
pub const FIN_WAIT: Duration = Duration::from_secs(3);
/// More connections than this in [`CONNECTION_WINDOW`] = a cost runaway.
pub const MAX_CONNECTIONS: usize = 20;
pub const CONNECTION_WINDOW: Duration = Duration::from_secs(10 * 60);
/// How often the running totals go to the meter while connected.
pub const USAGE_EVERY: Duration = Duration::from_secs(10);
/// 16 kHz mono: 16 samples per ms.
const SAMPLES_PER_MS: usize = 16;
const FIN_MARKER: &str = "<fin>";
const END_MARKER: &str = "<end>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachineConfig {
    /// No key = the first input finishes the session with `NoKey`. The
    /// config JSON is built per connection by the driver (C3-5), which
    /// also owns the context.
    pub key_present: bool,
}

#[derive(Debug, Clone)]
pub enum Input {
    /// One 100 ms mic frame; `voiced` = above −50 dBFS.
    Frame {
        pcm: MicFrame,
        voiced: bool,
        at: Instant,
    },
    Stop {
        at: Instant,
    },
}

/// What a connection task reports.
#[derive(Debug, Clone)]
pub enum Report {
    Connected {
        conn: u32,
        at: Instant,
    },
    Response {
        conn: u32,
        resp: SonioxResponse,
        at: Instant,
        /// The wall clock of the response (the batch's `received_at`).
        received_at: DateTime<Utc>,
    },
    /// `code` = the close frame's code; `None` = no close frame (IO error).
    Closed {
        conn: u32,
        code: Option<u16>,
        reason: String,
        at: Instant,
    },
    /// `status` = the HTTP status of a refused upgrade, if there was one.
    /// The machine has no connect deadline: the driver reports this after
    /// its own connect timeout (10 s, C3-5).
    ConnectFailed {
        conn: u32,
        status: Option<u16>,
        error: String,
        at: Instant,
    },
}

impl Report {
    fn at(&self) -> Instant {
        match self {
            Self::Connected { at, .. }
            | Self::Response { at, .. }
            | Self::Closed { at, .. }
            | Self::ConnectFailed { at, .. } => *at,
        }
    }
}

/// Why a connection is opened (for the driver's log line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectKind {
    /// The first voiced frame (session start or after a park).
    Lazy,
    /// After a lost connection.
    Retry,
    /// Before the 300-min limit.
    Rollover,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Connect {
        conn: u32,
        offset_ms: u64,
        kind: ConnectKind,
    },
    SendAudio {
        conn: u32,
        pcm: MicFrame,
    },
    SendControl {
        conn: u32,
        control: Control,
    },
    /// The empty TEXT frame: the stream ends after its finals.
    EndAudio {
        conn: u32,
    },
    /// Abort the connection task.
    Drop {
        conn: u32,
    },
    Emit(TokenBatch),
    /// The assembler drops this connection's live tail.
    ConnectionEnded {
        conn: u32,
    },
    State {
        state: TranscriptionState,
        error: Option<String>,
    },
    /// The session's running totals (not deltas).
    Usage {
        audio_ms: u64,
        connected_ms: u64,
    },
    Onset {
        latency_ms: u32,
    },
    Finished(Option<OffReason>),
}

/// Counters for the session's summary log line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MachineStats {
    pub connections: u32,
    pub reconnects: u32,
    pub tokens_final: u64,
    /// Queued frames dropped because no connection took them within 5 s.
    pub dropped_frames: u64,
    /// Audio sent, resends included (what Soniox bills).
    pub audio_ms: u64,
}

#[derive(Debug)]
enum Phase {
    /// No connection: waiting for a voiced frame.
    Idle,
    Connecting {
        id: u32,
        offset_ms: u64,
        kind: ConnectKind,
    },
    Live(Live),
    Backoff {
        until: Instant,
    },
    /// `finalize` sent before a rollover; waiting for `<fin>`.
    Rolling {
        live: Live,
        until: Instant,
    },
    /// `finalize` + end sent; waiting for `finished`.
    Stopping {
        live: Live,
        until: Instant,
    },
    Done,
}

#[derive(Debug)]
struct Live {
    id: u32,
    offset_ms: u64,
    connected_at: Instant,
    last_response_at: Instant,
    audio_since_response_ms: u64,
    gate: Option<Gate>,
}

#[derive(Debug, Clone, Copy)]
struct Gate {
    since: Instant,
    next_keepalive: Instant,
}

#[derive(Debug)]
struct Sent {
    start_ms: u64,
    pcm: MicFrame,
}

impl Sent {
    fn end_ms(&self) -> u64 {
        self.start_ms + frame_ms(&self.pcm)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retry {
    /// Reconnect at once (or wait for voice when nobody speaks).
    Now,
    Backoff,
}

/// The session machine of one guest-mic transcription session.
#[derive(Debug)]
pub struct SonioxCore {
    key_present: bool,
    phase: Phase,
    state: TranscriptionState,
    next_conn: u32,
    /// Unsent frames before a voiced one, with their arrival (≤ 1 s).
    preroll: VecDeque<(Instant, MicFrame)>,
    /// Frames waiting for a connection (≤ 5 s).
    pending: VecDeque<MicFrame>,
    pending_ms: u64,
    /// Sent frames after the last confirmed ms (≤ 5 s).
    replay: VecDeque<Sent>,
    sent_end_ms: u64,
    /// Session ms up to which Soniox gave finals (a final's end, or the
    /// audio a `<fin>` answered).
    confirmed_ms: u64,
    /// The session ms each sent `finalize` covers, oldest first, until
    /// its `<fin>`.
    finalizes: VecDeque<u64>,
    /// Audio went out on this connection since the last `finalize`.
    unfinalized: bool,
    last_voiced_at: Option<Instant>,
    /// A voiced onset: when its first frame went out and its session ms.
    onset: Option<(Instant, u64)>,
    backoff_step: usize,
    connects: VecDeque<Instant>,
    /// Ended connections waiting for `finished`; their tokens are ignored
    /// (anything unconfirmed is resent on the next connection).
    draining: Vec<(u32, Instant)>,
    closed_connected_ms: u64,
    next_usage_at: Option<Instant>,
    stats: MachineStats,
}

impl SonioxCore {
    pub fn new(cfg: MachineConfig) -> Self {
        Self {
            key_present: cfg.key_present,
            phase: Phase::Idle,
            state: TranscriptionState::Armed,
            next_conn: 1,
            preroll: VecDeque::with_capacity(PRE_ROLL_FRAMES + 1),
            pending: VecDeque::new(),
            pending_ms: 0,
            replay: VecDeque::new(),
            sent_end_ms: 0,
            confirmed_ms: 0,
            finalizes: VecDeque::new(),
            unfinalized: false,
            last_voiced_at: None,
            onset: None,
            backoff_step: 0,
            connects: VecDeque::new(),
            draining: Vec::new(),
            closed_connected_ms: 0,
            next_usage_at: None,
            stats: MachineStats::default(),
        }
    }

    /// The state last reported (`Armed` before the first change); the
    /// driver reads the reported states, the tests read this.
    #[cfg(test)]
    pub fn state(&self) -> TranscriptionState {
        self.state
    }

    pub fn stats(&self) -> MachineStats {
        self.stats
    }

    pub fn on_input(&mut self, input: Input) -> Vec<Action> {
        let mut out = Vec::new();
        if matches!(self.phase, Phase::Done) {
            return out;
        }
        if !self.key_present {
            self.phase = Phase::Done;
            out.push(Action::Finished(Some(OffReason::NoKey)));
            return out;
        }
        match input {
            Input::Frame { pcm, voiced, at } => {
                if voiced {
                    self.last_voiced_at = Some(at);
                }
                self.check_timers(at, &mut out);
                self.on_frame(pcm, voiced, at, &mut out);
            }
            Input::Stop { at } => self.stop(at, &mut out),
        }
        out
    }

    pub fn on_report(&mut self, report: Report) -> Vec<Action> {
        let mut out = Vec::new();
        if matches!(self.phase, Phase::Done) {
            return out;
        }
        let at = report.at();
        match report {
            Report::Connected { conn, at } => self.on_connected(conn, at, &mut out),
            Report::Response {
                conn,
                resp,
                at,
                received_at,
            } => self.on_response(conn, resp, at, received_at, &mut out),
            Report::Closed {
                conn,
                code,
                reason,
                at,
            } => self.on_closed(conn, code, &reason, at, &mut out),
            Report::ConnectFailed {
                conn,
                status,
                error,
                at,
            } => self.on_connect_failed(conn, status, &error, at, &mut out),
        }
        self.check_timers(at, &mut out);
        out
    }

    pub fn on_tick(&mut self, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        self.check_timers(now, &mut out);
        out
    }

    /// When the running back-off ends (`None` = none runs; the driver's
    /// reconnect log line).
    pub fn retry_at(&self) -> Option<Instant> {
        match self.phase {
            Phase::Backoff { until } => Some(until),
            _ => None,
        }
    }

    /// The earliest moment [`on_tick`](Self::on_tick) has something to do.
    pub fn next_deadline(&self) -> Option<Instant> {
        let phase = match &self.phase {
            Phase::Live(live) => Some(self.live_deadline(live)),
            Phase::Backoff { until }
            | Phase::Rolling { until, .. }
            | Phase::Stopping { until, .. } => Some(*until),
            Phase::Idle | Phase::Connecting { .. } | Phase::Done => None,
        };
        self.draining
            .iter()
            .map(|(_, until)| *until)
            .chain(self.next_usage_at)
            .chain(phase)
            .min()
    }

    fn live_deadline(&self, live: &Live) -> Instant {
        match live.gate {
            None => {
                let gate = self.last_voiced_at.unwrap_or(live.connected_at) + GATE_AFTER;
                let stall = (live.audio_since_response_ms >= STALL_MIN_AUDIO_MS)
                    .then(|| live.last_response_at + STALL_AFTER);
                let hard = live.connected_at + ROLLOVER_HARD;
                stall.map_or(gate, |stall| stall.min(gate)).min(hard)
            }
            Some(gate) => gate
                .next_keepalive
                .min(gate.since + PARK_AFTER)
                .min(live.connected_at + ROLLOVER_FROM),
        }
    }

    // ---- inputs ----

    fn on_frame(&mut self, pcm: MicFrame, voiced: bool, at: Instant, out: &mut Vec<Action>) {
        let gated = match &self.phase {
            Phase::Live(live) => Some(live.gate.is_some()),
            _ => None,
        };
        match (&self.phase, gated) {
            (Phase::Idle, _) => {
                self.push_preroll(at, pcm);
                if voiced {
                    for frame in self.take_preroll(at) {
                        self.queue(frame);
                    }
                    self.connect(at, ConnectKind::Lazy, out);
                }
            }
            (Phase::Live(_), Some(false)) => self.send_new(pcm, out),
            (Phase::Live(_), _) => {
                self.push_preroll(at, pcm);
                if voiced {
                    self.resume(at, out);
                }
            }
            (Phase::Connecting { .. } | Phase::Backoff { .. } | Phase::Rolling { .. }, _) => {
                // An onset while no connection takes audio keeps its pre-roll.
                if voiced {
                    for frame in self.take_preroll(at) {
                        self.queue(frame);
                    }
                }
                if self.speaking(at) {
                    self.queue(pcm);
                } else {
                    self.push_preroll(at, pcm);
                }
            }
            (Phase::Stopping { .. } | Phase::Done, _) => {}
        }
    }

    fn stop(&mut self, at: Instant, out: &mut Vec<Action>) {
        if matches!(self.phase, Phase::Stopping { .. }) {
            return;
        }
        match self.take_live() {
            Some(live) => {
                let conn = live.id;
                out.push(Action::SendControl {
                    conn,
                    control: Control::Finalize,
                });
                out.push(Action::EndAudio { conn });
                self.phase = Phase::Stopping {
                    live,
                    until: at + FIN_WAIT,
                };
            }
            None => self.finish(None, None, at, out),
        }
    }

    // ---- helpers ----

    fn speaking(&self, now: Instant) -> bool {
        self.last_voiced_at
            .is_some_and(|t| now.saturating_duration_since(t) < GATE_AFTER)
    }

    fn live_mut(&mut self) -> Option<&mut Live> {
        match &mut self.phase {
            Phase::Live(live) | Phase::Rolling { live, .. } | Phase::Stopping { live, .. } => {
                Some(live)
            }
            _ => None,
        }
    }

    fn take_live(&mut self) -> Option<Live> {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Live(live) | Phase::Rolling { live, .. } | Phase::Stopping { live, .. } => {
                Some(live)
            }
            other => {
                self.phase = other;
                None
            }
        }
    }

    fn active_id(&self) -> Option<u32> {
        match &self.phase {
            Phase::Connecting { id, .. } => Some(*id),
            Phase::Live(live) | Phase::Rolling { live, .. } | Phase::Stopping { live, .. } => {
                Some(live.id)
            }
            _ => None,
        }
    }

    fn usage(&self, now: Instant) -> Action {
        let open = match &self.phase {
            Phase::Live(live) | Phase::Rolling { live, .. } | Phase::Stopping { live, .. } => {
                ms_u64(now.saturating_duration_since(live.connected_at))
            }
            _ => 0,
        };
        Action::Usage {
            audio_ms: self.stats.audio_ms,
            connected_ms: self.closed_connected_ms.saturating_add(open),
        }
    }

    fn set_state(
        &mut self,
        state: TranscriptionState,
        error: Option<String>,
        out: &mut Vec<Action>,
    ) {
        if state != self.state || error.is_some() {
            self.state = state;
            out.push(Action::State { state, error });
        }
    }
}

mod audio;
mod reports;
mod timers;

fn frame_ms(pcm: &[i16]) -> u64 {
    (pcm.len() / SAMPLES_PER_MS) as u64
}

fn ms_u64(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn ms_u32(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
