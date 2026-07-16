//! #555 song-sync engine: a symmetric pull loop against the peer instance, plus a
//! debounced nudge after local mutations. Reuses the AbleSet-tracker background-task
//! shape (interval + oneshot shutdown). Applied rows carry the PEER timestamp → no echo.
use std::sync::Arc;

use chrono::{DateTime, Utc};
use presenter_persistence::{SyncPresentation, TrashedPresentation};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::time::{interval, Duration, MissedTickBehavior};
use tracing::{info, warn};

use super::AppState;

const SYNC_INTERVAL: Duration = Duration::from_secs(30);
const NUDGE_DEBOUNCE: Duration = Duration::from_secs(2);

/// Wire DTOs (both directions). camelCase to match the repo convention.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncManifestEntryDto {
    pub sync_id: String,
    pub library_name: String,
    pub name: String,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPresentationDto {
    pub sync_id: String,
    pub library_name: String,
    pub name: String,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub slides: Vec<presenter_core::Slide>,
}

impl From<SyncPresentationDto> for SyncPresentation {
    fn from(d: SyncPresentationDto) -> Self {
        SyncPresentation {
            sync_id: d.sync_id,
            library_name: d.library_name,
            name: d.name,
            updated_at: d.updated_at,
            deleted_at: d.deleted_at,
            slides: d.slides,
        }
    }
}

/// Trash row for the settings UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashedPresentationDto {
    pub id: String,
    pub sync_id: String,
    pub name: String,
    pub library_name: String,
    pub deleted_at: DateTime<Utc>,
}

impl From<TrashedPresentation> for TrashedPresentationDto {
    fn from(t: TrashedPresentation) -> Self {
        Self {
            id: t.id,
            sync_id: t.sync_id,
            name: t.name,
            library_name: t.library_name,
            deleted_at: t.deleted_at,
        }
    }
}

/// Operator-facing status (AbleSet status pattern).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    pub peer_url: Option<String>,
    pub peer_version: Option<String>,
    pub peer_healthy: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub pulled_last_cycle: usize,
    pub applied_last_cycle: usize,
    /// Songs whose per-song fetch/apply failed during the last cycle
    /// (#558 round-4 U1(b)) — a single song's failure (e.g. an oversize
    /// legacy stored slide, or any other per-song error) is isolated and
    /// counted here, never allowed to abort the whole cycle.
    pub last_cycle_errors: usize,
}

/// Clonable handle stored on AppState. The receiver is taken once by the loop.
#[derive(Clone)]
pub struct SyncCoordinator {
    nudge_tx: mpsc::Sender<()>,
    nudge_rx: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
    status: Arc<RwLock<SyncStatus>>,
    /// S1: keeps the loop's shutdown oneshot Sender ALIVE for as long as the
    /// coordinator (and thus AppState) lives. The Sender used to be handed
    /// back to the caller and immediately dropped, which resolves
    /// `tokio::select! { _ = &mut shutdown_rx => break, ... }`'s shutdown
    /// branch on the very next pass — the loop died before ever completing a
    /// real cycle in production. Every clone of AppState shares this same
    /// `Arc`, so storing the sender here (rather than returning it) keeps
    /// exactly one authoritative slot regardless of how many clones exist.
    shutdown: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

impl Default for SyncCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncCoordinator {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(1);
        Self {
            nudge_tx: tx,
            nudge_rx: Arc::new(Mutex::new(Some(rx))),
            status: Arc::new(RwLock::new(SyncStatus::default())),
            shutdown: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Non-blocking; a full channel already has a nudge pending.
    pub fn nudge(&self) {
        let _ = self.nudge_tx.try_send(());
    }

    pub async fn snapshot(&self) -> SyncStatus {
        self.status.read().await.clone()
    }

    /// Check-and-claim the shutdown slot atomically (#558 R5): if a sender is
    /// ALREADY installed (a loop is already running), leave it untouched and
    /// return `false` — the caller must not spawn anything. Otherwise store
    /// `sender` and return `true`. This must happen under ONE lock
    /// acquisition so no window exists where an old sender is overwritten
    /// before the "already running" check runs.
    fn claim_shutdown_slot(&self, sender: oneshot::Sender<()>) -> bool {
        let mut guard = match self.shutdown.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.is_some() {
            return false;
        }
        *guard = Some(sender);
        true
    }

    /// TEST-ONLY: peek whether the shutdown slot is currently claimed,
    /// without claiming or releasing it (#558 round-4 U3/U5) — lets a test
    /// poll-with-timeout for "the slot became free" instead of an
    /// arbitrary sleep.
    #[cfg(test)]
    fn shutdown_slot_claimed(&self) -> bool {
        let guard = match self.shutdown.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.is_some()
    }

    /// Release a previously-claimed shutdown slot (#558 round-3 T9): used
    /// ONLY when `spawn_sync_task`'s spawned task bails out EARLY, after it
    /// already claimed the slot but before a loop is actually running (an
    /// already-taken `nudge_rx`, or a failed HTTP client build). Without
    /// this, the slot stays claimed FOREVER — `claim_shutdown_slot` would
    /// keep reporting "already running" to every future spawn attempt even
    /// though nothing is actually running.
    fn release_shutdown_slot(&self) {
        let mut guard = match self.shutdown.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = None;
    }
}

impl AppState {
    /// Fire-and-forget nudge after a local song mutation.
    pub(crate) fn nudge_sync(&self) {
        self.sync.nudge();
    }

