//! The transcription supervisor (#852 C3-7, plan Tasks 7.3–7.5, Decisions
//! 9–11; #852 T1): ONE task decides with core `start_decision` which mics'
//! sessions run — every published Dante handheld plus the chosen guest mic,
//! at every service (no Kázeň layout gate since T1) — on every rows /
//! mic-list change and on a 1 s tick. Each running session is ONE mic task
//! (`mic`): the mic's frames in, the provider's events out into the
//! transcript hub, the meter (`meter`) and the mic's spoken language (T2).
//! A session connects only while its mic is voiced (the provider's own gate
//! and park), so a silent handheld costs nothing; ONE daily cap counts every
//! mic's connected time. A session the provider stopped for a person to fix
//! (key, account, request, too many connections) stops EVERY mic until the
//! switch, the guest mic or today's choice changes or the day ends — never a
//! restart loop. A session that ends by itself is retried per mic after its
//! own back-off. [`Transcription`] (owned by `ListenService`) holds the rows,
//! the meter and each mic's state for `GET /listen/status`; its std mutex is
//! never held across an `.await`.

mod meter;
mod mic;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDate;
use futures_util::future::{join_all, select_all};
use presenter_core::listen::context::{context_terms, ContextInputs};
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::transcription::{
    cap_ms, cost_usd, start_decision, today_mode, transcribed_mics, MicTranscription, OffReason,
    SessionOverride, StartInputs, TranscriptionSetting, TranscriptionState, TranscriptionStatus,
    UsageRow,
};
use tokio::sync::watch;
use tokio::time::{interval, Instant, MissedTickBehavior};

use self::meter::Meter;
use self::mic::MicTask;
use super::asr::{AsrSessionConfig, StreamingAsr};
use super::soniox::config::SonioxEnv;
use super::transcript_hub::TranscriptHub;
use super::{bind_retry_delay, should_log_event, ListenService};

/// The target of every transcription log line (plan Decision 16).
pub(crate) const ASR_TARGET: &str = "presenter::listen::asr";
/// The supervisor decides at least this often.
const TICK: Duration = Duration::from_secs(1);
/// A stopped session gets this long to send its last words.
const STOP_WAIT: Duration = Duration::from_secs(5);
/// `listen.usage` is written at most this often while a session runs.
const USAGE_SAVE_EVERY: Duration = Duration::from_secs(60);
/// A write's answer waits for the supervisor's decision at most a stop plus this.
const DECIDE_SLACK: Duration = Duration::from_secs(2);
/// A session that ran this long resets the unexpected-end back-off.
const STABLE_SESSION: Duration = Duration::from_secs(60);

/// The two settings rows the supervisor follows.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Rows {
    pub(crate) setting: TranscriptionSetting,
    pub(crate) over: Option<SessionOverride>,
    /// Counts today's choices (`PUT /listen/session`): a new choice lets a
    /// stopped mic run again, even when it repeats the stored mode.
    pub(crate) choice: u64,
    /// Counts every change; the supervisor reports the one it decided on.
    pub(crate) generation: u64,
}

/// What the supervisor reads from and writes to the database.
// `#[async_trait]` marks the methods `#[must_use]` (clippy::double_must_use).
#[allow(clippy::double_must_use)]
#[async_trait]
pub(crate) trait TranscriptionStore: Send + Sync {
    /// Today's prepared references and active šepkár terms.
    async fn context_inputs(&self, today: NaiveDate) -> anyhow::Result<ContextInputs>;
    /// Write the `listen.usage` row.
    async fn save_usage(&self, row: UsageRow) -> anyhow::Result<()>;
}

/// What the supervisor runs with.
pub(crate) struct TranscriptionDeps {
    pub(crate) asr: Arc<dyn StreamingAsr>,
    pub(crate) key: bool,
    pub(crate) model: String,
    pub(crate) store: Arc<dyn TranscriptionStore>,
}

/// One transcribed mic's state as the status shows it.
#[derive(Debug, Clone)]
struct SessionView {
    mic_id: String,
    state: TranscriptionState,
    reason: Option<OffReason>,
    error: Option<String>,
    cap_ms: u64,
}

struct View {
    key_configured: bool,
    model: String,
    meter: Meter,
    /// The shown mics in `transcribed_mics` order.
    sessions: Vec<SessionView>,
}

