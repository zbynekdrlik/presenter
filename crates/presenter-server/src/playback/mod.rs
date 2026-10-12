//! The Drive mirror behind the stream PLAYBACK element (#843).
//!
//! Every Drive folder referenced by a `playback` element (across all outputs)
//! is mirrored into `<stream-assets>/playback/<folder_id>/` by ONE background
//! task (`state/stream_playback.rs`, normal startup only), one folder at a
//! time. A pass runs every [`SYNC_INTERVAL`], after a stream config change
//! (`StreamConfigChanged` — a new or edited element) and when the operator
//! presses „Obnoviť" (`POST /stream/api/playback/refresh`).
//!
//! - [`sync::sync_folder`] mirrors one folder (list → HEAD → download changed →
//!   manifest → prune).
//! - [`manifest`] is the on-disk playlist + its pure helpers.
//! - [`PlaybackSync`] is the shared, `Arc`-backed state: the Drive client, the
//!   "refresh" nudge, which folders are queued/syncing (the API's `syncing`),
//!   folders asked for by a refresh before any element names them, and the
//!   per-folder failure streaks of the #484 log gate.
//!
//! The mirror is NOT `stream_assets` rows: those are uploads the operator
//! manages; these files belong to the sync, which prunes them.

pub(crate) mod manifest;
pub(crate) mod sync;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use presenter_core::DriveFolder;

use self::manifest::Manifest;
use crate::drive::DriveClient;

/// The cache's subdirectory of the stream-assets directory.
pub(crate) const PLAYBACK_DIR: &str = "playback";
/// How often every referenced folder is re-listed.
pub(crate) const SYNC_INTERVAL: Duration = Duration::from_secs(300);
/// A burst of config changes / refreshes within this window runs one pass.
pub(crate) const TRIGGER_DEBOUNCE: Duration = Duration::from_secs(2);
/// A config-change pass skips a folder synced less than this long ago (an
/// element drag must not re-HEAD every file of every folder); a new folder,
/// a periodic pass and a refresh always sync.
const CONFIG_RESYNC_MIN_AGE_SECS: i64 = 60;
/// A cached folder no element references any more is deleted once its last
/// sync is this old, so an element removed for a moment (or a folder synced by
/// a refresh before its element was saved) does not lose its mirror at once.
const UNREFERENCED_GRACE_SECS: i64 = 24 * 3600;

/// Why a sync pass runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncTrigger {
    /// The [`SYNC_INTERVAL`] ticker (and the startup pass).
    Periodic,
    /// A `StreamConfigChanged` event — an element was created or edited.
    ConfigChanged,
    /// The operator's „Obnoviť".
    Forced,
}

impl SyncTrigger {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Periodic => "periodic",
            Self::ConfigChanged => "config-changed",
            Self::Forced => "refresh",
        }
    }
}

/// `true` when a pass for `trigger` should sync a folder whose current
/// manifest is `manifest`.
pub(crate) fn should_sync(
    trigger: SyncTrigger,
    manifest: Option<&Manifest>,
    now: DateTime<Utc>,
) -> bool {
    match trigger {
        SyncTrigger::Periodic | SyncTrigger::Forced => true,
        SyncTrigger::ConfigChanged => match manifest.and_then(|m| m.synced_at) {
            None => true,
            Some(synced_at) => (now - synced_at).num_seconds() >= CONFIG_RESYNC_MIN_AGE_SECS,
        },
    }
}

/// `true` when an UNREFERENCED folder's cache may be deleted: its last activity
/// (`synced_at`, else `error_at`) is older than the grace period. A folder
/// with no manifest at all is a crash leftover and goes at once.
pub(crate) fn should_prune_unreferenced(manifest: Option<&Manifest>, now: DateTime<Utc>) -> bool {
    let last_activity = manifest.and_then(|m| m.synced_at.or(m.error_at));
    match last_activity {
        None => true,
        Some(at) => (now - at).num_seconds() >= UNREFERENCED_GRACE_SECS,
    }
}

/// #484 log gate: log the 1st failure and the power-of-two milestones only.
pub(crate) fn should_log_failure(consecutive_failures: u32) -> bool {
    consecutive_failures > 0 && consecutive_failures.is_power_of_two()
}

/// First retry delay of a file that failed; doubles per consecutive failure.
const FILE_RETRY_BASE_SECS: i64 = 300;
/// Longest wait before a failing file is tried again.
const FILE_RETRY_MAX_SECS: i64 = 6 * 3600;

/// How long a file that failed `streak` syncs in a row waits before the next
/// try: 5 min, 10, 20, … capped at 6 h. ("Obnoviť" retries at once.)
pub(crate) fn retry_backoff(streak: u32) -> chrono::Duration {
    let shift = streak.saturating_sub(1).min(16);
    let secs = FILE_RETRY_BASE_SECS
        .saturating_mul(1_i64 << shift)
        .min(FILE_RETRY_MAX_SECS);
    chrono::Duration::seconds(secs)
}

/// Default byte budget of the whole mirror (all folders together), in GiB.
const DEFAULT_BUDGET_GIB: u64 = 20;
const GIB: u64 = 1024 * 1024 * 1024;

/// `PRESENTER_PLAYBACK_MAX_GB` as whole GiB (a positive integer), else 20.
pub(crate) fn budget_gib(value: Option<&str>) -> u64 {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|gib| *gib > 0)
        .unwrap_or(DEFAULT_BUDGET_GIB)
}

