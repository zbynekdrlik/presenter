//! The recorder (#852 C1b, spec §5.5, §12): with the per-day toggle on, every
//! configured mic's 16 kHz frames — exactly what the ASR gets — go to
//! `<root>/<YYYY-MM-DD>/<mic id>_<HHMMSS>.wav` (`recording_file`). A writer per
//! mic reads the mic's frame channel and never slows its chain (12.8 s behind,
//! the oldest frames go and are counted); each batch (≤ 5 s) is written in
//! `spawn_blocking`. A file opens with the mic's first frame and is named by the
//! time it was heard; the writer ends at the first frame after midnight; a
//! failed write ends it too, and the supervisor starts it again. Day folders
//! older than 30 days go once per local date, the first pass
//! [`FIRST_PASS_DELAY`] after the start. (C1b-5 adds: a new file on a drift,
//! the midnight split, the restart back-off.)
//!
//! The folder: `PRESENTER_LISTEN_RECORDINGS_DIR`, else `recordings` next to the
//! sqlite file of `PRESENTER_DB_URL` (prod / PP `/opt/presenter/recordings`, the
//! spec's path; dev `/opt/presenter-dev/recordings`; both inside the units'
//! `ReadWritePaths`), else `./recordings`. One supervisor task, started with the
//! socket task (never in validate mode); see [`supervise`].

use std::collections::{hash_map::Entry, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime, TimeDelta};
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::recorder::RECORDING_RETENTION_DAYS;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{broadcast, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{interval_at, Instant, Interval, MissedTickBehavior};
use tracing::{info, warn};

use super::recording_file::{prune_recordings, PruneReport, RecordError, RecordingFile};
use super::{should_log_event, ListenService, MicFrame};

/// The target of every recorder log line.
pub(crate) const RECORDER_TARGET: &str = "presenter::listen::recorder";
/// Overrides the recordings folder (tests use temp dirs).
pub(crate) const RECORDINGS_DIR_ENV: &str = "PRESENTER_LISTEN_RECORDINGS_DIR";
const RECORDINGS_DIR_NAME: &str = "recordings";
/// The supervisor reconciles the writers at least this often.
const RECONCILE_EVERY: Duration = Duration::from_secs(60);
/// At most this many frames (5 s) per blocking write.
const MAX_BATCH: usize = 50;
/// One frame is 100 ms.
const FRAME_MS: i64 = 100;
/// How long a production start waits before its first retention pass. A box that
/// boots with a wrong clock (RTC lost, NTP not yet synced) can read a date more
/// than 30 days ahead, and a pass at that moment would delete every recording;
/// ten minutes gives NTP time to set the clock first. Tests start at once (the
/// `cfg(test)` default of [`RecorderEnv::first_pass_delay`] is zero).
pub(crate) const FIRST_PASS_DELAY: Duration = Duration::from_secs(10 * 60);

/// The server's local wall time (tests inject a settable one).
pub(crate) type Clock = Arc<dyn Fn() -> NaiveDateTime + Send + Sync>;

/// Where the recorder writes and what time it is.
#[derive(Clone)]
pub(crate) struct RecorderEnv {
    pub(crate) root: PathBuf,
    pub(crate) clock: Clock,
    /// How long the supervisor waits before its first retention pass.
    pub(crate) first_pass_delay: Duration,
}

impl RecorderEnv {
    /// The folder from the environment and the local clock. A unit test's
    /// default folder is a temp dir, never `./recordings` inside the crate.
    pub(crate) fn from_env() -> Self {
        Self {
            root: default_root(),
            clock: Arc::new(|| chrono::Local::now().naive_local()),
            first_pass_delay: default_first_pass_delay(),
        }
    }

    /// The local wall time now.
    pub(crate) fn now(&self) -> NaiveDateTime {
        (*self.clock)()
    }
}

#[cfg(not(test))]
fn default_root() -> PathBuf {
    recordings_root(
        std::env::var_os(RECORDINGS_DIR_ENV),
        &std::env::var("PRESENTER_DB_URL").unwrap_or_default(),
    )
}

/// A unit test that never sets its own folder (`set_recorder_env`) still never
/// writes into the crate.
#[cfg(test)]
fn default_root() -> PathBuf {
    std::env::temp_dir().join("presenter-unit-test-recordings")
}

/// Production waits [`FIRST_PASS_DELAY`] before the first retention pass.
#[cfg(not(test))]
fn default_first_pass_delay() -> Duration {
    FIRST_PASS_DELAY
}

/// A unit test that starts a service wants its retention pass at once.
#[cfg(test)]
fn default_first_pass_delay() -> Duration {
    Duration::ZERO
}

/// The recordings folder: `explicit` (`PRESENTER_LISTEN_RECORDINGS_DIR`) when
/// set, else `recordings` next to the sqlite file of `db_url`.
pub(crate) fn recordings_root(explicit: Option<OsString>, db_url: &str) -> PathBuf {
    match explicit.filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => crate::state::stream_assets::dir_next_to_db(db_url, RECORDINGS_DIR_NAME),
    }
}