    pub async fn sync_status_snapshot(&self) -> SyncStatus {
        self.sync.snapshot().await
    }

    /// Restore a trashed song; a restore is a local edit that must propagate.
    pub async fn restore_presentation(
        &self,
        presentation_id: presenter_core::PresentationId,
    ) -> anyhow::Result<()> {
        self.repository()
            .restore_presentation(presentation_id)
            .await?;
        self.drop_presentation_caches().await;
        self.nudge_sync();
        Ok(())
    }

    /// Synced-in / restored rows invalidate the per-id presentation cache wholesale —
    /// entries are lazily reloaded from the DB on next access.
    pub(crate) async fn drop_presentation_caches(&self) {
        self.caches.presentation.write().await.clear();
    }

    /// Spawn the sync loop iff a peer is configured (#555).
    pub(crate) fn maybe_spawn_sync(&self, peer_url: Option<String>) {
        if let Some(peer_url) = peer_url {
            tracing::info!(%peer_url, "song sync enabled");
            self.spawn_sync_task(peer_url);
        }
    }

    /// Start the pull loop against `peer_url`. Called once from `from_config` when the
    /// env var is set. The shutdown sender is kept ALIVE on the coordinator (S1) —
    /// it must NOT be dropped right after spawn, or the loop's
    /// `tokio::select!` shutdown branch resolves immediately and kills the
    /// loop before it does any real work. It is only dropped (cleanly
    /// stopping the loop) when the coordinator's Arc itself is finally
    /// dropped at process exit.
    ///
    /// #558 R5: a second call must NEVER overwrite an already-installed
    /// shutdown sender — doing so drops the OLD sender, which resolves the
    /// RUNNING loop's `shutdown_rx` and kills it, while the NEW task then
    /// finds `nudge_rx` already taken (by the first, now-dying loop) and
    /// refuses to start. Net effect: zero live loops. So the "is a loop
    /// already running?" check-and-claim happens under ONE lock
    /// acquisition, BEFORE anything else — a second spawn bails out
    /// immediately, leaving the first loop completely untouched.
    ///
    /// #558 round-3 T9: the slot is claimed SYNCHRONOUSLY here, before the
    /// task is even spawned — but the spawned task can still bail out
    /// EARLY (an already-taken `nudge_rx`, or a failed HTTP client build)
    /// BEFORE a loop is actually running. Each such early-return path MUST
    /// release the slot it claimed (and reset `status` if it already
    /// flipped `enabled`), or the slot stays claimed forever and every
    /// subsequent spawn attempt silently refuses to start a loop that was
    /// never actually running.
    ///
    /// #558 round-4 U3: the HTTP client is built BEFORE `nudge_rx` is
    /// taken — both preconditions are acquired ahead of any `status`
    /// mutation. The client used to be built AFTER nudge_rx was taken, so
    /// a client-build failure left nudge_rx PERMANENTLY gone (nothing ever
    /// put it back): every later spawn attempt then found nudge_rx already
    /// missing and hit the MISLEADING "already started" warning forever,
    /// even though no loop had ever actually run. Building the client
    /// first means a client-build failure never removes nudge_rx from its
    /// slot at all. Every early-return path also resets `status` BEFORE
    /// releasing the claimed slot — releasing first would let a concurrent
    /// respawn claim the slot and flip status to enabled, which this
    /// cleanup would then clobber back to disabled.
    pub(crate) fn spawn_sync_task(&self, peer_url: String) {
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let state = self.clone();
        let coordinator = self.sync.clone();
        let rx_slot = coordinator.nudge_rx.clone();
        let status = coordinator.status.clone();

        if !coordinator.claim_shutdown_slot(shutdown_tx) {
            warn!("sync task already started; not spawning a second loop");
            return;
        }

        tokio::spawn(async move {
            let client = match reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
            {
                Ok(client) => client,
                Err(err) => {
                    warn!(?err, "sync disabled: could not build HTTP client");
                    {
                        let mut s = status.write().await;
                        s.enabled = false;
                        s.peer_url = None;
                    }
                    coordinator.release_shutdown_slot();
                    return;
                }
            };

            let mut nudge_rx = match rx_slot.lock().await.take() {
                Some(rx) => rx,
                None => {
                    warn!("sync task already started; not starting a second loop");
                    {
                        let mut s = status.write().await;
                        s.enabled = false;
                        s.peer_url = None;
                    }
                    coordinator.release_shutdown_slot();
                    return;
                }
            };
            {
                let mut s = status.write().await;
                s.enabled = true;
                s.peer_url = Some(peer_url.clone());
            }

            let mut ticker = interval(SYNC_INTERVAL);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        info!("sync loop shutting down");
                        break;
                    }
                    _ = ticker.tick() => {
                        run_and_record(&state, &peer_url, &client, &status).await;
                    }
                    maybe = nudge_rx.recv() => {
                        if maybe.is_none() { break; }
                        // Debounce: coalesce a burst of edits into one cycle.
                        tokio::time::sleep(NUDGE_DEBOUNCE).await;
                        while nudge_rx.try_recv().is_ok() {}
                        run_and_record(&state, &peer_url, &client, &status).await;
                    }
                }
            }
        });
    }
}

