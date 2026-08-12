//! #555 song-sync engine: a symmetric pull loop against the peer instance, plus a
//! debounced nudge after local mutations. Reuses the AbleSet-tracker background-task
//! shape (interval + oneshot shutdown). Applied rows carry the PEER timestamp → no echo.
use std::sync::Arc;

use chrono::{DateTime, Utc};
use presenter_core::PresentationId;
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
    /// #647: the library's own STABLE identity (its `sync_id`), carried
    /// alongside `library_name` so the receiving apply path can join by
    /// IDENTITY instead of the current name string — a rename in flight, or
    /// a #636 disambiguated collision name, means the CURRENT local name is
    /// no longer a reliable key and joining by it can mis-file the
    /// presentation onto an unrelated library, or manufacture a phantom
    /// one. `#[serde(default)]`: an OLD peer that has not upgraded never
    /// sends this field, so it deserializes to `None` here — the apply path
    /// then falls back to the pre-#647 name-only join, unchanged. Safe in
    /// both wire directions since neither DTO has `deny_unknown_fields`
    /// (see the comment below).
    #[serde(default)]
    pub library_sync_id: Option<String>,
    pub name: String,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPresentationDto {
    pub sync_id: String,
    pub library_name: String,
    /// #647 — see `SyncManifestEntryDto::library_sync_id`'s doc comment.
    #[serde(default)]
    pub library_sync_id: Option<String>,
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
            library_sync_id: d.library_sync_id,
            name: d.name,
            updated_at: d.updated_at,
            deleted_at: d.deleted_at,
            slides: d.slides,
        }
    }
}

