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
}

/// Clonable handle stored on AppState. The receiver is taken once by the loop.
#[derive(Clone)]
pub struct SyncCoordinator {
    nudge_tx: mpsc::Sender<()>,
    nudge_rx: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
    status: Arc<RwLock<SyncStatus>>,
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
        }
    }

    /// Non-blocking; a full channel already has a nudge pending.
    pub fn nudge(&self) {
        let _ = self.nudge_tx.try_send(());
    }

    pub async fn snapshot(&self) -> SyncStatus {
        self.status.read().await.clone()
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

    /// Spawn the sync loop iff a peer is configured (#555). The returned shutdown
    /// sender is intentionally dropped — the loop runs for the process lifetime;
    /// the oneshot closing on drop is a clean shutdown-on-exit.
    pub(crate) fn maybe_spawn_sync(&self, peer_url: Option<String>) {
        if let Some(peer_url) = peer_url {
            tracing::info!(%peer_url, "song sync enabled");
            let _ = self.spawn_sync_task(peer_url);
        }
    }

    /// Start the pull loop against `peer_url`. Called once from `from_config` when the
    /// env var is set. Returns the shutdown sender (dropped-on-exit is fine in prod).
    pub(crate) fn spawn_sync_task(&self, peer_url: String) -> oneshot::Sender<()> {
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let state = self.clone();
        let coordinator = self.sync.clone();
        let rx_slot = coordinator.nudge_rx.clone();
        let status = coordinator.status.clone();

        tokio::spawn(async move {
            let mut nudge_rx = match rx_slot.lock().await.take() {
                Some(rx) => rx,
                None => {
                    warn!("sync task already started; not starting a second loop");
                    return;
                }
            };
            {
                let mut s = status.write().await;
                s.enabled = true;
                s.peer_url = Some(peer_url.clone());
            }
            let client = match reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
            {
                Ok(client) => client,
                Err(err) => {
                    warn!(?err, "sync disabled: could not build HTTP client");
                    return;
                }
            };

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
        shutdown_tx
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
        Ok((pulled, applied)) => {
            let mut s = status.write().await;
            s.last_run = Some(started);
            s.last_success = Some(Utc::now());
            s.last_error = None;
            s.peer_healthy = peer_version.is_some();
            s.peer_version = peer_version;
            s.pulled_last_cycle = pulled;
            s.applied_last_cycle = applied;
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

/// One reconciliation pass against the peer. Returns (pulled, applied). Directly
/// callable from the integration test (bypasses the loop).
pub(crate) async fn run_sync_cycle(
    state: &AppState,
    peer_url: &str,
    client: &reqwest::Client,
) -> anyhow::Result<(usize, usize)> {
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

    let mut pulled = 0usize;
    let mut applied = 0usize;
    for entry in peer_manifest {
        let local_updated = local_map.get(&entry.sync_id).copied();
        if !presenter_persistence::sync_should_apply(entry.updated_at, local_updated) {
            continue;
        }
        pulled += 1;
        let dto: SyncPresentationDto = client
            .get(format!("{peer_url}/sync/presentations/{}", entry.sync_id))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        match repo.apply_sync_presentation(&dto.into()).await {
            Ok(outcome) => {
                if outcome.wrote() {
                    applied += 1;
                }
                info!(sync_id = %entry.sync_id, name = %entry.name, ?outcome, "sync applied");
            }
            Err(err) => warn!(?err, sync_id = %entry.sync_id, "sync apply failed"),
        }
    }
    if applied > 0 {
        // Synced-in changes must be visible to this instance's own UI/caches.
        state.drop_presentation_caches().await;
    }
    Ok((pulled, applied))
}

#[cfg(test)]
mod tests {
    use super::SyncCoordinator;

    #[tokio::test]
    async fn coordinator_defaults_and_nudge_do_not_panic() {
        let c = SyncCoordinator::new();
        c.nudge();
        c.nudge(); // full channel is fine — a nudge is already pending
        let s = c.snapshot().await;
        assert!(!s.enabled);
        assert!(s.peer_url.is_none());
    }
}