impl View {
    fn session_mut(&mut self, mic: &str) -> Option<&mut SessionView> {
        self.sessions.iter_mut().find(|s| s.mic_id == mic)
    }
}

/// The transcription state every `ListenService` clone shares.
pub(crate) struct Transcription {
    rows: watch::Sender<Rows>,
    /// The rows generation the supervisor last decided on.
    decided: watch::Sender<u64>,
    view: Mutex<View>,
}

impl Transcription {
    pub(crate) fn new(env: &SonioxEnv) -> Self {
        let (rows, _) = watch::channel(Rows::default());
        let (decided, _) = watch::channel(0);
        Self {
            rows,
            decided,
            view: Mutex::new(View {
                key_configured: env.key.is_some(),
                model: env.model.clone(),
                meter: Meter::new(None, NaiveDate::MIN),
                sessions: Vec::new(),
            }),
        }
    }

    pub(crate) fn rows(&self) -> Rows {
        self.rows.borrow().clone()
    }

    /// The stored rows, read at startup.
    pub(crate) fn load(&self, setting: TranscriptionSetting, over: Option<SessionOverride>) {
        let _generation = self.change(|rows| {
            rows.setting = setting;
            rows.over = over;
        });
    }

    /// A saved setting; the generation to wait for ([`Self::decided_on`]).
    pub(crate) fn set_setting(&self, setting: TranscriptionSetting) -> u64 {
        self.change(|rows| rows.setting = setting)
    }

    /// Today's choice: also counted, so it unblocks a stopped mic.
    pub(crate) fn set_override(&self, over: SessionOverride) -> u64 {
        self.change(|rows| {
            rows.over = Some(over);
            rows.choice += 1;
        })
    }

    fn change(&self, f: impl FnOnce(&mut Rows)) -> u64 {
        let mut generation = 0;
        self.rows.send_modify(|rows| {
            f(rows);
            rows.generation += 1;
            generation = rows.generation;
        });
        generation
    }

    /// Wait until the supervisor decided on rows `generation`, so a write's
    /// answer shows the state it caused; bounded, since a stop may take
    /// [`STOP_WAIT`].
    pub(crate) async fn decided_on(&self, generation: u64) {
        let mut decided = self.decided.subscribe();
        let waited = tokio::time::timeout(
            STOP_WAIT + DECIDE_SLACK,
            decided.wait_for(|done| *done >= generation),
        )
        .await;
        if waited.is_err() {
            tracing::warn!(target: ASR_TARGET, generation, "the transcription supervisor did not decide in time — the answer shows the state before it");
        }
    }

    /// Today's usage from the stored row (another day's starts at 0).
    pub(crate) fn load_usage(&self, row: Option<UsageRow>, today: NaiveDate) {
        self.with_view(|view| view.meter = Meter::new(row, today));
    }

    /// Today's cap of connected time (every mic's together) is reached.
    pub(crate) fn cap_reached(&self, today: NaiveDate) -> bool {
        let rows = self.rows();
        let connected = self.with_view(|view| {
            view.meter.roll(today);
            view.meter.connected_total()
        });
        connected >= cap_ms(&rows.setting, rows.over.as_ref(), today)
    }

    /// The transcription part of `GET /listen/status`.
    pub(crate) fn status(&self, today: NaiveDate) -> TranscriptionStatus {
        let rows = self.rows();
        self.with_view(|view| {
            view.meter.roll(today);
            TranscriptionStatus {
                key_configured: view.key_configured,
                model: view.model.clone(),
                mode: today_mode(rows.over.as_ref(), today),
                setting: rows.setting.clone(),
                mics: view
                    .sessions
                    .iter()
                    .map(|session| session_status(view, session))
                    .collect(),
            }
        })
    }

    /// Send one mic's state to `/listen/ws` (the hub sends only a visible
    /// change).
    fn publish_mic(&self, hub: &TranscriptHub, mic: &str) {
        let shown = self.with_view(|view| {
            view.sessions
                .iter()
                .find(|s| s.mic_id == mic)
                .map(|session| session_status(view, session))
        });
        if let Some(session) = shown {
            hub.set_session(session);
        }
    }

    fn with_view<R>(&self, f: impl FnOnce(&mut View) -> R) -> R {
        let mut view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut view)
    }
}

