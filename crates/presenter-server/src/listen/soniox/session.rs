//! The Soniox provider task (#852 C3-5, plan Task 5.4): [`SonioxAsr`] is the
//! production [`StreamingAsr`]. Each session runs ONE task that feeds the
//! pure [`SonioxCore`] with mic frames, connection reports and its own
//! deadline, and executes the actions it returns — spawning / aborting the
//! connection tasks (`conn`), writing their commands and turning the rest
//! into [`AsrEvent`]s. No rule lives here; the machine decides.
//!
//! Logs (`presenter::listen::asr`, plan Decision 16): one line per open,
//! connect, rollover, close (code + reason, key redacted), gate, park; the
//! reconnects WARN on power-of-two attempts of a streak (#484) with the
//! wait, one recovery line, one summary per session.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use presenter_core::audio::level::VOICED_DBFS;
use presenter_core::listen::transcription::{cost_usd, OffReason, TranscriptionState};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use super::config::SonioxEnv;
use super::conn::{self, ConnCommand, Connector, TlsConnector};
use super::machine::{Action, ConnectKind, Input, MachineConfig, Report, SonioxCore};
use super::wire::{config_message, redact, SonioxConfig};
use crate::listen::asr::{
    AsrEvent, AsrInput, AsrSession, AsrSessionConfig, StreamingAsr, EVENT_CAPACITY, INPUT_CAPACITY,
};
use crate::listen::should_log_event;

const TARGET: &str = "presenter::listen::asr";
/// Reports from all connection tasks of one session.
const REPORT_CAPACITY: usize = 256;

pub struct SonioxAsr {
    env: Arc<SonioxEnv>,
    connector: Arc<dyn Connector>,
}

impl SonioxAsr {
    pub fn new(env: SonioxEnv, connector: Arc<dyn Connector>) -> Self {
        Self {
            env: Arc::new(env),
            connector,
        }
    }

    /// The real API over TCP + TLS.
    pub fn production(env: SonioxEnv) -> Self {
        Self::new(env, Arc::new(TlsConnector))
    }
}

impl StreamingAsr for SonioxAsr {
    fn open(&self, cfg: AsrSessionConfig) -> AsrSession {
        let (input, inputs) = mpsc::channel(INPUT_CAPACITY);
        let (events_tx, events) = mpsc::channel(EVENT_CAPACITY);
        let driver = Driver::new(
            Arc::clone(&self.env),
            Arc::clone(&self.connector),
            cfg,
            events_tx,
        );
        tokio::spawn(driver.run(inputs));
        AsrSession { input, events }
    }
}

struct Conn {
    commands: mpsc::UnboundedSender<ConnCommand>,
    abort: AbortHandle,
}

struct Driver {
    env: Arc<SonioxEnv>,
    connector: Arc<dyn Connector>,
    cfg: AsrSessionConfig,
    core: SonioxCore,
    events: mpsc::Sender<AsrEvent>,
    reports_tx: mpsc::Sender<Report>,
    reports: mpsc::Receiver<Report>,
    conns: HashMap<u32, Conn>,
    /// Lost connections since the last streaming state (the WARN gate).
    streak: u64,
    connected_ms: u64,
}

impl Driver {
    fn new(
        env: Arc<SonioxEnv>,
        connector: Arc<dyn Connector>,
        cfg: AsrSessionConfig,
        events: mpsc::Sender<AsrEvent>,
    ) -> Self {
        let (reports_tx, reports) = mpsc::channel(REPORT_CAPACITY);
        let core = SonioxCore::new(MachineConfig {
            key_present: env.key.is_some(),
        });
        Self {
            env,
            connector,
            cfg,
            core,
            events,
            reports_tx,
            reports,
            conns: HashMap::new(),
            streak: 0,
            connected_ms: 0,
        }
    }