/// The supervisor's tick: the first one `first_pass_delay` from now, then every
/// [`RECONCILE_EVERY`].
fn retention_tick(first_pass_delay: Duration) -> Interval {
    let mut tick = interval_at(Instant::now() + first_pass_delay, RECONCILE_EVERY);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    tick
}

/// The wall-time length of `frames` frames.
fn frames_duration(frames: usize) -> TimeDelta {
    let frames = i64::try_from(frames).unwrap_or(i64::MAX);
    TimeDelta::try_milliseconds(frames.saturating_mul(FRAME_MS)).unwrap_or(TimeDelta::MAX)
}

/// A running writer, the day it records and the way to stop it.
struct Writer {
    stop: oneshot::Sender<Ended>,
    task: JoinHandle<()>,
    day: NaiveDate,
}

impl Writer {
    /// Subscribe to the mic's frames — BEFORE it is marked waiting, so no frame
    /// after that is missed — and spawn its task.
    fn start(service: &ListenService, mic_id: &str, day: NaiveDate, root: &Path) -> Self {
        let frames = service.subscribe_frames(mic_id);
        service.with_health(|health| health.recorder.writer_started(mic_id));
        let (stop, stopped) = oneshot::channel();
        let job = WriterJob {
            mic_id: mic_id.to_string(),
            day,
            root: root.to_path_buf(),
        };
        let task = tokio::spawn(run_writer(service.clone(), job, frames, stopped));
        Self { stop, task, day }
    }

    /// Why the supervisor ends this writer now, if it does: its mic is no longer
    /// wanted (the toggle is off or the mic was removed), the toggle names
    /// another day, or the task ended by itself (a failed write, its day over)
    /// and is collected.
    fn ending(&self, wanted: bool, day: Option<NaiveDate>) -> Option<Ended> {
        if !wanted || self.task.is_finished() {
            Some(Ended::Stopped)
        } else if day.is_some_and(|day| day != self.day) {
            Some(Ended::DayOver)
        } else {
            None
        }
    }

    /// Tell the writer why to close its file, and wait until it has.
    async fn stop(self, why: Ended) {
        let _unheard = self.stop.send(why);
        if let Err(err) = self.task.await {
            warn!(target: RECORDER_TARGET, %err, "a recorder writer ended abnormally");
        }
    }
}

/// The supervisor's writers.
#[derive(Default)]
struct Writers {
    running: HashMap<String, Writer>,
}