/// One mic's `MicTranscription`, counters from the meter.
fn session_status(view: &View, guest: &SessionView) -> MicTranscription {
    let usage = view.meter.usage(&guest.mic_id);
    let (connections, reconnects) = view.meter.counts(&guest.mic_id);
    let latency = view.meter.latency(&guest.mic_id);
    MicTranscription {
        mic_id: guest.mic_id.clone(),
        state: guest.state,
        reason: guest.reason,
        error: guest.error.clone(),
        audio_ms_today: usage.audio_ms,
        connected_ms_today: usage.connected_ms,
        cap_ms: guest.cap_ms,
        cost_usd_today: cost_usd(usage.connected_ms),
        connections,
        reconnects,
        latency_last_ms: latency.last,
        latency_p50_ms: latency.p50,
        latency_p90_ms: latency.p90,
    }
}

/// The provider stopped a session for a person to fix (it stops every
/// mic: a key or an account is the box's), and what must change.
struct Block {
    reason: OffReason,
    error: Option<String>,
    key: BlockKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockKey {
    day: NaiveDate,
    enabled: bool,
    guest: Option<String>,
    choice: u64,
}

impl BlockKey {
    fn of(rows: &Rows, day: NaiveDate) -> Self {
        Self {
            day,
            enabled: rows.setting.enabled,
            guest: rows.setting.guest_mic.clone(),
            choice: rows.choice,
        }
    }
}

/// How a session ended.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Ended {
    /// The provider stopped it for a person to fix: the mic is blocked.
    Refused(OffReason),
    /// The supervisor stopped it.
    Stopped,
    /// It ended by itself without a reason, or its task panicked: retried
    /// after a back-off, never on the next tick.
    Unexpected,
}

type Joined = Result<Option<OffReason>, tokio::task::JoinError>;

/// Why the supervisor woke.
enum Wake {
    /// A running session's task ended by itself (`Err` = it panicked).
    Finished(String, Joined),
    Check,
}

/// The running session and what it ran under.
struct Running {
    task: MicTask,
    /// The rows it started with: a provider stop blocks THOSE rows.
    key: BlockKey,
    started: Instant,
}

/// One mic's unexpected ends in a row and when to try it again.
#[derive(Debug, Clone, Copy, Default)]
struct Retry {
    ends: u32,
    at: Option<Instant>,
}

impl Retry {
    fn backing_off(&self, now: Instant) -> bool {
        self.at.is_some_and(|at| now < at)
    }
}

struct Supervisor {
    listen: ListenService,
    deps: TranscriptionDeps,
    /// The running sessions by mic id.
    running: BTreeMap<String, Running>,
    blocked: Option<Block>,
    /// Per mic: a session stopped for its own "too many connections" (that
    /// one handheld reconnecting, never the box's key or account).
    mic_blocks: BTreeMap<String, Block>,
    /// Per mic: sessions that ended unexpectedly in a row.
    retries: BTreeMap<String, Retry>,
    /// The rows the last decision saw (a change resets the back-off).
    last_key: Option<BlockKey>,
    last_saved: Instant,
    save_failures: u64,
    /// The mics whose state the hub shows (one no longer shown is sent off).
    shown: Vec<String>,
}

/// A mic's shown state: its running session's (with today's cap), else the
/// block, else off with the decision's reason (`waiting` = a retry's pause).
fn shown_session(
    mic: &str,
    live: Option<SessionView>,
    blocked: Option<&(OffReason, Option<String>)>,
    off: Option<OffReason>,
    waiting: Option<TranscriptionState>,
    cap: u64,
) -> SessionView {
    match (live, blocked) {
        (Some(live), _) => SessionView {
            cap_ms: cap,
            ..live
        },
        (None, Some((reason, error))) => SessionView {
            mic_id: mic.to_string(),
            state: TranscriptionState::Stopped,
            reason: Some(*reason),
            error: error.clone(),
            cap_ms: cap,
        },
        (None, None) => SessionView {
            mic_id: mic.to_string(),
            state: waiting.unwrap_or(TranscriptionState::Off),
            reason: off,
            error: None,
            cap_ms: cap,
        },
    }
}

