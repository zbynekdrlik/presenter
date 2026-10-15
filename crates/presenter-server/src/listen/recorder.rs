//! The recorder (#852 C1b, spec §5.5, §12). C1b-2: its folder and its
//! retention — the day folders older than 30 days are deleted once per local
//! date (the first pass [`FIRST_PASS_DELAY`] after the start, then in the first
//! minute after each midnight), so no recording outlives its 30 days (its day
//! folder goes on the 31st). C1b-4 adds the writers.
//!
//! The folder: `PRESENTER_LISTEN_RECORDINGS_DIR`, else `recordings` next to the
//! sqlite file of `PRESENTER_DB_URL` (prod / PP `/opt/presenter/recordings`, the
//! spec's path; dev `/opt/presenter-dev/recordings`; both inside the units'
//! `ReadWritePaths`), else `./recordings`. One supervisor task, started with the
//! socket task (never in validate mode).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime};
use presenter_core::listen::recorder::RECORDING_RETENTION_DAYS;
use tokio::time::{interval_at, Instant, Interval, MissedTickBehavior};
use tracing::{info, warn};

use super::recording_file::{prune_recordings, PruneReport};
use super::ListenService;

/// The target of every recorder log line.
pub(crate) const RECORDER_TARGET: &str = "presenter::listen::recorder";
/// Overrides the recordings folder (tests use temp dirs).
pub(crate) const RECORDINGS_DIR_ENV: &str = "PRESENTER_LISTEN_RECORDINGS_DIR";
const RECORDINGS_DIR_NAME: &str = "recordings";
/// The supervisor looks at the date this often.
const RECONCILE_EVERY: Duration = Duration::from_secs(60);
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

/// The supervisor: the retention pass once per local date, the first one
/// [`FIRST_PASS_DELAY`] after the start (see there).
pub(super) async fn supervise(service: ListenService) {
    let mut pruned_on: Option<NaiveDate> = None;
    let mut tick = retention_tick(service.recorder_env().first_pass_delay);
    loop {
        tick.tick().await;
        prune_if_new_day(&service.recorder_env(), &mut pruned_on).await;
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