/// The mirror-wide byte budget: the mirror shares the disk with
/// `presenter.db`, so a huge folder (or refreshes of many folders) must stop
/// at a limit instead of filling the disk. `used` is the cached bytes of every
/// folder; a download that would pass `limit` is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Budget {
    pub(crate) limit: u64,
    pub(crate) used: u64,
}

impl Budget {
    pub(crate) fn remaining(self) -> u64 {
        self.limit.saturating_sub(self.used)
    }
}

/// Shared sync state, cheap to clone (one `Arc`).
#[derive(Clone)]
pub(crate) struct PlaybackSync {
    inner: Arc<Inner>,
}

struct Inner {
    /// Built (and logged) on the first sync, so a validate-mode boot or a unit
    /// test that never syncs never builds one.
    client: OnceLock<DriveClient>,
    /// The mirror-wide byte budget (`PRESENTER_PLAYBACK_MAX_GB`).
    budget_limit: u64,
    /// Bumped by the live-hub forwarder on every `StreamConfigChanged`; the
    /// worker waits on `changed()`, so a burst coalesces and nothing lags.
    config_changes: tokio::sync::watch::Sender<u64>,
    /// Wakes the worker for a refresh pass (a stored permit if it is busy).
    nudge: tokio::sync::Notify,
    /// Folders queued or being synced right now (the API's `syncing`).
    pending: Mutex<HashSet<String>>,
    /// Folders a refresh asked for that no element may name yet.
    extra: Mutex<HashMap<String, DriveFolder>>,
    /// Consecutive failed syncs per folder (#484 log gate).
    failures: Mutex<HashMap<String, u32>>,
    /// One pass at a time (the worker and a test can never overlap).
    pass_lock: tokio::sync::Mutex<()>,
}

impl PlaybackSync {
    pub(crate) fn new() -> Self {
        let gib = budget_gib(std::env::var("PRESENTER_PLAYBACK_MAX_GB").ok().as_deref());
        let (config_changes, _) = tokio::sync::watch::channel(0_u64);
        Self {
            inner: Arc::new(Inner {
                client: OnceLock::new(),
                budget_limit: gib.saturating_mul(GIB),
                config_changes,
                nudge: tokio::sync::Notify::new(),
                pending: Mutex::new(HashSet::new()),
                extra: Mutex::new(HashMap::new()),
                failures: Mutex::new(HashMap::new()),
                pass_lock: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// A sync over a given client (a mock Drive in tests).
    #[cfg(test)]
    pub(crate) fn with_client(client: DriveClient) -> Self {
        let sync = Self::new();
        let _ = sync.inner.client.set(client);
        sync
    }

    /// The Drive client — built from the env (and logged) on first use.
    pub(crate) fn client(&self) -> &DriveClient {
        self.inner.client.get_or_init(DriveClient::from_env)
    }

    pub(crate) fn budget_limit(&self) -> u64 {
        self.inner.budget_limit
    }

    /// A stream config changed (called by the live-hub forwarder).
    pub(crate) fn note_config_change(&self) {
        self.inner
            .config_changes
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }

    /// The worker's view of [`Self::note_config_change`].
    pub(crate) fn config_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.inner.config_changes.subscribe()
    }

    /// Ask the worker for a refresh pass.
    pub(crate) fn nudge(&self) {
        self.inner.nudge.notify_one();
    }

    /// Resolves at the next [`Self::nudge`] (at once if one is pending).
    pub(crate) async fn nudged(&self) {
        self.inner.nudge.notified().await;
    }

    /// Hold while running a pass.
    pub(crate) async fn lock_pass(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.pass_lock.lock().await
    }

    pub(crate) fn mark_pending<I: IntoIterator<Item = String>>(&self, folder_ids: I) {
        lock(&self.inner.pending).extend(folder_ids);
    }

    pub(crate) fn clear_pending(&self, folder_id: &str) {
        lock(&self.inner.pending).remove(folder_id);
    }

    /// The folders marked right now.
    pub(crate) fn pending_snapshot(&self) -> HashSet<String> {
        lock(&self.inner.pending).clone()
    }

    pub(crate) fn is_pending(&self, folder_id: &str) -> bool {
        lock(&self.inner.pending).contains(folder_id)
    }

    /// Remember a folder a refresh asked for (synced by the next pass even if
    /// no saved element names it yet).
    pub(crate) fn add_extra(&self, folder: DriveFolder) {
        lock(&self.inner.extra).insert(folder.id.clone(), folder);
    }

    pub(crate) fn take_extras(&self) -> Vec<DriveFolder> {
        lock(&self.inner.extra).drain().map(|(_, f)| f).collect()
    }

    /// Count a failed sync of `folder_id`; returns the new streak.
    pub(crate) fn record_failure(&self, folder_id: &str) -> u32 {
        let mut failures = lock(&self.inner.failures);
        let streak = failures.entry(folder_id.to_string()).or_insert(0);
        *streak = streak.saturating_add(1);
        *streak
    }

    /// A successful sync ends the streak; returns the streak that ended.
    pub(crate) fn record_success(&self, folder_id: &str) -> u32 {
        lock(&self.inner.failures)
            .remove(folder_id)
            .unwrap_or_default()
    }
}

impl Default for PlaybackSync {
    fn default() -> Self {
        Self::new()
    }
}

/// These locks are never held across an `.await`; a poisoned lock still holds
/// a consistent set/map (every mutation is a single insert/remove).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