/// Run the supervisor for the life of the process (never in validate mode;
/// the service it holds owns the watches it waits on).
pub(crate) async fn supervise(listen: ListenService, deps: TranscriptionDeps) {
    listen.transcription().with_view(|view| {
        view.key_configured = deps.key;
        view.model = deps.model.clone();
    });
    let mut rows = listen.transcription().rows.subscribe();
    let mut mics = listen.config_receiver();
    let mut tick = interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut sup = Supervisor {
        listen,
        deps,
        running: BTreeMap::new(),
        blocked: None,
        mic_blocks: BTreeMap::new(),
        retries: BTreeMap::new(),
        last_key: None,
        last_saved: Instant::now(),
        save_failures: 0,
        shown: Vec::new(),
    };
    loop {
        let wake = tokio::select! {
            biased;
            (mic, joined) = sup.join_running() => Wake::Finished(mic, joined),
            // The senders live in the service the supervisor holds: never closed.
            _ = rows.changed() => Wake::Check,
            _ = mics.changed() => Wake::Check,
            _ = tick.tick() => Wake::Check,
        };
        if let Wake::Finished(mic, joined) = wake {
            sup.ended_by_itself(&mic, joined).await;
        }
        sup.reconcile().await;
    }
}

impl Supervisor {
    /// The first running session's task that ended; pending while none runs.
    async fn join_running(&mut self) -> (String, Joined) {
        if self.running.is_empty() {
            return std::future::pending().await;
        }
        let ends = self.running.iter_mut().map(|(mic, running)| {
            Box::pin(async move { (mic.clone(), (&mut running.task.handle).await) })
        });
        select_all(ends).await.0
    }

    /// A running session's task ended without being asked to.
    async fn ended_by_itself(&mut self, mic: &str, joined: Joined) {
        let Some(running) = self.running.remove(mic) else {
            return;
        };
        let ended = match joined {
            Ok(Some(reason)) => Ended::Refused(reason),
            Ok(None) => Ended::Unexpected,
            Err(err) => {
                tracing::warn!(target: ASR_TARGET, mic, error = %err, "the transcription task failed");
                Ended::Unexpected
            }
        };
        // No line of its own: a refusal and an unexpected end WARN in
        // `after_session` (the latter on power-of-two ends only).
        self.after_session(&running, ended).await;
    }

    async fn reconcile(&mut self) {
        let transcription = self.listen.transcription();
        let today = self.listen.today();
        let rows = transcription.rows();
        let published = self.listen.mics();
        let decision = transcription.with_view(|view| {
            view.meter.roll(today);
            start_decision(&StartInputs {
                setting: &rows.setting,
                published: &published,
                key: self.deps.key,
                today,
                over: rows.over.as_ref(),
                connected_ms_today: view.meter.connected_total(),
            })
        });
        let off = decision.as_ref().err().copied();
        let want = decision.unwrap_or_default();
        // A mic not transcribed now keeps no language: pulpit must not rank
        // a handheld by a speaker of an earlier service or day (#852 T2).
        self.listen.keep_spoken_languages(&want);
        let key = BlockKey::of(&rows, today);
        self.follow_rows(&key);
        // A block or a back-off only exists while no session of that mic runs.
        let unwanted: Vec<String> = self
            .running
            .keys()
            .filter(|mic| self.blocked.is_some() || !want.contains(mic))
            .cloned()
            .collect();
        if !unwanted.is_empty() {
            let why = match (off, self.blocked.is_some()) {
                (Some(_), _) => "the decision is off",
                (None, true) => "blocked",
                (None, false) => "the mic is no longer transcribed",
            };
            self.stop(&unwanted, why, off).await;
            // That stop may have come back refused: its block names the rows
            // the session ran under, so rows changed since lift it at once.
            self.follow_rows(&key);
        }
        if self.blocked.is_none() {
            let now = Instant::now();
            let starting: Vec<&String> = want
                .iter()
                .filter(|mic| {
                    !self.running.contains_key(*mic)
                        && !self.mic_blocks.contains_key(*mic)
                        && !self.retries.get(*mic).is_some_and(|r| r.backing_off(now))
                })
                .collect();
            if !starting.is_empty() {
                // One read for every mic that starts now (a decision's answer
                // waits for it, `decided_on`).
                let inputs = self.context_inputs(today).await;
                for mic in starting {
                    self.start(mic.clone(), &inputs, &published, key.clone());
                }
            }
        }
        self.show(&rows, off, &published, today);
        self.save_usage(false).await;
        let _previous = self
            .listen
            .transcription()
            .decided
            .send_replace(rows.generation);
    }

