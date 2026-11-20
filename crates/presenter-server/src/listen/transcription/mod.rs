//! The guest's transcription supervisor (#852 C3-7, plan Tasks 7.3–7.5,
//! Decisions 9–11): ONE task decides with core `start_decision` whether the
//! guest mic's session runs — on every rows / mic-list change and on a 1 s
//! tick, which also reads the stage layout (the Kázeň automation, with its
//! 60 s stop delay). A running session is ONE mic task (`mic`): the mic's
//! frames in, the provider's events out into the transcript hub and the
//! meter (`meter`). A session the provider stopped for a person to fix (key,
//! account, request, too many connections) stays stopped until the switch,
//! the guest mic or today's choice changes or the day ends — never a restart
//! loop. [`Transcription`] (owned by `ListenService`) holds the rows, the
//! meter and the guest's state for `GET /listen/status`; its std mutex is
//! never held across an `.await`.

mod meter;
mod mic;
#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDate;
use presenter_core::listen::context::{context_terms, ContextInputs};
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::transcription::{
    cap_ms, cost_usd, preach_active, start_decision, today_mode, MicTranscription, OffReason,
    SessionOverride, StartInputs, TranscriptionSetting, TranscriptionState, TranscriptionStatus,
    UsageRow, PREACH_LAYOUT,
};
use tokio::sync::{watch, RwLock};
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
    pub(crate) layout: Arc<RwLock<String>>,
    pub(crate) store: Arc<dyn TranscriptionStore>,
}

/// The guest mic's state as the status shows it.
#[derive(Debug, Clone)]
struct GuestView {
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
    guest: Option<GuestView>,
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
                guest: None,
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

    /// Today's cap of connected time is reached for the guest mic.
    pub(crate) fn cap_reached(&self, today: NaiveDate) -> bool {
        let rows = self.rows();
        let Some(guest) = rows.setting.guest_mic.as_deref() else {
            return false;
        };
        let connected = self.with_view(|view| {
            view.meter.roll(today);
            view.meter.usage(guest).connected_ms
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
                mics: guest_status(view).into_iter().collect(),
            }
        })
    }

    /// Send the guest's state to `/listen/ws` (the hub sends only a visible
    /// change).
    fn publish_guest(&self, hub: &TranscriptHub) {
        if let Some(session) = self.with_view(|view| guest_status(view)) {
            hub.set_session(session);
        }
    }

    fn with_view<R>(&self, f: impl FnOnce(&mut View) -> R) -> R {
        let mut view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut view)
    }
}

/// The guest mic's `MicTranscription`, counters from the meter.
fn guest_status(view: &View) -> Option<MicTranscription> {
    let guest = view.guest.as_ref()?;
    let usage = view.meter.usage(&guest.mic_id);
    let (connections, reconnects) = view.meter.counts(&guest.mic_id);
    let latency = view.meter.latency(&guest.mic_id);
    Some(MicTranscription {
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
    })
}

/// A mic the provider stopped for a person to fix, and what must change.
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

/// Why the supervisor woke.
enum Wake {
    /// The running session's task ended by itself (`Err` = it panicked).
    Finished(Result<Option<OffReason>, tokio::task::JoinError>),
    Check,
}

/// The running session and what it ran under.
struct Running {
    task: MicTask,
    /// The rows it started with: a provider stop blocks THOSE rows.
    key: BlockKey,
    started: Instant,
}

struct Supervisor {
    listen: ListenService,
    deps: TranscriptionDeps,
    running: Option<Running>,
    blocked: Option<Block>,
    /// Sessions that ended unexpectedly in a row, and when to try again.
    unexpected: u32,
    retry_at: Option<Instant>,
    was_preach: bool,
    left_preach_at: Option<Instant>,
    last_saved: Instant,
    save_failures: u64,
    /// The mic whose state the hub shows (a changed guest mic is shown off).
    shown: Option<String>,
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
        running: None,
        blocked: None,
        unexpected: 0,
        retry_at: None,
        was_preach: false,
        left_preach_at: None,
        last_saved: Instant::now(),
        save_failures: 0,
        shown: None,
    };
    loop {
        let wake = tokio::select! {
            biased;
            joined = sup.join_running() => Wake::Finished(joined),
            // The senders live in the service the supervisor holds: never closed.
            _ = rows.changed() => Wake::Check,
            _ = mics.changed() => Wake::Check,
            _ = tick.tick() => Wake::Check,
        };
        if let Wake::Finished(joined) = wake {
            sup.ended_by_itself(joined).await;
        }
        sup.reconcile().await;
    }
}