async fn run_and_record(
    state: &AppState,
    peer_url: &str,
    client: &reqwest::Client,
    status: &Arc<RwLock<SyncStatus>>,
) {
    let started = Utc::now();
    let peer_version = fetch_peer_version(client, peer_url).await;
    match run_sync_cycle(state, peer_url, client).await {
        Ok((pulled, applied, errors)) => {
            let mut s = status.write().await;
            s.last_run = Some(started);
            s.last_success = Some(Utc::now());
            s.last_error = None;
            s.peer_healthy = peer_version.is_some();
            s.peer_version = peer_version;
            s.pulled_last_cycle = pulled;
            s.applied_last_cycle = applied;
            s.last_cycle_errors = errors;
        }
        Err(err) => {
            warn!(?err, "sync cycle failed");
            let mut s = status.write().await;
            s.last_run = Some(started);
            s.last_error = Some(err.to_string());
            s.peer_healthy = false;
            s.peer_version = peer_version;
        }
    }
}

async fn fetch_peer_version(client: &reqwest::Client, peer_url: &str) -> Option<String> {
    let resp = client
        .get(format!("{peer_url}/healthz"))
        .send()
        .await
        .ok()?;
    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// One reconciliation pass against the peer. Returns (pulled, applied, errors).
/// Directly callable from the integration test (bypasses the loop).
pub(crate) async fn run_sync_cycle(
    state: &AppState,
    peer_url: &str,
    client: &reqwest::Client,
) -> anyhow::Result<(usize, usize, usize)> {
    let repo = state.repository();

    // Index our local identities → updated_at for the LWW gate.
    let local = repo.list_sync_manifest().await?;
    let mut local_map = std::collections::HashMap::new();
    for row in &local {
        local_map.insert(row.sync_id.clone(), row.updated_at);
    }

    let peer_manifest: Vec<SyncManifestEntryDto> = client
        .get(format!("{peer_url}/sync/manifest"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // #558 round-4 U2: the peer's FULL sync_id set, so the apply step can
    // tell "a local adopt-by-name candidate whose sync_id the peer already
    // tracks separately" from "a genuinely orphaned local song" — see
    // `apply_sync_presentation`'s adopt-by-name single-shot gate.
    let peer_sync_ids: std::collections::HashSet<String> =
        peer_manifest.iter().map(|e| e.sync_id.clone()).collect();

    let mut pulled = 0usize;
    let mut applied = 0usize;
    let mut errors = 0usize;
    for entry in &peer_manifest {
        let local_updated = local_map.get(&entry.sync_id).copied();
        if !presenter_persistence::sync_should_apply(
            entry.updated_at,
            entry.deleted_at.is_some(),
            local_updated,
        ) {
            continue;
        }
        pulled += 1;
        // #558 round-4 U1(b): a single song's fetch/apply is ISOLATED — a
        // failure here (e.g. an oversize legacy stored slide the peer
        // still hasn't fixed, or any other per-song fault) must never
        // abort the whole cycle via `?`; every other manifest entry
        // deserves its own chance to sync in the same cycle.
        match fetch_and_apply_one(repo, client, peer_url, entry, &peer_sync_ids).await {
            Ok(wrote) => {
                if wrote {
                    applied += 1;
                }
            }
            Err(err) => {
                errors += 1;
                warn!(
                    ?err,
                    sync_id = %entry.sync_id,
                    name = %entry.name,
                    "sync: single-song fetch/apply failed — continuing with the next manifest entry"
                );
            }
        }
    }
    if applied > 0 {
        // Synced-in changes must be visible to this instance's own UI/caches.
        state.drop_presentation_caches().await;
    }
    Ok((pulled, applied, errors))
}

/// Fetch one song's full content from the peer and apply it locally.
/// Isolated per-song (#558 round-4 U1(b)) — the caller catches any error
/// this returns, logs + counts it, and moves on to the next manifest
/// entry instead of aborting the whole cycle.
async fn fetch_and_apply_one(
    repo: &presenter_persistence::Repository,
    client: &reqwest::Client,
    peer_url: &str,
    entry: &SyncManifestEntryDto,
    peer_sync_ids: &std::collections::HashSet<String>,
) -> anyhow::Result<bool> {
    let dto: SyncPresentationDto = client
        .get(format!("{peer_url}/sync/presentations/{}", entry.sync_id))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let outcome = repo
        .apply_sync_presentation(&dto.into(), peer_sync_ids)
        .await?;
    info!(sync_id = %entry.sync_id, name = %entry.name, ?outcome, "sync applied");
    Ok(outcome.wrote())
}

#[cfg(test)]
mod tests {
    use super::SyncCoordinator;
    use crate::state::AppState;

    /// Poll `cond` until it returns true, or panic once `timeout` elapses
    /// (#558 round-4 U5) — a deterministic replacement for an arbitrary
    /// `sleep` when waiting on a background task's side effect.
    async fn poll_until<F, Fut>(mut cond: F, timeout: std::time::Duration, what: &str)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if cond().await {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("timed out waiting for: {what}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn early_return_paths_always_reset_status_even_when_it_was_left_enabled() {
        // #558 round-4 U3: the nudge_rx-gone early-return branch used to
        // skip resetting `status` entirely — harmless by accident on a
        // fresh AppState (status starts disabled), but if `status.enabled`
        // was ever left `true` from an inconsistent prior state (exactly
        // the "status reset after slot-release can clobber a racing
        // successful respawn" race this fix addresses), an early return
        // must still leave status correctly DISABLED, never silently
        // preserve a stale "enabled" that no running loop backs. Building
        // the HTTP client BEFORE taking nudge_rx (and resetting status
        // BEFORE releasing the claimed slot) makes every early-return path
        // do this unconditionally.
        let state = AppState::in_memory().await.unwrap();

        // Simulate a stale "enabled" flag left over from an earlier
        // inconsistent run.
        {
            let mut s = state.sync.status.write().await;
            s.enabled = true;
            s.peer_url = Some("http://stale-peer".to_string());
        }
        // Force the early-return path: nudge_rx already gone.
        let _ = state.sync.nudge_rx.lock().await.take();

        state.spawn_sync_task("http://127.0.0.1:1".to_string());
        poll_until(
            || async { !state.sync.shutdown_slot_claimed() },
            std::time::Duration::from_secs(2),
            "shutdown slot released after the early return",
        )
        .await;

        let status = state.sync_status_snapshot().await;
        assert!(
            !status.enabled,
            "an early return must reset status to disabled, never leave a stale enabled flag"
        );
        assert!(
            status.peer_url.is_none(),
            "an early return must clear peer_url, never leave a stale one"
        );

        // A subsequent, legitimate spawn (nudge_rx restored) must actually
        // start a running loop — not find nudge_rx permanently gone, and
        // not have its own successful status flip clobbered by the
        // earlier cleanup.
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        *state.sync.nudge_rx.lock().await = Some(rx);
        state.spawn_sync_task("http://127.0.0.1:1".to_string());
        poll_until(
            || async { state.sync_status_snapshot().await.enabled },
            std::time::Duration::from_secs(2),
            "a subsequent spawn actually starts a running loop",
        )
        .await;
    }

    #[tokio::test]
    async fn spawn_sync_task_releases_the_shutdown_slot_when_nudge_rx_is_already_gone() {
        // #558 round-3 T9: spawn_sync_task claimed the shutdown slot
        // SYNCHRONOUSLY (before tokio::spawn), then discovered — only
        // INSIDE the spawned task — that nudge_rx was already gone, and
        // returned early WITHOUT releasing the slot it had just claimed.
        // The slot then stayed permanently claimed, so every SUBSEQUENT
        // spawn_sync_task call silently refused to start a loop forever
        // (claim_shutdown_slot sees it's occupied), even though no loop
        // was ever actually running.
        let state = AppState::in_memory().await.unwrap();
        // Simulate nudge_rx already taken (e.g. a prior partial init).
        let _ = state.sync.nudge_rx.lock().await.take();

        state.spawn_sync_task("http://127.0.0.1:1".to_string());
        // Let the spawned task run its early-return path.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Put a fresh receiver back — in production this slot is only ever
        // taken once for real (a genuinely running loop keeps it), so a
        // legitimate later attempt has one to take.
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        *state.sync.nudge_rx.lock().await = Some(rx);

        // If the slot leaked, this claim fails and no loop can ever start.
        let (probe_tx, _probe_rx) = tokio::sync::oneshot::channel::<()>();
        assert!(
            state.sync.claim_shutdown_slot(probe_tx),
            "the shutdown slot must be free after the early-return path, not leaked"
        );
    }

    #[tokio::test]
    async fn coordinator_defaults_and_nudge_do_not_panic() {
        let c = SyncCoordinator::new();
        c.nudge();
        c.nudge(); // full channel is fine — a nudge is already pending
        let s = c.snapshot().await;
        assert!(!s.enabled);
        assert!(s.peer_url.is_none());
    }

    #[test]
    fn claim_shutdown_slot_never_overwrites_an_already_installed_sender() {
        // R5 regression: a second claim used to overwrite the slot
        // unconditionally, dropping the FIRST sender (which kills a
        // running loop's `shutdown_rx`). The fix is check-and-claim: the
        // first claim succeeds and installs its sender; a second claim
        // must fail WITHOUT touching the slot, so the first sender stays
        // alive (provable here since dropping it would resolve `rx1`).
        let c = SyncCoordinator::new();
        let (tx1, mut rx1) = tokio::sync::oneshot::channel::<()>();
        let (tx2, _rx2) = tokio::sync::oneshot::channel::<()>();

        assert!(c.claim_shutdown_slot(tx1), "the first claim must succeed");
        assert!(
            !c.claim_shutdown_slot(tx2),
            "a second claim while one is already installed must fail"
        );
        assert!(
            rx1.try_recv().is_err(),
            "tx1 must still be alive (not dropped by the failed second claim)"
        );
        // A closed/dropped channel would report Closed here; an empty-but-alive
        // channel reports Empty -- either way `is_err()` above holds, so
        // assert the SPECIFIC alive signal too.
        assert_eq!(
            rx1.try_recv().unwrap_err(),
            tokio::sync::oneshot::error::TryRecvError::Empty,
            "tx1 specifically must be EMPTY (alive), not Closed (dropped)"
        );
    }
}