impl Writers {
    /// One writer per configured mic while recording `day`, none otherwise; a
    /// writer of another day is ended, one that ended by itself (a failed write,
    /// its day over) is replaced.
    async fn reconcile(
        &mut self,
        service: &ListenService,
        env: &RecorderEnv,
        day: Option<NaiveDate>,
        mics: &[MicConfig],
    ) {
        let configured: Vec<&str> = mics.iter().map(|mic| mic.id.as_str()).collect();
        let wanted: &[&str] = if day.is_some() { &configured } else { &[] };
        let ended: Vec<(String, Ended)> = self
            .running
            .iter()
            .filter_map(|(id, writer)| {
                let ending = writer.ending(wanted.contains(&id.as_str()), day);
                ending.map(|why| (id.clone(), why))
            })
            .collect();
        for (id, why) in ended {
            if let Some(writer) = self.running.remove(&id) {
                writer.stop(why).await;
            }
        }
        service.with_health(|health| health.recorder.retain(&configured));
        let Some(day) = day else {
            service.with_health(|health| health.recorder.all_stopped());
            return;
        };
        for id in wanted {
            if let Entry::Vacant(slot) = self.running.entry((*id).to_string()) {
                slot.insert(Writer::start(service, id, day, &env.root));
            }
        }
    }

    async fn stop_all(self) {
        for writer in self.running.into_values() {
            writer.stop(Ended::Stopped).await;
        }
    }
}

/// The day the recorder records now: the toggle's day while it is today. A day
/// AFTER today (NTP stepped the clock back while the server ran) would start
/// the recorder by itself on that day, so it is switched off here, WARNed once,
/// and the stored row is cleared by the state layer (`recorder_cleared`) — the
/// same rule the startup read applies.
fn recording_day(
    service: &ListenService,
    toggle: &mut watch::Receiver<Option<NaiveDate>>,
    today: NaiveDate,
) -> Option<NaiveDate> {
    let named = *toggle.borrow_and_update();
    if let Some(day) = named.filter(|day| *day > today) {
        if service.switch_off_future_day(day) {
            warn!(
                target: RECORDER_TARGET,
                %day,
                %today,
                "the recorder names a day in the future — it is switched off and the stored switch cleared"
            );
        }
    }
    let current = *toggle.borrow_and_update();
    current.filter(|day| *day == today)
}

/// The supervisor: two independent gates. Retention runs once per local date,
/// the first pass [`FIRST_PASS_DELAY`] after the start; the writers follow the
/// mic list, the toggle and the date AT ONCE after the start (a same-day restart
/// records on without that delay), on every change and every minute.
pub(super) async fn supervise(service: ListenService) {
    let mut config = service.config_receiver();
    let mut toggle = service.recorder_receiver();
    let mut writers = Writers::default();
    let mut pruned_on: Option<NaiveDate> = None;
    let mut retention = retention_tick(service.recorder_env().first_pass_delay);
    let mut reconcile = interval_at(Instant::now() + RECONCILE_EVERY, RECONCILE_EVERY);
    reconcile.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        let env = service.recorder_env();
        let day = recording_day(&service, &mut toggle, env.now().date());
        let mics = Arc::clone(&config.borrow_and_update());
        writers.reconcile(&service, &env, day, &mics).await;
        let gone = tokio::select! {
            _ = reconcile.tick() => false,
            _ = retention.tick() => {
                prune_if_new_day(&service.recorder_env(), &mut pruned_on).await;
                false
            }
            changed = config.changed() => changed.is_err(),
            changed = toggle.changed() => changed.is_err(),
        };
        if gone {
            break;
        }
    }
    writers.stop_all().await;
}

/// What a writer writes.
struct WriterJob {
    mic_id: String,
    day: NaiveDate,
    root: PathBuf,
}

/// An open file and the wall time its first sample was heard.
struct OpenFile {
    file: RecordingFile,
    started: NaiveDateTime,
}

impl OpenFile {
    fn create(root: &Path, started: NaiveDateTime, mic_id: &str) -> Result<Self, RecordError> {
        let file = RecordingFile::create(root, started, mic_id)?;
        Ok(Self { file, started })
    }
}

/// Why a writer ended or a file was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    Stopped,
    DayOver,
    Failed,
}

impl Ended {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "switched off or the mic was removed",
            Self::DayOver => "the day ended",
            Self::Failed => "a write failed",
        }
    }
}