    /// The rows changed (the day, the switch, the guest mic or a new choice):
    /// a provider block is lifted and the back-off starts over.
    fn follow_rows(&mut self, key: &BlockKey) {
        if self.blocked.as_ref().is_some_and(|block| block.key != *key) {
            tracing::info!(target: ASR_TARGET, "the setting changed, the stopped transcription may run again");
            self.blocked = None;
        }
        self.mic_blocks.retain(|_, block| block.key == *key);
        if self.last_key.as_ref() != Some(key) {
            self.retries.clear();
            self.last_key = Some(key.clone());
        }
    }

    /// Today's references and tips for the context (fixed terms only when
    /// they cannot be read).
    async fn context_inputs(&self, today: NaiveDate) -> ContextInputs {
        match self.deps.store.context_inputs(today).await {
            Ok(inputs) => inputs,
            Err(err) => {
                tracing::warn!(target: ASR_TARGET, error = %err, "today's references and tips cannot be read — the context carries the fixed terms only");
                ContextInputs::default()
            }
        }
    }

    fn start(
        &mut self,
        mic: String,
        inputs: &ContextInputs,
        published: &[MicConfig],
        key: BlockKey,
    ) {
        let context = context_terms(inputs);
        let name = published
            .iter()
            .find(|m| m.id == mic)
            .map_or("", |m| m.name.as_str());
        // A retry after unexpected ends logs with the same power-of-two gate.
        let retry = self.retries.get(&mic).map_or(0, |r| r.ends);
        if retry == 0 || should_log_event(u64::from(retry)) {
            tracing::info!(target: ASR_TARGET, mic = mic.as_str(), name, terms = context.terms.len(), refs = inputs.prepared_refs.len(), tips = inputs.tip_terms.len(), retry, "transcription started");
        }
        let frames = self.listen.subscribe_frames(&mic);
        let session = self.deps.asr.open(AsrSessionConfig {
            mic_id: mic.clone(),
            context,
        });
        self.listen.transcription().with_view(|view| {
            let armed = SessionView {
                mic_id: mic.clone(),
                state: TranscriptionState::Armed,
                reason: None,
                error: None,
                cap_ms: 0,
            };
            match view.session_mut(&mic) {
                Some(shown) => *shown = armed,
                None => view.sessions.push(armed),
            }
        });
        let task = mic::spawn(mic.clone(), frames, session, self.listen.clone());
        let running = Running {
            task,
            key,
            started: Instant::now(),
        };
        let _previous = self.running.insert(mic, running);
    }

    /// End these mics' sessions together: their last words go out, then they
    /// are gone (each gets [`STOP_WAIT`], all at once).
    async fn stop(&mut self, mics: &[String], why: &str, reason: Option<OffReason>) {
        let mut stopping: Vec<Running> = mics
            .iter()
            .filter_map(|mic| self.running.remove(mic))
            .collect();
        for running in &mut stopping {
            running.task.stop();
        }
        let ends = join_all(stopping.iter_mut().map(|running| async move {
            let mic = running.task.mic_id.clone();
            match tokio::time::timeout(STOP_WAIT, &mut running.task.handle).await {
                Ok(Ok(Some(reason))) => Ended::Refused(reason),
                Ok(Ok(None)) => Ended::Stopped,
                Ok(Err(err)) => {
                    tracing::warn!(target: ASR_TARGET, mic = mic.as_str(), error = %err, "the transcription task failed while stopping");
                    Ended::Stopped
                }
                Err(_) => {
                    running.task.handle.abort();
                    tracing::warn!(target: ASR_TARGET, mic = mic.as_str(), "the transcription did not end within 5 s — ended");
                    Ended::Stopped
                }
            }
        }))
        .await;
        for (running, ended) in stopping.iter().zip(ends) {
            tracing::info!(target: ASR_TARGET, mic = running.task.mic_id.as_str(), why, reason = ?reason, "transcription stopped");
            self.after_session(running, ended).await;
        }
    }

