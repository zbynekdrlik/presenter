use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use presenter_core::{extract_song_prefix, AbleSetSettings, AbleSetSongSnapshot};
use reqwest::Client;
use serde::Deserialize;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::{
    sync::{oneshot, Mutex, RwLock},
    task::JoinHandle,
};
use tracing::{debug, info, warn};

const SETLIST_ENDPOINT: &str = "/api/setlist";
const POLL_INTERVAL_MS: u64 = 250;
/// Backoff cap when the AbleSet host is unreachable — a down host is re-polled
/// at most this often (#735), mirroring the resolume driver's `BACKOFF_CAP` (#484).
const ABLESET_BACKOFF_CAP: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct AbleSetBridge {
    inner: Arc<AbleSetInner>,
}

#[allow(dead_code)] // Trait abstraction for test mocking
type AbleSetFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[allow(dead_code)] // Trait abstraction for test mocking
pub trait AbleSetClient: Send + Sync {
    fn apply_settings(&self, settings: AbleSetSettings) -> AbleSetFuture<'_, anyhow::Result<()>>;
    fn status_snapshot(&self) -> AbleSetFuture<'_, AbleSetStatusSnapshot>;
    fn set_follow_enabled(&self, enabled: bool) -> AbleSetFuture<'_, AbleSetStatusSnapshot>;
    fn song_snapshot(&self) -> AbleSetFuture<'_, Option<AbleSetSongSnapshot>>;
}

#[allow(dead_code)] // Trait abstraction for test mocking
pub type DynAbleSetClient = Arc<dyn AbleSetClient>;

struct AbleSetInner {
    status: RwLock<AbleSetStatusInner>,
    tracker: Mutex<Option<TrackerGuard>>,
    song_changed_tx: tokio::sync::broadcast::Sender<()>,
    /// Fires whenever the tracked SETLIST (song list/order/skip-flags)
    /// changes, independent of whether the active song changed (#655 F1/F2)
    /// — a setlist can be loaded, reordered, or edited before the service
    /// starts, well before any song becomes active. `state::mod`'s
    /// `spawn_ableset_setlist_change_refresh` subscribes to this to keep the
    /// #601 mismatch report from freezing at boot.
    setlist_changed_tx: tokio::sync::broadcast::Sender<()>,
}

struct AbleSetStatusInner {
    enabled: bool,
    host: String,
    http_port: u16,
    osc_port: u16,
    library_name: String,
    song_prefix_length: u8,
    tracking: bool,
    last_song: Option<SongState>,
    setlist_songs: Vec<SetlistCachedSong>,
    last_error: Option<String>,
    follow_enabled: bool,
}

struct SongState {
    id: String,
    name: String,
    prefix: String,
    index: Option<u32>,
    last_seen_at: DateTime<Utc>,
}

#[derive(Clone)]
struct SetlistCachedSong {
    id: String,
    name: String,
    skipped: bool,
}