/// One mic's writer: frames in, batches written off the runtime. Ends when told
/// to stop, at its first frame after midnight, or on a failed write (the
/// supervisor starts it again).
async fn run_writer(
    service: ListenService,
    job: WriterJob,
    mut frames: broadcast::Receiver<MicFrame>,
    mut stop: oneshot::Receiver<Ended>,
) {
    let mut open: Option<OpenFile> = None;
    let ended = loop {
        let received = tokio::select! {
            biased;
            why = &mut stop => break why.unwrap_or(Ended::Stopped),
            received = frames.recv() => received,
        };
        let first = match received {
            Ok(frame) => frame,
            Err(RecvError::Lagged(missed)) => {
                note_dropped(&service, &job.mic_id, missed);
                continue;
            }
            Err(RecvError::Closed) => break Ended::Stopped,
        };
        // The batch's first frame began this long ago: itself and every frame
        // sent since — those a lag inside the batch is about to cost included.
        let behind = frames_duration(frames.len().saturating_add(1));
        let now = service.recorder_env().now();
        let start = now.checked_sub_signed(behind).unwrap_or(now);
        let batch = collect_batch(&service, &job.mic_id, first, &mut frames);
        if now.date() != job.day {
            break Ended::DayOver;
        }
        let written = write_batch(open.take(), &job, start, batch).await;
        open = written.open;
        if let Some(error) = &written.failure {
            note_failed(&service, &job, error);
            break Ended::Failed;
        }
        if let Some(file) = &open {
            note_written(&service, &job, file, written.created);
        }
    };
    if let Some(last) = open {
        close(&job, last.file, ended).await;
    }
    if ended != Ended::Failed {
        service.with_health(|health| health.recorder.writer_stopped(&job.mic_id));
    }
}

/// The frame just received and whatever else is already waiting, at most
/// [`MAX_BATCH`]. A lag ends the batch: the frames before it and the frames
/// after it are not one stretch of audio, so the later ones are the next batch.
fn collect_batch(
    service: &ListenService,
    mic_id: &str,
    first: MicFrame,
    frames: &mut broadcast::Receiver<MicFrame>,
) -> Vec<MicFrame> {
    let mut batch = vec![first];
    while batch.len() < MAX_BATCH {
        match frames.try_recv() {
            Ok(frame) => batch.push(frame),
            Err(TryRecvError::Lagged(missed)) => {
                note_dropped(service, mic_id, missed);
                break;
            }
            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
        }
    }
    batch
}

/// What a blocking write left behind: the file as it stands (none when it could
/// not even be created), whether this write created it, and the Slovak reason
/// when it failed. A failed write keeps the file, so the writer can close it.
struct Written {
    open: Option<OpenFile>,
    created: bool,
    failure: Option<String>,
}

/// Write a batch off the runtime, into the open file or into a new one whose
/// first sample was heard at `start`.
async fn write_batch(
    open: Option<OpenFile>,
    job: &WriterJob,
    start: NaiveDateTime,
    batch: Vec<MicFrame>,
) -> Written {
    let (root, mic_id) = (job.root.clone(), job.mic_id.clone());
    let joined =
        tokio::task::spawn_blocking(move || append_batch(open, &root, &mic_id, start, &batch))
            .await;
    joined.unwrap_or_else(|err| Written {
        open: None,
        created: false,
        failure: Some(format!("zápis nahrávky sa prerušil: {err}")),
    })
}

/// The blocking part of [`write_batch`].
fn append_batch(
    open: Option<OpenFile>,
    root: &Path,
    mic_id: &str,
    start: NaiveDateTime,
    batch: &[MicFrame],
) -> Written {
    let created = open.is_none();
    let mut open = match open {
        Some(open) => open,
        None => match OpenFile::create(root, start, mic_id) {
            Ok(open) => open,
            Err(err) => {
                return Written {
                    open: None,
                    created: false,
                    failure: Some(err.to_string()),
                }
            }
        },
    };
    let failure = open.file.append(batch).err().map(|err| err.to_string());
    Written {
        open: Some(open),
        created,
        failure,
    }
}