impl Supervisor {
    /// The running session's task result; pending while none runs.
    async fn join_running(&mut self) -> Result<Option<OffReason>, tokio::task::JoinError> {
        match self.running.as_mut() {
            Some(running) => (&mut running.task.handle).await,
            None => std::future::pending().await,
        }
    }

    /// The running session's task ended without being asked to.
    async fn ended_by_itself(&mut self, joined: Result<Option<OffReason>, tokio::task::JoinError>) {
        let Some(running) = self.running.take() else {
            return;
        };
        let mic = running.task.mic_id.as_str();
        let ended = match joined {
            Ok(Some(reason)) => Ended::Refused(reason),
            Ok(None) => Ended::Unexpected,
            Err(err) => {
                tracing::warn!(target: ASR_TARGET, mic, error = %err, "the guest transcription task failed");
                Ended::Unexpected
            }
        };
        tracing::info!(target: ASR_TARGET, mic, ended = ?ended, "guest transcription session ended by itself");
        self.after_session(&running, ended).await;
    }

    async fn reconcile(&mut self) {
        let preach = self.track_preach().await;
        let transcription = self.listen.transcription();
        let today = self.listen.today();
        let rows = transcription.rows();
        let published = self.listen.mics();
        let decision = transcription.with_view(|view| {
            view.meter.roll(today);
            let guest = rows.setting.guest_mic.as_deref().unwrap_or("");
            start_decision(&StartInputs {
                setting: &rows.setting,
                published: &published,
                key: self.deps.key,
                today,
                over: rows.over.as_ref(),
                preach,
                connected_ms_today: view.meter.usage(guest).connected_ms,
            })
        });
        let off = decision.as_ref().err().copied();
        let key = BlockKey::of(&rows, today);
        if self.blocked.as_ref().is_some_and(|block| block.key != key) {
            tracing::info!(target: ASR_TARGET, "the setting changed, the stopped guest transcription may run again");
            self.blocked = None;
        }
        let want = decision.as_ref().ok().cloned();
        let blocked = self.blocked.is_some();
        if self
            .running
            .as_ref()
            .is_some_and(|r| blocked || Some(&r.task.mic_id) != want.as_ref())
        {
            let why = off.map_or("blocked", |reason| reason.slovak());
            self.stop(why).await;
        }
        // Decided AFTER the stop: the stopped session may have been refused.
        let backing_off = self.retry_at.is_some_and(|at| Instant::now() < at);
        let start = want.filter(|_| self.blocked.is_none() && !backing_off);
        if let (Some(mic), None) = (start, self.running.as_ref()) {
            self.start(mic, today, &published, key).await;
        }
        self.show_guest(&rows, off, today);
        self.save_usage(false).await;
        let _previous = self
            .listen
            .transcription()
            .decided
            .send_replace(rows.generation);
    }

    /// Follow the stage layout: the moment it left Kázeň starts the delay.
    async fn track_preach(&mut self) -> bool {
        let layout = self.deps.layout.read().await.clone();
        let now = Instant::now();
        let preach_now = layout == PREACH_LAYOUT;
        if preach_now {
            self.left_preach_at = None;
        } else if self.was_preach {
            self.left_preach_at = Some(now);
        }
        self.was_preach = preach_now;
        preach_active(
            &layout,
            self.left_preach_at.map(Instant::into_std),
            now.into_std(),
        )
    }

    async fn start(
        &mut self,
        mic: String,
        today: NaiveDate,
        published: &[MicConfig],
        key: BlockKey,
    ) {
        let inputs = match self.deps.store.context_inputs(today).await {
            Ok(inputs) => inputs,
            Err(err) => {
                tracing::warn!(target: ASR_TARGET, error = %err, "today's references and tips cannot be read — the context carries the fixed terms only");
                ContextInputs::default()
            }
        };
        let context = context_terms(&inputs);
        let name = published
            .iter()
            .find(|m| m.id == mic)
            .map_or("", |m| m.name.as_str());
        tracing::info!(target: ASR_TARGET, mic = mic.as_str(), name, terms = context.terms.len(), refs = inputs.prepared_refs.len(), tips = inputs.tip_terms.len(), "guest transcription started");
        let frames = self.listen.subscribe_frames(&mic);
        let session = self.deps.asr.open(AsrSessionConfig {
            mic_id: mic.clone(),
            context,
        });
        self.listen.transcription().with_view(|view| {
            view.guest = Some(GuestView {
                mic_id: mic.clone(),
                state: TranscriptionState::Armed,
                reason: None,
                error: None,
                cap_ms: 0,
            });
        });
        self.running = Some(Running {
            task: mic::spawn(mic, frames, session, self.listen.clone()),
            key,
            started: Instant::now(),
        });
    }