struct TrackerGuard {
    shutdown: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

pub use presenter_core::AbleSetStatusSnapshot;

#[derive(Clone)]
struct AbleSetTrackerConfig {
    client: Client,
    host: String,
    http_port: u16,
    song_prefix_length: u8,
}

#[derive(Debug, Deserialize)]
struct SetlistResponse {
    #[serde(rename = "activeSongId")]
    active_song_id: Option<String>,
    #[serde(default)]
    songs: Vec<SetlistSong>,
}

#[derive(Debug, Deserialize)]
struct SetlistSong {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    meta: Option<SetlistSongMeta>,
    #[serde(rename = "internalMeta")]
    #[serde(default)]
    internal_meta: Option<SetlistSongInternalMeta>,
    #[serde(default)]
    cue: Option<SetlistCue>,
}

#[derive(Debug, Deserialize)]
struct SetlistSongMeta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    raw: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetlistSongInternalMeta {
    #[serde(default)]
    order: Option<u32>,
    #[serde(default)]
    skipped: bool,
}

#[derive(Debug, Deserialize)]
struct SetlistCue {
    #[serde(default)]
    name: Option<String>,
}

impl AbleSetBridge {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AbleSetInner {
                status: RwLock::new(AbleSetStatusInner {
                    enabled: false,
                    host: "fohabl.lan".to_string(),
                    http_port: 80,
                    osc_port: 39051,
                    library_name: "NEW LEVEL".to_string(),
                    song_prefix_length: 3,
                    tracking: false,
                    last_song: None,
                    setlist_songs: Vec::new(),
                    last_error: None,
                    follow_enabled: false,
                }),
                tracker: Mutex::new(None),
                song_changed_tx: tokio::sync::broadcast::channel(16).0,
                setlist_changed_tx: tokio::sync::broadcast::channel(16).0,
            }),
        }
    }

    /// Returns a receiver that fires whenever the active song changes.
    pub fn subscribe_song_changes(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.inner.song_changed_tx.subscribe()
    }

    /// Returns a receiver that fires whenever the tracked SETLIST changes
    /// (#655 F1/F2) — song list membership, order, or skip flags, independent
    /// of whether the active song changed. Used to keep the #601 mismatch
    /// report from freezing at the last active-song change.
    pub fn subscribe_setlist_changes(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.inner.setlist_changed_tx.subscribe()
    }

    pub async fn apply_settings(&self, mut settings: AbleSetSettings) -> anyhow::Result<()> {
        settings.host = settings.host.trim().to_string();
        settings.library_name = settings.library_name.trim().to_string();
        {
            let mut status = self.inner.status.write().await;
            status.enabled = settings.enabled;
            status.host = settings.host.clone();
            status.http_port = settings.http_port;
            status.osc_port = settings.osc_port;
            status.library_name = settings.library_name.clone();
            status.song_prefix_length = settings.song_prefix_length;
            status.last_error = None;
            if !settings.enabled {
                status.tracking = false;
                status.last_song = None;
                status.follow_enabled = false;
            }
        }

        self.stop_tracker().await;

        if !settings.enabled {
            return Ok(());
        }

        match self.start_tracker(settings.clone()).await {
            Ok(()) => Ok(()),
            Err(err) => {
                let mut status = self.inner.status.write().await;
                status.tracking = false;
                status.last_error = Some(err.to_string());
                Err(err)
            }
        }
    }

    pub async fn song_snapshot(&self) -> Option<AbleSetSongSnapshot> {
        let status = self.inner.status.read().await;
        status.last_song.as_ref().map(|song| {
            AbleSetSongSnapshot::new(
                song.name.clone(),
                song.prefix.clone(),
                song.index,
                Some(song.last_seen_at),
            )
        })
    }

    /// Full tracked AbleSet setlist as `(prefix, name)` pairs, valid-prefix
    /// songs only (#601). Unlike `song_snapshot` (the one song currently
    /// ACTIVE), this returns every song AbleSet is tracking right now — the
    /// basis for the number<->title mismatch report against the Presenter
    /// library. Skipped songs are excluded: a skipped song will not play
    /// tonight, so a stale/missing title for it is not actionable.
    pub async fn setlist_song_titles(&self) -> Vec<(String, String)> {
        let status = self.inner.status.read().await;
        status
            .setlist_songs
            .iter()
            .filter(|song| !song.skipped)
            .filter_map(|song| {
                extract_song_prefix(&song.name, status.song_prefix_length)
                    .map(|prefix| (prefix, song.name.clone()))
            })
            .collect()
    }

    pub async fn next_song_name(&self) -> Option<String> {
        let status = self.inner.status.read().await;
        let last_song = status.last_song.as_ref()?;
        let active_idx = status
            .setlist_songs
            .iter()
            .position(|s| s.id == last_song.id)?;
        // Find the next entry in the active setlist (skip songs not in tonight's set)
        status.setlist_songs[active_idx + 1..]
            .iter()
            .find(|s| !s.skipped)
            .map(|s| s.name.clone())
    }

    pub async fn status_snapshot(&self) -> AbleSetStatusSnapshot {
        let status = self.inner.status.read().await;
        AbleSetStatusSnapshot {
            enabled: status.enabled,
            tracking: status.tracking,
            follow_enabled: status.follow_enabled,
            host: status.host.clone(),
            http_port: status.http_port,
            osc_port: status.osc_port,
            library_name: status.library_name.clone(),
            song_prefix_length: status.song_prefix_length,
            last_song: status.last_song.as_ref().map(|song| {
                AbleSetSongSnapshot::new(
                    song.name.clone(),
                    song.prefix.clone(),
                    song.index,
                    Some(song.last_seen_at),
                )
            }),
            last_error: status.last_error.clone(),
            // Cache enrichment is the router-level status handler's job (#600).
            // The bridge does not own the library cache, so it leaves these
            // fields at their None/empty defaults.
            cache_size: None,
            cache_last_updated: None,
            cache_last_error: None,
            recent_attempts: Vec::new(),
            mismatches: Vec::new(),
            mismatch_count: 0,
        }
    }

    pub async fn set_follow_enabled(&self, enabled: bool) -> AbleSetStatusSnapshot {
        {
            let mut status = self.inner.status.write().await;
            status.follow_enabled = enabled;
        }
        self.status_snapshot().await
    }

    async fn start_tracker(&self, settings: AbleSetSettings) -> anyhow::Result<()> {
        let client = Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .context("failed to build AbleSet client")?;
        let config = AbleSetTrackerConfig {
            client,
            host: settings.host.trim().to_string(),
            http_port: settings.http_port,
            song_prefix_length: settings.song_prefix_length,
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let inner = self.inner.clone();
        let handle = tokio::spawn(run_tracker(inner.clone(), config, shutdown_rx));
        {
            let mut guard = self.inner.tracker.lock().await;
            *guard = Some(TrackerGuard {
                shutdown: shutdown_tx,
                handle,
            });
        }
        let mut status = self.inner.status.write().await;
        status.tracking = true;
        status.last_error = None;
        Ok(())
    }

    async fn stop_tracker(&self) {
        let mut guard = self.inner.tracker.lock().await;
        if let Some(tracker) = guard.take() {
            let _ = tracker.shutdown.send(());
            if let Err(err) = tracker.handle.await {
                debug!(?err, "ableset tracker join error");
            }
        }
        let mut status = self.inner.status.write().await;
        status.tracking = false;
    }
}