    async fn run(mut self, mut inputs: mpsc::Receiver<AsrInput>) {
        let mic = self.cfg.mic_id.clone();
        if self.env.key.is_none() {
            // Nothing to connect with: the status names the missing key.
            // (The machine's own no-key branch is the fallback.)
            tracing::info!(target: TARGET, mic = mic.as_str(), "no Soniox key, transcription session not started");
            let _ = self
                .events
                .send(AsrEvent::Finished(Some(OffReason::NoKey)))
                .await;
            return;
        }
        tracing::info!(target: TARGET, mic = mic.as_str(), model = self.env.model.as_str(), "transcription session opened");
        let mut inputs_open = true;
        loop {
            let deadline = self.core.next_deadline();
            let actions = tokio::select! {
                biased;
                Some(report) = self.reports.recv() => {
                    self.log_report(&report);
                    self.core.on_report(report)
                }
                input = inputs.recv(), if inputs_open => {
                    let at = conn::now();
                    match input {
                        Some(AsrInput::Frame { pcm, dbfs }) => {
                            self.core.on_input(Input::Frame { pcm, voiced: dbfs > VOICED_DBFS, at })
                        }
                        // The owner dropped the sender: the same as a stop.
                        Some(AsrInput::Stop) | None => {
                            inputs_open = false;
                            self.core.on_input(Input::Stop { at })
                        }
                    }
                }
                () = sleep_until(deadline) => self.core.on_tick(conn::now()),
            };
            if !self.execute(actions).await {
                break;
            }
        }
    }

    /// Runs the actions; `false` = the session is over.
    async fn execute(&mut self, actions: Vec<Action>) -> bool {
        let mut queue = VecDeque::from(actions);
        while let Some(action) = queue.pop_front() {
            let event = match action {
                Action::Connect {
                    conn,
                    offset_ms,
                    kind,
                } => {
                    queue.extend(self.connect(conn, offset_ms, kind));
                    continue;
                }
                Action::SendAudio { conn, pcm } => {
                    self.command(conn, ConnCommand::Audio(pcm));
                    continue;
                }
                Action::SendControl { conn, control } => {
                    tracing::debug!(target: TARGET, conn, control = ?control, "control sent");
                    self.command(conn, ConnCommand::Control(control));
                    continue;
                }
                Action::EndAudio { conn } => {
                    self.command(conn, ConnCommand::End);
                    continue;
                }
                Action::Drop { conn } => {
                    if let Some(conn) = self.conns.remove(&conn) {
                        conn.abort.abort();
                    }
                    continue;
                }
                Action::Emit(batch) => AsrEvent::Tokens(batch),
                Action::ConnectionEnded { conn } => AsrEvent::ConnectionEnded(conn),
                Action::State { state, error } => {
                    // Redacted at the source: the status shows this text.
                    let error = error.map(|e| self.redacted(&e));
                    self.log_state(state, error.as_deref());
                    AsrEvent::State { state, error }
                }
                Action::Usage {
                    audio_ms,
                    connected_ms,
                } => {
                    self.connected_ms = connected_ms;
                    AsrEvent::Usage {
                        audio_ms,
                        connected_ms,
                    }
                }
                Action::Onset { latency_ms } => AsrEvent::Onset { latency_ms },
                Action::Finished(reason) => {
                    self.log_summary(reason);
                    let _ = self.events.send(AsrEvent::Finished(reason)).await;
                    return false;
                }
            };
            if self.events.send(event).await.is_err() {
                tracing::info!(target: TARGET, mic = self.cfg.mic_id.as_str(), "transcription events not read any more, session ended");
                return false;
            }
        }
        true
    }