/// #578 library-sync wire DTO — a library has no content to fetch, so the
/// manifest row carries everything the apply needs. camelCase, and (like the
/// other sync DTOs) NO `deny_unknown_fields`, so a future additive field never
/// 422s an older peer mid-rollout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncLibraryManifestEntryDto {
    pub sync_id: String,
    pub name: String,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl From<SyncLibraryManifestEntryDto> for presenter_persistence::SyncLibraryManifestRow {
    fn from(d: SyncLibraryManifestEntryDto) -> Self {
        presenter_persistence::SyncLibraryManifestRow {
            sync_id: d.sync_id,
            name: d.name,
            updated_at: d.updated_at,
            deleted_at: d.deleted_at,
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
    /// Fire-and-forget nudge after a local song mutation. Also invalidates
    /// the resolved AbleSet cache (#575) — every caller of `nudge_sync` is a
    /// mutation that can change which presentations exist or what they're
    /// named, and the cache must never be left resolving stale/missing ids
    /// until the next unrelated AbleSet-settings save.
    pub(crate) async fn nudge_sync(&self) {
        self.invalidate_ableset_cache().await;
        self.sync.nudge();
    }

    pub async fn sync_status_snapshot(&self) -> SyncStatus {
        self.sync.snapshot().await
    }

    /// Restore a trashed song; a restore is a local edit that must propagate.
    /// Deliberately calls BOTH `drop_presentation_caches()` (also clears the
    /// AbleSet cache, #575) AND `nudge_sync()` (which does the same,
    /// harmlessly redundant here) — the former handles the sync engine's own
    /// bulk-apply path, the latter is the seam every OTHER local mutation
    /// goes through; restore is the one call site that needs both.
    pub async fn restore_presentation(
        &self,
        presentation_id: presenter_core::PresentationId,
    ) -> anyhow::Result<()> {
        self.repository()
            .restore_presentation(presentation_id)
            .await?;
        self.drop_presentation_caches().await;
        self.nudge_sync().await;
        Ok(())
    }

    /// Synced-in / restored rows invalidate the per-id presentation cache wholesale —
    /// entries are lazily reloaded from the DB on next access. Also invalidates the
    /// resolved AbleSet cache (#575) — this is the wholesale post-mutation
    /// convergence point the sync engine itself calls (which never goes through
    /// `nudge_sync`, to avoid re-nudging the very sync loop that just ran).
    pub(crate) async fn drop_presentation_caches(&self) {
        self.caches.presentation.write().await.clear();
        self.invalidate_ableset_cache().await;
    }

    /// #558 V2: evict ONE presentation's cache entry — used after a
    /// successful sync apply that wrote to a KNOWN existing local id, so the
    /// next caller to acquire that presentation's lock (any snapshot-replace
    /// edit op) re-reads fresh from the DB instead of a stale pre-apply
    /// snapshot. Cheaper than the wholesale `drop_presentation_caches` when
    /// only one presentation actually changed.
    pub(crate) async fn drop_one_presentation_cache(&self, presentation_id: PresentationId) {
        self.caches
            .presentation
            .write()
            .await
            .remove(&presentation_id);
    }

    /// #558 V2: acquire the shared per-presentation lock for a sync apply —
    /// the SAME registry `slides/edit_ops.rs`'s snapshot-replace ops use, so
    /// the two families of writers can never interleave on one presentation.
    pub(crate) async fn presentation_lock_for_sync(
        &self,
        presentation_id: PresentationId,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        self.presentation_locks.lock(presentation_id).await
    }

    /// TEST-ONLY (#558 W5): non-blocking probe proving the shared
    /// per-presentation lock is currently FREE — used to demonstrate that
    /// `fetch_and_apply_one` no longer holds it across the peer content
    /// fetch.
    #[cfg(test)]
    pub(crate) fn presentation_lock_try_acquire_for_test(
        &self,
        presentation_id: PresentationId,
    ) -> bool {
        self.presentation_locks.try_lock(presentation_id)
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
            s.peer_healthy = peer_version.is_some();
            s.peer_version = peer_version;
            s.pulled_last_cycle = pulled;
            s.applied_last_cycle = applied;
            s.last_cycle_errors = errors;

            // #558 V6: a cycle where EVERY fetched entry failed (a systemic
            // problem — e.g. the peer answers /sync/manifest fine but every
            // per-song fetch errors) must not report itself healthy. Per-song
            // failures are isolated from this cycle's own `Result` (#558
            // round-4 U1(b)) precisely so ONE bad song never aborts the rest
            // — but that isolation also means an all-fail cycle previously
            // fell through to this `Ok` branch and unconditionally refreshed
            // `last_success` / cleared `last_error`, hiding a 100%-failed
            // cycle behind a healthy-looking status.
            if cycle_all_failed(pulled, applied, errors) {
                s.last_error = Some(format!("{errors}/{pulled} song fetches failed"));
            } else {
                s.last_success = Some(Utc::now());
                s.last_error = None;
            }
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

/// #558 V6: whether a completed cycle counts as a systemic failure (every
/// FETCHED entry errored) that must not report itself healthy. `pulled == 0`
/// (nothing needed fetching this cycle — the quiet, genuinely healthy case)
/// is never "all failed".
fn cycle_all_failed(pulled: usize, applied: usize, errors: usize) -> bool {
    pulled > 0 && applied == 0 && errors == pulled
}

/// #558 W2, reclassified by X4: whether `err` is a TRANSPORT-shaped failure
/// — the peer is genuinely unreachable (connection refused, a request
/// timeout, or the connection was reset/dropped partway through the
/// response body) — as opposed to an APPLICATION-level failure (the peer
/// answered with an error status for this one song, a malformed body on an
/// otherwise-successful response, or the local apply itself failed). A
/// `reqwest::Error` carries a `status()` ONLY when it was built from
/// `error_for_status()` (a real 4xx/5xx the peer sent); every other kind
/// reports `status() == None`.
///
/// #558 X4: the OLD `is_timeout() || is_connect()` check missed a
/// connection that accepts the request, starts streaming the response,
/// then resets/closes mid-body — neither a timeout nor a connect-phase
/// failure by reqwest's own classification, so it fell through to
/// "application-level" even though the peer is genuinely gone from that
/// point on. `reqwest::Error::is_decode()` CANNOT tell this apart from a
/// genuine malformed-JSON parse failure on a fully-received 2xx body —
/// `Response::bytes()`/`json()` route EVERY body-read failure through the
/// same `Kind::Decode` (verified against reqwest 0.12: both a connection
/// reset mid-transfer AND a JSON syntax error report `is_decode() == true`,
/// `status() == None`). The two ARE distinguishable one level deeper: a
/// dropped/reset connection carries a `std::io::Error` in its `source()`
/// chain whose `ErrorKind` names the connection death (`UnexpectedEof` —
/// the promised body never fully arrived — or `ConnectionReset` /
/// `ConnectionAborted` / `BrokenPipe`); a JSON syntax error's source chain
/// never contains an `io::Error` at all (its source is a `serde_json`
/// parse error over bytes that were already fully, successfully read).
fn is_transport_failure(err: &anyhow::Error) -> bool {
    let Some(e) = err.downcast_ref::<reqwest::Error>() else {
        return false;
    };
    if e.status().is_some() {
        return false;
    }
    if e.is_timeout() || e.is_connect() {
        return true;
    }
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        if let Some(io_err) = cause.downcast_ref::<std::io::Error>() {
            use std::io::ErrorKind;
            if matches!(
                io_err.kind(),
                ErrorKind::UnexpectedEof
                    | ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::BrokenPipe
            ) {
                return true;
            }
        }
        source = cause.source();
    }
    false
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
/// Directly callable from the integration test (bypasses the loop). Thin
/// wrapper over `run_sync_cycle_with_clients` using ONE client for both the
/// manifest fetch and the per-song content fetches — the real production
/// shape, where a single client with a generous timeout serves everything.
pub(crate) async fn run_sync_cycle(
    state: &AppState,
    peer_url: &str,
    client: &reqwest::Client,
) -> anyhow::Result<(usize, usize, usize)> {
    run_sync_cycle_with_clients(state, peer_url, client, client).await
}

/// Same reconciliation pass as `run_sync_cycle`, but lets the manifest fetch
/// and the per-song content fetches use DIFFERENT clients (#558 X7) — needed
/// by the breaker unit tests, which pin a razor-thin timeout on the per-song
/// fetches ONLY (to force a genuine transport failure deterministically).
/// Routing the cheap, always-fast manifest fetch through a NORMAL-timeout
/// client keeps it from itself flaking under a loaded CI runner, where even
/// an un-delayed mock response can occasionally exceed a few milliseconds.
pub(crate) async fn run_sync_cycle_with_clients(
    state: &AppState,
    peer_url: &str,
    manifest_client: &reqwest::Client,
    content_client: &reqwest::Client,
) -> anyhow::Result<(usize, usize, usize)> {
    let repo = state.repository();

    // #578: reconcile library identities (rename + tombstone propagation)
    // FIRST — before presentations — so a synced presentation attaches by name
    // to a library whose name/tombstone state already matches the peer.
    // Best-effort and isolated (see `reconcile_libraries`): degrading here
    // never blocks the presentation tombstones that actually fix the
    // resurrection loop.
    let libraries_applied = reconcile_libraries(state, manifest_client, peer_url).await;

    // Index our local identities → updated_at for the LWW gate.
    let local = repo.list_sync_manifest().await?;
    let mut local_map = std::collections::HashMap::new();
    for row in &local {
        local_map.insert(row.sync_id.clone(), row.updated_at);
    }

    let peer_manifest: Vec<SyncManifestEntryDto> = manifest_client
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
    let mut consecutive_failures = 0u32;
    for entry in &peer_manifest {
        let local_updated = local_map.get(&entry.sync_id).copied();
        process_manifest_entry(
            state,
            content_client,
            peer_url,
            entry,
            &peer_sync_ids,
            local_updated,
            &mut pulled,
            &mut applied,
            &mut errors,
            &mut consecutive_failures,
        )
        .await?;
    }
    if applied > 0 || libraries_applied > 0 {
        // Synced-in changes — songs OR library renames/tombstones (#578) —
        // must be visible to this instance's own UI/caches, incl. the AbleSet
        // resolved-name cache (#575) that `drop_presentation_caches` clears.
        state.drop_presentation_caches().await;
    }
    Ok((pulled, applied, errors))
}

/// #578: reconcile the peer's library identities (rename + tombstone
/// propagation) under LWW. Best-effort and fully ISOLATED — a missing endpoint
/// (an OLD peer during rollout skew answers 404), a decode failure, or any
/// single-library apply error is logged and skipped, NEVER aborting the whole
/// cycle. This is safe because the presentation-level tombstones (from
/// `delete_library` soft-deleting each song) already fix the resurrection loop
/// through the EXISTING presentation manifest with zero library-sync support;
/// library sync only adds library-row rename + tombstone convergence on top.
/// Returns the number of libraries actually written.
async fn reconcile_libraries(state: &AppState, client: &reqwest::Client, peer_url: &str) -> usize {
    let resp = match client
        .get(format!("{peer_url}/sync/libraries/manifest"))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(err) => {
            warn!(
                ?err,
                "sync: library manifest fetch failed — skipping library reconciliation"
            );
            return 0;
        }
    };
    if !resp.status().is_success() {
        // An OLD peer (rollout skew) has no such endpoint → 404. Not an error.
        info!(
            status = %resp.status(),
            "sync: peer has no library manifest (older peer?) — skipping library reconciliation"
        );
        return 0;
    }
    let manifest: Vec<SyncLibraryManifestEntryDto> = match resp.json().await {
        Ok(manifest) => manifest,
        Err(err) => {
            warn!(
                ?err,
                "sync: library manifest decode failed — skipping library reconciliation"
            );
            return 0;
        }
    };
    let repo = state.repository();
    let mut applied = 0usize;
    for entry in manifest {
        let incoming: presenter_persistence::SyncLibraryManifestRow = entry.into();
        match repo.apply_sync_library(&incoming).await {
            Ok(outcome) if outcome.wrote() => applied += 1,
            Ok(_) => {}
            Err(err) => warn!(
                ?err,
                "sync: single-library apply failed — continuing with the next library"
            ),
        }
    }
    applied
}

/// Process ONE manifest entry within `run_sync_cycle_with_clients`'s loop —
/// the LWW pre-filter, the isolated per-song fetch+apply (#558 round-4
/// U1(b): a failure here must never abort the whole cycle via `?`), and
/// folding the result into the running counters. (Extracted per the #558
/// function-length gate.)
///
/// #558 V7: 3 consecutive per-song fetch failures signal a SYSTEMIC problem
/// (the peer answered `/sync/manifest` fine but then died, or became
/// unreachable, mid-cycle) rather than scattered per-song faults — trip the
/// breaker and abort (an `Err` return) instead of burning a full timeout on
/// every remaining manifest entry. Scattered, non-consecutive failures (an
/// occasional bad song among healthy ones) never reach the threshold and
/// stay fully isolated, as before.
///
/// #558 W2: "consecutive" counts only TRANSPORT-shaped failures (the peer is
/// genuinely unreachable — connection refused/reset, or the request timed
/// out) — never a per-song APPLICATION-level failure (the peer answered,
/// just badly, for this ONE song: a 4xx/5xx status body, or a local apply
/// error). The breaker used to count EVERY per-song failure indiscriminately,
/// so 3 adjacent but individually harmless broken songs (each isolated per
/// round-4 U1(b)) tripped it and starved every healthy song after them —
/// exactly the isolation guarantee U1(b) exists to provide.
///
/// #558 X2: an application-level failure also RESETS the streak — any
/// response carrying an HTTP status (success OR a 4xx/5xx failure) proves
/// the peer is reachable, exactly like a genuine success does. Leaving the
/// streak untouched on an application-level failure let non-consecutive
/// transport errors accumulate ACROSS one (transport, transport,
/// (application 500 — reachable!), transport still summed to 3 and tripped
/// the breaker even though the peer proved itself reachable in between).
#[allow(clippy::too_many_arguments)]
async fn process_manifest_entry(
    state: &AppState,
    content_client: &reqwest::Client,
    peer_url: &str,
    entry: &SyncManifestEntryDto,
    peer_sync_ids: &std::collections::HashSet<String>,
    local_updated: Option<DateTime<Utc>>,
    pulled: &mut usize,
    applied: &mut usize,
    errors: &mut usize,
    consecutive_failures: &mut u32,
) -> anyhow::Result<()> {
    const CONSECUTIVE_FAILURE_BREAKER: u32 = 3;

    if !presenter_persistence::sync_should_apply(
        entry.updated_at,
        entry.deleted_at.is_some(),
        local_updated,
    ) {
        return Ok(());
    }
    *pulled += 1;
    match fetch_and_apply_one(state, content_client, peer_url, entry, peer_sync_ids).await {
        Ok(wrote) => {
            *consecutive_failures = 0;
            if wrote {
                *applied += 1;
            }
        }
        Err(err) => {
            *errors += 1;
            warn!(
                ?err,
                sync_id = %entry.sync_id,
                name = %entry.name,
                "sync: single-song fetch/apply failed — continuing with the next manifest entry"
            );
            if is_transport_failure(&err) {
                *consecutive_failures += 1;
                if *consecutive_failures >= CONSECUTIVE_FAILURE_BREAKER {
                    if *applied > 0 {
                        state.drop_presentation_caches().await;
                    }
                    anyhow::bail!(
                        "sync cycle aborted: {consecutive_failures} consecutive TRANSPORT \
                         failures (peer likely unreachable) — {applied} applied, {errors} \
                         errored before the breaker tripped"
                    );
                }
            } else {
                *consecutive_failures = 0;
            }
        }
    }
    Ok(())
}

/// Fetch one song's full content from the peer and apply it locally.
/// Isolated per-song (#558 round-4 U1(b)) — the caller catches any error
/// this returns, logs + counts it, and moves on to the next manifest
/// entry instead of aborting the whole cycle.
///
/// #558 W5: the peer content fetch (up to the client's 15s timeout) runs
/// FIRST, holding NO lock at all — holding the shared per-presentation lock
/// across that network round-trip used to block any concurrent edit op on
/// this presentation for the whole request.
///
/// #558 W8/W9: the lock target is then resolved from the JUST-FETCHED
/// content via `resolve_sync_apply_target` — the SAME candidate rule
/// `apply_sync_presentation` itself applies (by `sync_id`, or — for a live
/// entry with no `sync_id` match — the single live adopt-by-name
/// candidate). One shared implementation on both sides means this can never
/// again resolve a DIFFERENT (and therefore wrong) target than what the
/// real apply decides — which is exactly what the old, separately
/// implemented `find_live_presentation_id_by_name` probe could do (it never
/// gained the `peer_sync_ids` single-shot exclusion `try_adopt_by_name`
/// picked up in round-4 U2). The lock is acquired immediately before the
/// real apply transaction and holds the SAME per-presentation lock a
/// snapshot-replace edit op takes (`slides/edit_ops.rs`), so the two can
/// never interleave. A genuinely new identity (no existing local row at
/// all) has no lock target — nothing else could be concurrently editing a
/// presentation id nobody has learned yet. On a successful WRITE to a
/// resolved existing target, the AppState cache entry for that one
/// presentation is evicted (still under the lock) so the very next
/// lock-holder's read is guaranteed fresh, never a stale pre-apply snapshot.
async fn fetch_and_apply_one(
    state: &AppState,
    client: &reqwest::Client,
    peer_url: &str,
    entry: &SyncManifestEntryDto,
    peer_sync_ids: &std::collections::HashSet<String>,
) -> anyhow::Result<bool> {
    let repo = state.repository();

    let dto: SyncPresentationDto = client
        .get(format!("{peer_url}/sync/presentations/{}", entry.sync_id))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let incoming: presenter_persistence::SyncPresentation = dto.into();

    let lock_target = repo
        .resolve_sync_apply_target(&incoming, peer_sync_ids)
        .await?;
    let _guard = match lock_target {
        Some(id) => Some(state.presentation_lock_for_sync(id).await),
        None => None,
    };

    let (outcome, written_id) = repo
        .apply_sync_presentation(&incoming, peer_sync_ids)
        .await?;
    if outcome.wrote() {
        if let Some(id) = lock_target {
            state.drop_one_presentation_cache(id).await;
        }
        // #558 X5: the probe (`resolve_sync_apply_target`, above) and this
        // apply transaction can theoretically resolve to a DIFFERENT row
        // across the probe→transaction gap (see the doc comment on this
        // function). The DB write itself is already transaction-serialized
        // and LWW-safe regardless — but evict the id the apply ACTUALLY
        // wrote too, whenever it diverges from the pre-resolved lock
        // target, so a divergence never leaves a stale cached snapshot of
        // the real target un-evicted.
        if let Some(written) = written_id {
            if lock_target != Some(written) {
                warn!(
                    sync_id = %entry.sync_id,
                    name = %entry.name,
                    ?lock_target,
                    written = %written,
                    "sync: probe→transaction gap — locked target differs from the id \
                     actually written; evicting both"
                );
                state.drop_one_presentation_cache(written).await;
            }
        }
    }
    info!(sync_id = %entry.sync_id, name = %entry.name, ?outcome, "sync applied");
    Ok(outcome.wrote())
}

#[cfg(test)]
mod tests {
    use super::{is_transport_failure, SyncCoordinator};
    use crate::state::AppState;

    /// #558 X4: a peer that accepts the connection, sends headers, then
    /// drops the socket PARTWAY through the promised body is a GENUINE
    /// transport-shaped failure (the peer really did become unreachable
    /// mid-response) — the OLD `is_timeout() || is_connect()` check missed
    /// this exact case (neither a timeout nor a connect-phase failure by
    /// reqwest's own classification), so it never counted toward the
    /// systemic-failure breaker. Verified empirically (reqwest 0.12):
    /// `Response::bytes()` routes this through `Kind::Decode` — the SAME
    /// kind a malformed-JSON parse failure uses — so `is_decode()` alone
    /// cannot tell the two apart; see the companion test below and the
    /// `is_transport_failure` doc comment for the actual distinguishing
    /// signal (an `io::Error` in the source chain).
    #[tokio::test]
    async fn a_connection_reset_mid_body_is_classified_as_a_transport_failure() {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            // Promise 100 bytes, deliver 10, then drop the connection — the
            // client's body read must observe a genuine transport error.
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n0123456789")
                .await;
            // Dropping `socket` here closes the connection before the
            // promised body completes.
        });

        let err = reqwest::Client::new()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect("headers arrive fine — only the body read fails")
            .bytes()
            .await
            .expect_err("an incomplete body must error");

        assert!(
            err.status().is_none(),
            "a body-stream error carries no HTTP status"
        );
        assert!(
            is_transport_failure(&anyhow::Error::from(err)),
            "a connection reset mid-body must count toward the systemic-failure breaker"
        );
    }

    /// #558 X4: the fix above must NOT rely on `is_decode()` to exclude this
    /// case — a JSON decode failure on an otherwise-200 response (the peer
    /// answered IN FULL, proving it's reachable; the received bytes just
    /// aren't valid JSON) reports `is_decode() == true` too (reqwest 0.12
    /// routes every body-read failure through the same `Kind::Decode`), so
    /// `is_decode()` cannot distinguish it from the companion test's
    /// connection-reset case. `is_transport_failure` must still classify
    /// THIS one as application-level — the real distinguishing signal is
    /// the absence of an `io::Error` in the source chain (a JSON syntax
    /// error's source is a `serde_json` parse error over bytes that were
    /// already fully received, never an OS-level connection failure).
    #[tokio::test]
    async fn a_response_decode_failure_is_never_classified_as_transport() {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nnot json!!!")
                .await;
        });

        let err = reqwest::Client::new()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .expect_err("malformed JSON must fail to decode");

        assert!(
            err.status().is_none(),
            "a decode error carries no HTTP status either"
        );
        assert!(
            !is_transport_failure(&anyhow::Error::from(err)),
            "a decode failure proves the peer answered — never transport"
        );
    }

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
        // Let the spawned task run its early-return path (#558 round-4 U5:
        // poll the observable slot state instead of an arbitrary sleep).
        poll_until(
            || async { !state.sync.shutdown_slot_claimed() },
            std::time::Duration::from_secs(2),
            "shutdown slot released after the early-return path",
        )
        .await;

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