impl AbleSetClient for AbleSetBridge {
    fn apply_settings(&self, settings: AbleSetSettings) -> AbleSetFuture<'_, anyhow::Result<()>> {
        let bridge = self.clone();
        Box::pin(async move { AbleSetBridge::apply_settings(&bridge, settings).await })
    }

    fn status_snapshot(&self) -> AbleSetFuture<'_, AbleSetStatusSnapshot> {
        let bridge = self.clone();
        Box::pin(async move { AbleSetBridge::status_snapshot(&bridge).await })
    }

    fn set_follow_enabled(&self, enabled: bool) -> AbleSetFuture<'_, AbleSetStatusSnapshot> {
        let bridge = self.clone();
        Box::pin(async move { AbleSetBridge::set_follow_enabled(&bridge, enabled).await })
    }

    fn song_snapshot(&self) -> AbleSetFuture<'_, Option<AbleSetSongSnapshot>> {
        let bridge = self.clone();
        Box::pin(async move { AbleSetBridge::song_snapshot(&bridge).await })
    }
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Clone, Default)]
pub struct MockAbleSetClient {
    inner: Arc<Mutex<MockAbleSetState>>,
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Default, Clone)]
struct MockAbleSetState {
    settings: Option<AbleSetSettings>,
    follow_enabled: bool,
    last_song: Option<AbleSetSongSnapshot>,
}