    /// End the running session: its last words go out, then it is gone.
    async fn stop(&mut self, why: &str) {
        let Some(mut running) = self.running.take() else {
            return;
        };
        let mic = running.task.mic_id.clone();
        running.task.stop();
        let ended = match tokio::time::timeout(STOP_WAIT, &mut running.task.handle).await {
            Ok(Ok(Some(reason))) => Ended::Refused(reason),
            Ok(Ok(None)) => Ended::Stopped,
            Ok(Err(err)) => {
                tracing::warn!(target: ASR_TARGET, mic = mic.as_str(), error = %err, "the guest transcription task failed while stopping");
                Ended::Stopped
            }
            Err(_) => {
                running.task.handle.abort();
                tracing::warn!(target: ASR_TARGET, mic = mic.as_str(), "the guest transcription did not end within 5 s — ended");
                Ended::Stopped
            }
        };
        tracing::info!(target: ASR_TARGET, mic = mic.as_str(), why, "guest transcription stopped");
        self.after_session(&running, ended).await;
    }

    /// A session ended: a provider refusal blocks the rows it ran under, an
    /// unexpected end backs off (1, 2, 4 … 60 s), the usage is written.
    async fn after_session(&mut self, running: &Running, ended: Ended) {
        let mic = running.task.mic_id.as_str();
        if ended == Ended::Stopped || running.started.elapsed() >= STABLE_SESSION {
            self.unexpected = 0;
            self.retry_at = None;
        }
        match ended {
            Ended::Refused(reason) => {
                let error = self.listen.transcription().with_view(|view| {
                    view.guest
                        .as_ref()
                        .filter(|guest| guest.mic_id == mic)
                        .and_then(|guest| guest.error.clone())
                });
                tracing::warn!(target: ASR_TARGET, mic, reason = ?reason, "guest transcription stopped until the setting or today's choice changes");
                self.blocked = Some(Block {
                    reason,
                    error,
                    key: running.key.clone(),
                });
            }
            Ended::Unexpected => {
                self.unexpected = self.unexpected.saturating_add(1);
                let wait = bind_retry_delay(self.unexpected);
                self.retry_at = Some(Instant::now() + wait);
                if should_log_event(u64::from(self.unexpected)) {
                    tracing::warn!(target: ASR_TARGET, mic, ends = self.unexpected, retry_in_s = wait.as_secs(), "the guest transcription ended unexpectedly — retried after a pause");
                }
            }
            Ended::Stopped => {}
        }
        self.save_usage(true).await;
    }

    /// Set the guest's state for the status and send it to `/listen/ws`.
    fn show_guest(&mut self, rows: &Rows, off: Option<OffReason>, today: NaiveDate) {
        let transcription = self.listen.transcription();
        let guest = rows.setting.guest_mic.clone();
        let hub = self.listen.transcripts();
        if let Some(old) = self.shown.take().filter(|old| Some(old) != guest.as_ref()) {
            hub.set_session(MicTranscription {
                mic_id: old,
                reason: Some(OffReason::NoGuestMic),
                ..MicTranscription::default()
            });
        }
        let cap = cap_ms(&rows.setting, rows.over.as_ref(), today);
        let running = self.running.as_ref().map(|r| r.task.mic_id.clone());
        let blocked = self.blocked.as_ref().map(|b| (b.reason, b.error.clone()));
        transcription.with_view(|view| {
            view.guest = guest.clone().map(|mic_id| {
                let live = view
                    .guest
                    .take()
                    .filter(|g| running.as_ref() == Some(&g.mic_id) && g.mic_id == mic_id);
                match (live, &blocked) {
                    (Some(live), _) => GuestView {
                        cap_ms: cap,
                        ..live
                    },
                    (None, Some((reason, error))) => GuestView {
                        mic_id,
                        state: TranscriptionState::Stopped,
                        reason: Some(*reason),
                        error: error.clone(),
                        cap_ms: cap,
                    },
                    (None, None) => GuestView {
                        mic_id,
                        state: TranscriptionState::Off,
                        reason: off,
                        error: None,
                        cap_ms: cap,
                    },
                }
            });
        });
        transcription.publish_guest(&hub);
        self.shown = guest;
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