/// The status after a batch; one INFO with the millisecond start time when the
/// batch opened a file (the bake-off places the show log by it).
fn note_written(service: &ListenService, job: &WriterJob, written: &OpenFile, created: bool) {
    let file = written.file.path_text();
    let bytes = written.file.data_bytes();
    service.with_health(|health| health.recorder.wrote(&job.mic_id, &file, bytes));
    if created {
        info!(
            target: RECORDER_TARGET,
            mic = job.mic_id.as_str(),
            file = %file,
            started = %written.started.format("%H:%M:%S%.3f"),
            "recording started"
        );
    }
}

/// Frames a lagging writer lost: counted; WARN on power-of-two lag events.
fn note_dropped(service: &ListenService, mic_id: &str, missed: u64) {
    let (events, dropped_frames) =
        service.with_health(|health| health.recorder.dropped(mic_id, missed));
    if should_log_event(events) {
        warn!(
            target: RECORDER_TARGET,
            mic = mic_id,
            missed,
            dropped_frames,
            events,
            "recorder fell behind — dropped frames"
        );
    }
}

/// A failed write: the mic's state `error`; WARN on power-of-two failures in a row.
fn note_failed(service: &ListenService, job: &WriterJob, error: &str) {
    let failures = service.with_health(|health| health.recorder.failed(&job.mic_id, error));
    if should_log_event(failures) {
        warn!(
            target: RECORDER_TARGET,
            mic = job.mic_id.as_str(),
            day = %job.day,
            error,
            failures,
            "recording write failed — the writer starts again within a minute"
        );
    }
}

/// Close a file — header and `fsync` off the runtime — and log it.
async fn close(job: &WriterJob, file: RecordingFile, why: Ended) {
    let path = file.path_text();
    let finished = match tokio::task::spawn_blocking(move || file.finish()).await {
        Ok(result) => result.map_err(|err| err.to_string()),
        Err(err) => Err(err.to_string()),
    };
    match finished {
        Ok(bytes) => info!(
            target: RECORDER_TARGET,
            mic = job.mic_id.as_str(),
            file = %path,
            bytes,
            why = why.as_str(),
            "recording stopped"
        ),
        Err(error) => warn!(
            target: RECORDER_TARGET,
            mic = job.mic_id.as_str(),
            file = %path,
            error,
            "closing the recording failed — its header covers the last batch written"
        ),
    }
}

/// Retention once per local date: after the start, then after each midnight. A
/// failed pass WARNs and is tried again the next day.
async fn prune_if_new_day(env: &RecorderEnv, pruned_on: &mut Option<NaiveDate>) {
    let today = env.now().date();
    if *pruned_on == Some(today) {
        return;
    }
    *pruned_on = Some(today);
    let root = env.root.clone();
    let shown = root.display().to_string();
    match tokio::task::spawn_blocking(move || prune_recordings(&root, today)).await {
        Ok(Ok(report)) => log_prune(&report, &env.root),
        Ok(Err(err)) => warn!(
            target: RECORDER_TARGET,
            root = %shown,
            %err,
            "recordings retention failed — next try tomorrow"
        ),
        Err(err) => warn!(
            target: RECORDER_TARGET,
            root = %shown,
            %err,
            "recordings retention was interrupted — next try tomorrow"
        ),
    }
}

/// One INFO per deleted file and per folder kept, a WARN per failure (a failure
/// listed against `root` itself is a listing item that could not be read).
fn log_prune(report: &PruneReport, root: &Path) {
    for (file, bytes) in &report.deleted {
        info!(
            target: RECORDER_TARGET,
            file = %file.display(),
            bytes,
            retention_days = RECORDING_RETENTION_DAYS,
            "recording deleted (retention)"
        );
    }
    for dir in &report.kept {
        info!(
            target: RECORDER_TARGET,
            dir = %dir.display(),
            "expired recordings folder kept — it holds files the recorder did not write"
        );
    }
    for (path, error) in &report.failed {
        let what = if path == root {
            "an entry of the recordings folder could not be read"
        } else {
            "an expired recordings folder could not be read or cleaned"
        };
        warn!(
            target: RECORDER_TARGET,
            dir = %path.display(),
            error = error.as_str(),
            "{what} — next try tomorrow"
        );
    }
}

#[cfg(test)]
mod tests;