#[cfg(test)]
#[allow(dead_code)]
fn mock_status_from_state(state: &MockAbleSetState) -> AbleSetStatusSnapshot {
    if let Some(settings) = &state.settings {
        AbleSetStatusSnapshot {
            enabled: settings.enabled,
            tracking: settings.enabled,
            follow_enabled: state.follow_enabled,
            host: settings.host.clone(),
            http_port: settings.http_port,
            osc_port: settings.osc_port,
            library_name: settings.library_name.clone(),
            song_prefix_length: settings.song_prefix_length,
            last_song: state.last_song.clone(),
            last_error: None,
            cache_size: None,
            cache_last_updated: None,
            cache_last_error: None,
            recent_attempts: Vec::new(),
            mismatches: Vec::new(),
            mismatch_count: 0,
        }
    } else {
        AbleSetStatusSnapshot {
            enabled: false,
            tracking: false,
            follow_enabled: state.follow_enabled,
            host: "mock.local".into(),
            http_port: 80,
            osc_port: 39051,
            library_name: "Mock".into(),
            song_prefix_length: 3,
            last_song: state.last_song.clone(),
            last_error: None,
            cache_size: None,
            cache_last_updated: None,
            cache_last_error: None,
            recent_attempts: Vec::new(),
            mismatches: Vec::new(),
            mismatch_count: 0,
        }
    }
}

#[cfg(test)]
impl AbleSetClient for MockAbleSetClient {
    fn apply_settings(&self, settings: AbleSetSettings) -> AbleSetFuture<'_, anyhow::Result<()>> {
        let state = self.inner.clone();
        Box::pin(async move {
            let mut guard = state.lock().await;
            guard.follow_enabled = settings.enabled && guard.follow_enabled;
            guard.settings = Some(settings);
            Ok(())
        })
    }

    fn status_snapshot(&self) -> AbleSetFuture<'_, AbleSetStatusSnapshot> {
        let state = self.inner.clone();
        Box::pin(async move {
            let guard = state.lock().await;
            mock_status_from_state(&guard)
        })
    }

    fn set_follow_enabled(&self, enabled: bool) -> AbleSetFuture<'_, AbleSetStatusSnapshot> {
        let state = self.inner.clone();
        Box::pin(async move {
            let mut guard = state.lock().await;
            guard.follow_enabled = enabled;
            mock_status_from_state(&guard)
        })
    }

    fn song_snapshot(&self) -> AbleSetFuture<'_, Option<AbleSetSongSnapshot>> {
        let state = self.inner.clone();
        Box::pin(async move { state.lock().await.last_song.clone() })
    }
}

/// Cheap per-tick fingerprint of the raw AbleSet setlist (#655 F1) — hashes
/// id/name/skipped-flag for every song WITHOUT cloning any strings, so it can
/// run on every 250ms poll tick regardless of whether anything changed. Only
/// a fingerprint CHANGE triggers the allocation-heavy `setlist_songs` rebuild
/// below (preserves the original allocation guard); comparing this instead of
/// only the active-song id is what lets a setlist loaded/reordered/edited
/// BEFORE the service starts (no active song yet) still be detected.
fn setlist_fingerprint(songs: &[SetlistSong]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for song in songs {
        song.id.hash(&mut hasher);
        let name: Option<&str> = song
            .meta
            .as_ref()
            .and_then(|m| m.name.as_deref().or(m.raw.as_deref()))
            .or_else(|| song.cue.as_ref().and_then(|c| c.name.as_deref()));
        name.hash(&mut hasher);
        let skipped = song.internal_meta.as_ref().is_some_and(|m| m.skipped);
        skipped.hash(&mut hasher);
    }
    hasher.finish()
}