    /// A session ended: a provider refusal blocks the rows it ran under, an
    /// unexpected end backs off (1, 2, 4 … 60 s), the usage is written.
    async fn after_session(&mut self, running: &Running, ended: Ended) {
        let mic = running.task.mic_id.as_str();
        if ended == Ended::Stopped || running.started.elapsed() >= STABLE_SESSION {
            let _reset = self.retries.remove(mic);
        }
        match ended {
            Ended::Refused(reason) => {
                let error = self.listen.transcription().with_view(|view| {
                    view.session_mut(mic)
                        .and_then(|session| session.error.clone())
                });
                let block = Block {
                    reason,
                    error,
                    key: running.key.clone(),
                };
                if reason == OffReason::TooManyConnections {
                    tracing::warn!(target: ASR_TARGET, mic, reason = ?reason, "this mic's transcription stopped until the setting or today's choice changes");
                    let _previous = self.mic_blocks.insert(mic.to_string(), block);
                } else {
                    tracing::warn!(target: ASR_TARGET, mic, reason = ?reason, "transcription of every mic stopped until the setting or today's choice changes");
                    self.blocked = Some(block);
                }
            }
            Ended::Unexpected => {
                let retry = self.retries.entry(mic.to_string()).or_default();
                retry.ends = retry.ends.saturating_add(1);
                let wait = bind_retry_delay(retry.ends);
                retry.at = Some(Instant::now() + wait);
                if should_log_event(u64::from(retry.ends)) {
                    tracing::warn!(target: ASR_TARGET, mic, ends = retry.ends, retry_in_s = wait.as_secs(), "the transcription ended unexpectedly — retried after a pause");
                }
            }
            Ended::Stopped => {}
        }
        self.save_usage(true).await;
    }

    /// Set every shown mic's state for the status and send it to
    /// `/listen/ws`: the transcribed mics, or — with none — the chosen guest
    /// mic with the reason; a mic no longer shown is sent once as `off`.
    fn show(
        &mut self,
        rows: &Rows,
        off: Option<OffReason>,
        published: &[MicConfig],
        today: NaiveDate,
    ) {
        let transcription = self.listen.transcription();
        let mut shown = transcribed_mics(&rows.setting, published);
        if shown.is_empty() {
            shown.extend(rows.setting.guest_mic.clone());
        }
        let hub = self.listen.transcripts();
        for old in self.shown.iter().filter(|old| !shown.contains(old)) {
            // off, no reason: a handheld no longer published is no fault
            hub.set_session(MicTranscription {
                mic_id: old.clone(),
                ..MicTranscription::default()
            });
        }
        let cap = cap_ms(&rows.setting, rows.over.as_ref(), today);
        let blocked = self.blocked.as_ref().map(|b| (b.reason, b.error.clone()));
        let mic_blocks: BTreeMap<&String, (OffReason, Option<String>)> = self
            .mic_blocks
            .iter()
            .map(|(mic, b)| (mic, (b.reason, b.error.clone())))
            .collect();
        let now = Instant::now();
        transcription.with_view(|view| {
            let mut previous = std::mem::take(&mut view.sessions);
            view.sessions = shown
                .iter()
                .map(|mic| {
                    let live = previous
                        .iter()
                        .position(|s| &s.mic_id == mic && self.running.contains_key(mic))
                        .map(|at| previous.swap_remove(at));
                    let waiting = (off.is_none()
                        && self.retries.get(mic).is_some_and(|r| r.backing_off(now)))
                    .then_some(TranscriptionState::Reconnecting);
                    let block = blocked.as_ref().or_else(|| mic_blocks.get(mic));
                    shown_session(mic, live, block, off, waiting, cap)
                })
                .collect();
        });
        for mic in &shown {
            transcription.publish_mic(&hub, mic);
        }
        self.shown = shown;
    }

    /// Write `listen.usage` when it changed: every 60 s, or now (`force`).
    async fn save_usage(&mut self, force: bool) {
        if !force && self.last_saved.elapsed() < USAGE_SAVE_EVERY {
            return;
        }
        self.last_saved = Instant::now();
        let transcription = self.listen.transcription();
        let Some(row) = transcription.with_view(|view| view.meter.take_dirty_row()) else {
            return;
        };
        match self.deps.store.save_usage(row).await {
            Ok(()) => self.save_failures = 0,
            Err(err) => {
                transcription.with_view(|view| view.meter.mark_dirty());
                self.save_failures += 1;
                if should_log_event(self.save_failures) {
                    tracing::warn!(target: ASR_TARGET, error = %err, failures = self.save_failures, "listen.usage cannot be written — retried in 60 s");
                }
            }
        }
    }
}