    /// Spawns connection `conn` and queues its config; a request that
    /// cannot be built is reported to the machine as a failed connect.
    fn connect(&mut self, conn: u32, offset_ms: u64, kind: ConnectKind) -> Vec<Action> {
        let req = match conn::request(&self.env) {
            Ok(req) => req,
            Err(error) => {
                return self.core.on_report(Report::ConnectFailed {
                    conn,
                    status: error.status(),
                    error: error.to_string(),
                    at: conn::now(),
                })
            }
        };
        let mic = self.cfg.mic_id.as_str();
        match kind {
            ConnectKind::Lazy => {
                tracing::info!(target: TARGET, mic, conn, offset_ms, "connecting to Soniox")
            }
            ConnectKind::Rollover => {
                tracing::info!(target: TARGET, mic, conn, offset_ms, "rolling over to a new Soniox connection")
            }
            ConnectKind::Retry => {
                tracing::debug!(target: TARGET, mic, conn, offset_ms, "reconnecting to Soniox")
            }
        }
        let (commands, abort) = conn::spawn_connection(
            conn,
            Arc::clone(&self.connector),
            req,
            self.reports_tx.clone(),
        );
        let config = config_message(&SonioxConfig {
            model: self.env.model.clone(),
            context: self.cfg.context.clone(),
            endpoint_delay_ms: self.env.endpoint_delay_ms,
        });
        // An unbounded send fails only once the task ended; its report
        // tells the machine.
        let _ = commands.send(ConnCommand::Config(config));
        self.conns.insert(conn, Conn { commands, abort });
        Vec::new()
    }

    fn command(&self, conn: u32, command: ConnCommand) {
        if let Some(conn) = self.conns.get(&conn) {
            // A failed send = the task already ended; its report follows.
            let _ = conn.commands.send(command);
        }
    }

    fn log_report(&self, report: &Report) {
        if let Report::Closed {
            conn, code, reason, ..
        } = report
        {
            let reason = self.redacted(reason);
            tracing::info!(target: TARGET, mic = self.cfg.mic_id.as_str(), conn, code = ?code, reason = reason.as_str(), "Soniox connection closed");
        }
    }

    fn log_state(&mut self, state: TranscriptionState, error: Option<&str>) {
        let mic = self.cfg.mic_id.as_str();
        let error = error.unwrap_or("");
        if state != TranscriptionState::Reconnecting && self.streak > 0 {
            // Any other state ends the streak (#484: reset on success), so
            // the next outage's first attempt WARNs again.
            tracing::info!(target: TARGET, mic, attempts = self.streak, state = ?state, "Soniox reconnect attempts ended");
            self.streak = 0;
        }
        match state {
            TranscriptionState::Reconnecting => {
                self.streak += 1;
                if should_log_event(self.streak) {
                    // 0 = the retry runs at once (no back-off).
                    let wait = self.core.retry_at().map_or(Duration::ZERO, |at| {
                        at.saturating_duration_since(conn::now())
                    });
                    let wait_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX);
                    tracing::warn!(target: TARGET, mic, attempt = self.streak, wait_ms, error, "Soniox connection lost, reconnecting");
                }
            }
            TranscriptionState::Stopped => {
                tracing::warn!(target: TARGET, mic, error, "transcription stopped, a person must fix it")
            }
            _ => tracing::info!(target: TARGET, mic, state = ?state, "transcription state"),
        }
    }

    fn log_summary(&self, reason: Option<OffReason>) {
        let stats = self.core.stats();
        tracing::info!(
            target: TARGET,
            mic = self.cfg.mic_id.as_str(),
            reason = ?reason,
            audio_ms = stats.audio_ms,
            connected_ms = self.connected_ms,
            connections = stats.connections,
            reconnects = stats.reconnects,
            tokens_final = stats.tokens_final,
            dropped_frames = stats.dropped_frames,
            cost_usd = cost_usd(self.connected_ms),
            "transcription session finished"
        );
    }

    fn redacted(&self, text: &str) -> String {
        self.env
            .key
            .as_deref()
            .map_or_else(|| text.to_string(), |key| redact(text, key))
    }
}

impl Drop for Driver {
    /// However the session task ends (finished, its reader gone, aborted),
    /// no connection task outlives it.
    fn drop(&mut self) {
        for (_, conn) in self.conns.drain() {
            conn.abort.abort();
        }
    }
}

/// Sleeps until `deadline`; forever without one.
async fn sleep_until(deadline: Option<std::time::Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
        None => std::future::pending().await,
    }
}