/// #655 F1: the setlist LIST can change (loaded, reordered, songs
/// skipped/unskipped) independent of the active song -- a setlist loaded
/// before the service starts never has an active song at all, so gating the
/// rebuild on the active-song id alone left it permanently empty
/// pre-service. Fingerprint-compares the current tick's setlist against the
/// last tick's and rebuilds the cached `status.setlist_songs` only when it
/// actually changed, to avoid unnecessary allocations every 250ms.
///
/// Extracted from `run_tracker` (#655 F17) to keep it under the
/// function-length cap. Pure motion: the caller still updates
/// `prev_setlist_fingerprint` and fires `setlist_changed_tx` itself, AFTER
/// releasing the status write lock, exactly as before this extraction —
/// this fn only returns `(list_changed, new_fingerprint)` so the caller can
/// do that unchanged.
fn refresh_setlist_songs(
    status: &mut AbleSetStatusInner,
    setlist: &SetlistResponse,
    prev_setlist_fingerprint: Option<u64>,
) -> (bool, u64) {
    let new_fingerprint = setlist_fingerprint(&setlist.songs);
    let list_changed = prev_setlist_fingerprint != Some(new_fingerprint);

    if list_changed {
        // Rebuild cached song list only when the list actually changed to
        // avoid unnecessary allocations every 250ms.
        status.setlist_songs = setlist
            .songs
            .iter()
            .map(|s| {
                let name = s
                    .meta
                    .as_ref()
                    .and_then(|m| m.name.as_ref().cloned().or_else(|| m.raw.clone()))
                    .or_else(|| s.cue.as_ref().and_then(|c| c.name.clone()))
                    .unwrap_or_default();
                let skipped = s.internal_meta.as_ref().map_or(false, |m| m.skipped);
                SetlistCachedSong {
                    id: s.id.clone().unwrap_or_default(),
                    name,
                    skipped,
                }
            })
            .collect();
    }

    (list_changed, new_fingerprint)
}

/// Minimum spacing before the next AbleSet poll given `consecutive_failures`
/// consecutive failed fetches (1-based). `0` → the normal [`POLL_INTERVAL_MS`]
/// cadence; otherwise exponential (0.5 s, 1 s, 2 s, …) capped at
/// [`ABLESET_BACKOFF_CAP`], so a down host stops being hammered every 250 ms
/// (#735). Mirrors the resolume driver's `backoff_interval` (#484). Pure +
/// deterministic → unit-tested without sleeping.
fn ableset_backoff_interval(consecutive_failures: u32) -> Duration {
    if consecutive_failures == 0 {
        return Duration::from_millis(POLL_INTERVAL_MS);
    }
    // 500 ms * 2^(n-1), saturating, then capped. Shift clamped well under 64.
    let shift = consecutive_failures.saturating_sub(1).min(32);
    let ms = 500u64.saturating_mul(1u64 << shift);
    Duration::from_millis(ms).min(ABLESET_BACKOFF_CAP)
}

/// Whether to emit the WARN log line for the AbleSet poll failure with this
/// 1-based `consecutive_failures` count. Logs the first failure and then only at
/// power-of-two milestones, so a host down for N ticks produces ~log2(N)+1 WARN
/// lines instead of N — the resolume `should_log_error` idiom (#484), fixing the
/// 244,039-line flood this ticket addresses (#735).
fn should_log_ableset_failure(consecutive_failures: u32) -> bool {
    consecutive_failures > 0 && consecutive_failures.is_power_of_two()
}

/// #735: log an "AbleSet reachable again" recovery line on the first reachable
/// response after a failure streak. The caller resets `consecutive_failures` to
/// 0 afterwards (resetting when already 0 is a no-op). Extracted from
/// `run_tracker` to keep it under the fn-length cap.
fn log_ableset_recovery(host: &str, consecutive_failures: u32) {
    if consecutive_failures > 0 {
        info!(
            host = %host,
            consecutive_failures,
            "AbleSet reachable again — resuming normal poll cadence (#735)"
        );
    }
}

/// Update `status.last_song` / `status.last_error` from the setlist's active
/// song. Extracted verbatim from `run_tracker`'s `Ok(Some(..))` arm to keep that
/// function under the fn-length cap; behavior is unchanged.
fn update_active_song_status(
    status: &mut AbleSetStatusInner,
    setlist: &SetlistResponse,
    song_prefix_length: u8,
) {
    let Some(active_id) = &setlist.active_song_id else {
        status.last_song = None;
        status.last_error = None;
        return;
    };
    let mut found = false;
    for (idx, song) in setlist.songs.iter().enumerate() {
        if song.id.as_deref() == Some(active_id.as_str()) {
            // `status.setlist_songs` is always current here: freshly rebuilt by
            // `refresh_setlist_songs` if the list changed, or left as-is from a
            // prior tick whose fingerprint proves it is still identical.
            let Some(name) = status.setlist_songs.get(idx).map(|s| s.name.clone()) else {
                continue;
            };
            if let Some(prefix) = extract_song_prefix(&name, song_prefix_length) {
                let index = song
                    .internal_meta
                    .as_ref()
                    .and_then(|m| m.order)
                    .or(Some(idx as u32));
                status.last_song = Some(SongState {
                    id: active_id.clone(),
                    name,
                    prefix,
                    index,
                    last_seen_at: Utc::now(),
                });
                status.last_error = None;
                found = true;
            } else {
                status.last_error = Some(format!(
                    "unable to extract prefix of length {} from song '{name}'",
                    song_prefix_length
                ));
            }
            break;
        }
    }
    if !found && status.last_error.is_none() {
        status.last_song = None;
    }
}

async fn run_tracker(
    inner: Arc<AbleSetInner>,
    config: AbleSetTrackerConfig,
    mut shutdown: oneshot::Receiver<()>,
) {
    let AbleSetTrackerConfig {
        client,
        host,
        http_port,
        song_prefix_length,
    } = config;
    let mut prev_active_id: Option<String> = None;
    let mut prev_setlist_fingerprint: Option<u64> = None;
    // #735: consecutive failed fetches drive an exponential backoff on the poll
    // cadence AND a power-of-two-gated WARN, so an unreachable AbleSet host is
    // neither hammered every 250 ms nor floods the journal. Reset to 0 on any
    // reachable response (Ok(Some)/Ok(None)).
    let mut consecutive_failures: u32 = 0;

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                break;
            }
            _ = tokio::time::sleep(ableset_backoff_interval(consecutive_failures)) => {
                match fetch_setlist(&client, &host, http_port).await {
                    Ok(Some(setlist)) => {
                        log_ableset_recovery(&host, consecutive_failures);
                        consecutive_failures = 0;
                        let new_active_id = setlist.active_song_id.clone();
                        let song_changed = new_active_id != prev_active_id;
                        let mut status = inner.status.write().await;
                        // #655 F1/F17: fingerprint-compares the setlist LIST itself
                        // (independent of the active song) and rebuilds
                        // status.setlist_songs on a real change — see
                        // refresh_setlist_songs's doc comment for the full "why".
                        let (list_changed, new_fingerprint) =
                            refresh_setlist_songs(&mut status, &setlist, prev_setlist_fingerprint);
                        update_active_song_status(&mut status, &setlist, song_prefix_length);
                        drop(status);
                        if list_changed {
                            prev_setlist_fingerprint = Some(new_fingerprint);
                            let _ = inner.setlist_changed_tx.send(());
                        }
                        if song_changed {
                            prev_active_id = new_active_id;
                            let _ = inner.song_changed_tx.send(());
                        }
                    }
                    Ok(None) => {
                        log_ableset_recovery(&host, consecutive_failures);
                        consecutive_failures = 0;
                        let mut status = inner.status.write().await;
                        status.last_song = None;
                        status.setlist_songs.clear();
                        status.last_error = None;
                        drop(status);
                        if prev_setlist_fingerprint.take().is_some() {
                            let _ = inner.setlist_changed_tx.send(());
                        }
                    }
                    Err(err) => {
                        // #735: exponential backoff (via `consecutive_failures`
                        // feeding `ableset_backoff_interval`) stops hammering a
                        // dead host; the WARN is power-of-two-gated so a host
                        // down for N ticks logs ~log2(N)+1 lines, not N.
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        let mut status = inner.status.write().await;
                        status.last_error = Some(err.to_string());
                        drop(status);
                        if should_log_ableset_failure(consecutive_failures) {
                            warn!(
                                ?err,
                                host = %host,
                                consecutive_failures,
                                "AbleSet setlist poll failing — backing off (#735)"
                            );
                        } else {
                            debug!(?err, "ableset fetch failed");
                        }
                    }
                }
            }
        }
    }

    let mut status = inner.status.write().await;
    status.tracking = false;
}

async fn fetch_setlist(
    client: &Client,
    host: &str,
    http_port: u16,
) -> anyhow::Result<Option<SetlistResponse>> {
    let url = format!("http://{host}:{http_port}{SETLIST_ENDPOINT}");
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to query AbleSet at {url}"))?;

    if response.status().is_success() {
        let payload: SetlistResponse = response
            .json()
            .await
            .context("failed to parse AbleSet setlist payload")?;
        return Ok(Some(payload));
    }

    if response.status().as_u16() == 404 {
        return Ok(None);
    }

    Err(anyhow!(
        "AbleSet responded with status {}",
        response.status()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ableset_backoff_grows_exponentially_and_caps() {
        // #735: 0 failures = the normal poll cadence; each subsequent failure
        // doubles from 500 ms, capped at 60 s — so an unreachable AbleSet host
        // is not re-requested every 250 ms. Without the fix there is no backoff
        // (the loop polled at a fixed 250 ms regardless of failures).
        assert_eq!(
            ableset_backoff_interval(0),
            Duration::from_millis(POLL_INTERVAL_MS)
        );
        assert_eq!(ableset_backoff_interval(1), Duration::from_millis(500));
        assert_eq!(ableset_backoff_interval(2), Duration::from_secs(1));
        assert_eq!(ableset_backoff_interval(3), Duration::from_secs(2));
        assert_eq!(ableset_backoff_interval(7), Duration::from_secs(32));
        // 500 ms * 2^7 = 64 s → capped at the 60 s BACKOFF_CAP.
        assert_eq!(ableset_backoff_interval(8), ABLESET_BACKOFF_CAP);
        assert_eq!(ableset_backoff_interval(100), ABLESET_BACKOFF_CAP);
    }

    #[test]
    fn ableset_failure_log_is_power_of_two_gated() {
        // #735: the 244,039-line flood came from logging EVERY failed tick.
        // Now only the 1st failure + power-of-two milestones emit a WARN, so a
        // host down for N ticks produces ~log2(N)+1 lines instead of N.
        assert!(
            !should_log_ableset_failure(0),
            "a never-failed poll must not log"
        );
        assert!(should_log_ableset_failure(1));
        assert!(should_log_ableset_failure(2));
        assert!(!should_log_ableset_failure(3));
        assert!(should_log_ableset_failure(4));
        assert!(!should_log_ableset_failure(5));
        assert!(!should_log_ableset_failure(6));
        assert!(!should_log_ableset_failure(7));
        assert!(should_log_ableset_failure(8));
        assert!(!should_log_ableset_failure(9));
    }

    #[tokio::test]
    async fn next_song_name_returns_next_non_skipped_song() {
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![
                SetlistCachedSong {
                    id: "s1".into(),
                    name: "001 First Song".into(),
                    skipped: false,
                },
                SetlistCachedSong {
                    id: "s2".into(),
                    name: "002 Second Song".into(),
                    skipped: false,
                },
                SetlistCachedSong {
                    id: "s3".into(),
                    name: "003 Third Song".into(),
                    skipped: false,
                },
            ];
            status.last_song = Some(SongState {
                id: "s1".into(),
                name: "001 First Song".into(),
                prefix: "001".into(),
                index: Some(0),
                last_seen_at: Utc::now(),
            });
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, Some("002 Second Song".to_string()));
    }

    #[tokio::test]
    async fn next_song_name_skips_songs_not_in_setlist() {
        // Simulates real AbleSet: Arriba is active, next two songs are
        // skipped (not in tonight's setlist), Ja v Teba verim is next
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![
                SetlistCachedSong {
                    id: "s1".into(),
                    name: "076 Arriba".into(),
                    skipped: false,
                },
                SetlistCachedSong {
                    id: "s2".into(),
                    name: "140 Pane zosli".into(),
                    skipped: true,
                },
                SetlistCachedSong {
                    id: "s3".into(),
                    name: "134 Tancujem".into(),
                    skipped: true,
                },
                SetlistCachedSong {
                    id: "s4".into(),
                    name: "138 Ja v Teba verim".into(),
                    skipped: false,
                },
            ];
            status.last_song = Some(SongState {
                id: "s1".into(),
                name: "076 Arriba".into(),
                prefix: "076".into(),
                index: Some(0),
                last_seen_at: Utc::now(),
            });
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, Some("138 Ja v Teba verim".to_string()));
    }

    #[tokio::test]
    async fn next_song_name_returns_none_when_last_in_setlist() {
        // Active song is the last non-skipped song — no next
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![
                SetlistCachedSong {
                    id: "s1".into(),
                    name: "001 First Song".into(),
                    skipped: false,
                },
                SetlistCachedSong {
                    id: "s2".into(),
                    name: "002 Last Song".into(),
                    skipped: false,
                },
                SetlistCachedSong {
                    id: "s3".into(),
                    name: "003 Not In Set".into(),
                    skipped: true,
                },
            ];
            status.last_song = Some(SongState {
                id: "s2".into(),
                name: "002 Last Song".into(),
                prefix: "002".into(),
                index: Some(1),
                last_seen_at: Utc::now(),
            });
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, None);
    }

    #[tokio::test]
    async fn next_song_name_returns_none_when_no_active_song() {
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![SetlistCachedSong {
                id: "s1".into(),
                name: "001 First Song".into(),
                skipped: false,
            }];
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, None);
    }

    /// #655 F1 — RED (this commit): a setlist loaded before the service
    /// starts (nothing playing yet, `activeSongId: null`) must populate
    /// `setlist_song_titles()` immediately — this is #601's primary
    /// pre-service-checklist use case. Before the fix, `run_tracker` only
    /// rebuilds `setlist_songs` inside `if song_changed`, and
    /// `prev_active_id` starts `None`; with no active song ever reported,
    /// `new_active_id (None) != prev_active_id (None)` is always `false`, so
    /// the rebuild never runs and the mismatch report stays "all gaps"
    /// indefinitely, until the first song is played (too late).
    #[tokio::test]
    async fn setlist_populates_before_any_active_song_is_played() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/setlist"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "activeSongId": null,
                    "songs": [
                        { "id": "s1", "meta": { "name": "017 Viem, ze Ty Pan" } },
                        { "id": "s2", "meta": { "name": "018 Another Song" } }
                    ]
                })),
            )
            .mount(&mock_server)
            .await;

        let bridge = AbleSetBridge::new();
        let addr = mock_server.address();
        let settings = AbleSetSettings::new(
            true,
            addr.ip().to_string(),
            39051,
            addr.port(),
            "Hymnal".to_string(),
            3,
            Utc::now(),
            Utc::now(),
        );
        bridge.apply_settings(settings).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if !bridge.setlist_song_titles().await.is_empty() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "setlist_song_titles() must populate from a loaded setlist even with \
                 no active song (activeSongId: null) — #655 F1"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let titles = bridge.setlist_song_titles().await;
        assert_eq!(
            titles.len(),
            2,
            "both setlist songs must be present pre-service: {titles:?}"
        );
    }

    #[tokio::test]
    async fn setlist_song_titles_excludes_skipped_and_no_prefix_songs() {
        // #601: the mismatch report is built from this list, so it must
        // exclude songs that cannot ever be a resolvable AbleSet number.
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.song_prefix_length = 3;
            status.setlist_songs = vec![
                SetlistCachedSong {
                    id: "s1".into(),
                    name: "017 Viem, ze Ty Pan".into(),
                    skipped: false,
                },
                SetlistCachedSong {
                    id: "s2".into(),
                    name: "140 Pane zosli".into(),
                    skipped: true,
                },
                SetlistCachedSong {
                    id: "s3".into(),
                    name: "No Number Intro".into(),
                    skipped: false,
                },
            ];
        }
        let titles = bridge.setlist_song_titles().await;
        assert_eq!(
            titles,
            vec![("017".to_string(), "017 Viem, ze Ty Pan".to_string())],
            "skipped songs and songs without a valid numeric prefix must be excluded"
        );
    }
}
