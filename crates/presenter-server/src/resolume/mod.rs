mod clip_map;
mod driver;
mod handlers;
#[cfg(test)]
mod latency_tests;
mod types;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use presenter_core::{BibleBroadcast, BibleSlideOutput, ResolumeHost, ResolumeHostId};
use presenter_persistence::{Repository, ResolumePushAuditEntry};
use reqwest::Client;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tracing::error;
use uuid::Uuid;

use driver::{run_host_worker, HostCommand};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const HOST_COMMAND_CAPACITY: usize = 16;
/// Bounded buffer for per-push audit rows (#483). `try_send` drops rows when
/// full so the push path never blocks on the DB writer.
const AUDIT_CHANNEL_CAPACITY: usize = 512;
/// How long to collect per-host completions for one slide before emitting the
/// cross-host perceived-latency line. Slides during singing are >1 s apart, so
/// a 2 s window captures all hosts (incl. a slow ~655 ms one) without colliding
/// with the next slide's correlation id.
const PERCEIVED_FLUSH_AFTER: Duration = Duration::from_secs(2);
const PERCEIVED_SWEEP_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolumeConnectionState {
    Disabled,
    Connecting,
    Connected,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolumeConnectionSnapshot {
    pub state: ResolumeConnectionState,
    pub last_success: Option<DateTime<Utc>>,
    pub last_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub last_attempt: Option<DateTime<Utc>>,
    pub error_since: Option<DateTime<Utc>>,
}

impl ResolumeConnectionSnapshot {
    pub fn disabled() -> Self {
        Self {
            state: ResolumeConnectionState::Disabled,
            last_success: None,
            last_latency_ms: None,
            last_error: None,
            consecutive_failures: 0,
            last_attempt: None,
            error_since: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StageUpdate {
    pub current_main: Option<String>,
    pub current_translation: Option<String>,
    pub song_name: Option<String>,
    pub band_name: Option<String>,
    /// When the StageUpdate was enqueued for delivery to host workers.
    /// Used to measure queue-wait latency in the worker. Populated by
    /// `ResolumeRegistry::stage_update` if not set by the producer.
    pub enqueued_at: Option<Instant>,
    /// Correlation ID joining the server-side "stage click timing" log line
    /// with the per-host "resolume stage timing" log line.
    /// Populated by Task 3 click-path instrumentation; read by Task 4 worker.
    pub correlation_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct BibleUpdate {
    /// Legacy: passage from database lookup (deprecated)
    pub passage: Option<BibleBroadcast>,
    pub secondary_text: Option<String>,
    pub secondary_translation_code: Option<String>,
    /// New: single source of truth slide output (preferred)
    pub slide_output: Option<BibleSlideOutput>,
}

impl BibleUpdate {
    /// Create a Bible update from the new single-source-of-truth slide output.
    /// This is the preferred way to create a BibleUpdate.
    pub fn from_slide_output(output: Option<BibleSlideOutput>) -> Self {
        Self {
            passage: None,
            secondary_text: None,
            secondary_translation_code: None,
            slide_output: output,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TimerFrame {
    pub formatted: String,
}

impl TimerFrame {
    pub fn new(formatted: String) -> Self {
        Self { formatted }
    }
}

#[derive(Debug, Clone)]
pub struct ResolumeRegistry {
    client: Client,
    hosts: Arc<RwLock<HashMap<ResolumeHostId, HostEntry>>>,
    /// Non-blocking sink for per-push audit rows (#483). Set once via
    /// [`ResolumeRegistry::attach_audit_writer`] (the registry is constructed
    /// before the DB-backed writer is available, then the writer is attached
    /// before the first host is spawned). Empty in unit tests built via `new()`.
    /// `Arc<OnceLock>` so all clones of the registry share the same sink.
    audit_tx: Arc<OnceLock<mpsc::Sender<ResolumePushAuditEntry>>>,
}

#[derive(Debug)]
struct HostEntry {
    config: ResolumeHost,
    status: Arc<RwLock<ResolumeConnectionSnapshot>>,
    command_tx: mpsc::Sender<HostCommand>,
    handle: JoinHandle<()>,
}

impl ResolumeRegistry {
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| anyhow!("failed to build reqwest client: {e}"))?;
        Ok(Self {
            client,
            hosts: Arc::new(RwLock::new(HashMap::new())),
            audit_tx: Arc::new(OnceLock::new()),
        })
    }

    /// #483: wire the DB-backed per-push audit writer (idempotent). Spawns one
    /// background task that owns the repository, persists each push-audit row,
    /// and emits the cross-host perceived-latency line. The push path only
    /// `try_send`s to it. MUST be called before hosts are spawned (i.e. before
    /// `set_hosts`) so workers pick up the sink. Repeated calls are no-ops.
    pub fn attach_audit_writer(&self, repo: Repository) {
        if self.audit_tx.get().is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel(AUDIT_CHANNEL_CAPACITY);
        // If a concurrent caller won the race, drop our channel (its writer
        // task exits when the sender drops) and keep the installed one.
        if self.audit_tx.set(tx).is_ok() {
            spawn_audit_writer(repo, rx);
        }
    }

    pub async fn set_hosts(&self, hosts: Vec<ResolumeHost>) {
        let mut guard = self.hosts.write().await;
        let mut desired: HashMap<ResolumeHostId, ResolumeHost> =
            hosts.into_iter().map(|host| (host.id, host)).collect();

        // Stop hosts that no longer exist
        let existing_ids: Vec<_> = guard.keys().copied().collect();
        for id in existing_ids {
            if !desired.contains_key(&id) {
                if let Some(entry) = guard.remove(&id) {
                    let _ = entry.command_tx.send(HostCommand::Shutdown).await;
                    entry.handle.abort();
                }
            }
        }

        // Add or update entries
        for (id, host) in desired.drain() {
            match guard.get_mut(&id) {
                Some(entry) => {
                    if entry.config.host != host.host
                        || entry.config.port != host.port
                        || entry.config.is_enabled != host.is_enabled
                    {
                        let _ = entry
                            .command_tx
                            .send(HostCommand::RefreshConfig(host.clone()))
                            .await;
                        entry.config = host;
                    } else if entry.config.label != host.label {
                        entry.config = host;
                    }
                }
                None => {
                    let entry = self.spawn_host(host);
                    guard.insert(id, entry);
                }
            }
        }
    }

    fn spawn_host(&self, host: ResolumeHost) -> HostEntry {
        let (command_tx, command_rx) = mpsc::channel(HOST_COMMAND_CAPACITY);
        let status = Arc::new(RwLock::new(if host.is_enabled {
            ResolumeConnectionSnapshot {
                state: ResolumeConnectionState::Connecting,
                last_success: None,
                last_latency_ms: None,
                last_error: None,
                consecutive_failures: 0,
                last_attempt: None,
                error_since: None,
            }
        } else {
            ResolumeConnectionSnapshot::disabled()
        }));
        let client = self.client.clone();
        let status_clone = Arc::clone(&status);
        let config_clone = host.clone();
        let audit_tx = self.audit_tx.get().cloned();
        let handle = tokio::spawn(async move {
            if let Err(err) =
                run_host_worker(client, config_clone, status_clone, command_rx, audit_tx).await
            {
                error!(host_id = %host.id, ?err, "resolume worker terminated with error");
            }
        });
        HostEntry {
            config: host,
            status,
            command_tx,
            handle,
        }
    }

    pub async fn stage_update(&self, mut update: StageUpdate) {
        if update.enqueued_at.is_none() {
            update.enqueued_at = Some(Instant::now());
        }
        for (id, entry) in self.hosts.read().await.iter() {
            if entry
                .command_tx
                .try_send(HostCommand::Stage(update.clone()))
                .is_err()
            {
                tracing::warn!(host_id = %id, "resolume command channel full, dropping stage update");
            }
        }
    }

    pub async fn bible_update(&self, update: BibleUpdate) {
        for (id, entry) in self.hosts.read().await.iter() {
            if entry
                .command_tx
                .try_send(HostCommand::Bible(update.clone()))
                .is_err()
            {
                tracing::warn!(host_id = %id, "resolume command channel full, dropping bible update");
            }
        }
    }

    pub async fn timer_update(&self, frame: TimerFrame) {
        for (id, entry) in self.hosts.read().await.iter() {
            if entry
                .command_tx
                .try_send(HostCommand::Timer(frame.clone()))
                .is_err()
            {
                tracing::warn!(host_id = %id, "resolume command channel full, dropping timer update");
            }
        }
    }

    pub async fn snapshot(&self) -> HashMap<ResolumeHostId, ResolumeConnectionSnapshot> {
        let mut snapshots = HashMap::new();
        for (id, entry) in self.hosts.read().await.iter() {
            snapshots.insert(*id, entry.status.read().await.clone());
        }
        snapshots
    }

    pub async fn snapshot_for(&self, id: ResolumeHostId) -> ResolumeConnectionSnapshot {
        if let Some(entry) = self.hosts.read().await.get(&id) {
            entry.status.read().await.clone()
        } else {
            ResolumeConnectionSnapshot::disabled()
        }
    }
}

/// In-flight cross-host perceived-latency accumulator for one slide trigger
/// (keyed by correlation_id) — #483 logging requirement point 3.
struct PerceivedAgg {
    first_seen: Instant,
    /// max over hosts of (queue_wait + t_total) — the latency the user feels.
    max_perceived_ms: f64,
    hosts: usize,
    slowest_host: String,
}

/// Background task (one per `with_audit` registry) that owns the repository and:
/// 1. persists each per-push audit row (`resolume_push_audit`), and
/// 2. aggregates per-correlation_id perceived latency across all hosts and emits
///    ONE `presenter::resolume::timing` line per slide (the number felt on the
///    LED wall), so you don't mentally join the per-host lines (#483).
fn spawn_audit_writer(repo: Repository, mut rx: mpsc::Receiver<ResolumePushAuditEntry>) {
    tokio::spawn(async move {
        let mut pending: HashMap<String, PerceivedAgg> = HashMap::new();
        let mut sweep = tokio::time::interval(PERCEIVED_SWEEP_INTERVAL);
        loop {
            tokio::select! {
                maybe = rx.recv() => {
                    match maybe {
                        Some(entry) => {
                            if let Err(err) = repo.record_resolume_push_audit(&entry).await {
                                tracing::warn!(
                                    ?err,
                                    host = %entry.host,
                                    "failed to persist resolume push audit row"
                                );
                            }
                            if let Some(cid) = entry.correlation_id.clone() {
                                let perceived = entry.t_queue_wait_ms + entry.t_total_ms;
                                let agg = pending.entry(cid).or_insert_with(|| PerceivedAgg {
                                    first_seen: Instant::now(),
                                    max_perceived_ms: 0.0,
                                    hosts: 0,
                                    slowest_host: String::new(),
                                });
                                if perceived >= agg.max_perceived_ms {
                                    agg.max_perceived_ms = perceived;
                                    agg.slowest_host = entry.host.clone();
                                }
                                agg.hosts += 1;
                            }
                        }
                        None => break, // all senders dropped → registry gone
                    }
                }
                _ = sweep.tick() => flush_perceived(&mut pending, false),
            }
        }
        flush_perceived(&mut pending, true);
    });
}

/// Emit + remove perceived-latency aggregates. When `force` is false, only
/// entries older than `PERCEIVED_FLUSH_AFTER` are flushed (all hosts have had
/// time to report); when true, everything is drained (writer shutting down).
fn flush_perceived(pending: &mut HashMap<String, PerceivedAgg>, force: bool) {
    let now = Instant::now();
    pending.retain(|cid, agg| {
        if force || now.duration_since(agg.first_seen) >= PERCEIVED_FLUSH_AFTER {
            tracing::info!(
                target: "presenter::resolume::timing",
                correlation_id = cid.as_str(),
                hosts = agg.hosts,
                perceived_latency_ms = agg.max_perceived_ms,
                slowest_host = %agg.slowest_host,
                "resolume perceived latency (max across hosts)"
            );
            false
        } else {
            true
        }
    });
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionResult {
    pub success: bool,
    pub latency_ms: Option<f64>,
    pub error: Option<String>,
}

pub async fn test_connection(host: &ResolumeHost) -> anyhow::Result<TestConnectionResult> {
    use std::time::Instant;

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| anyhow!("failed to build test client: {e}"))?;

    let url = format!("http://{}:{}/api/v1/composition", host.host, host.port);
    let start = Instant::now();
    match client
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => Ok(TestConnectionResult {
            success: true,
            latency_ms: Some(start.elapsed().as_secs_f64() * 1000.0),
            error: None,
        }),
        Ok(response) => Ok(TestConnectionResult {
            success: false,
            latency_ms: Some(start.elapsed().as_secs_f64() * 1000.0),
            error: Some(format!("HTTP {}", response.status())),
        }),
        Err(err) => Ok(TestConnectionResult {
            success: false,
            latency_ms: None,
            error: Some(err.to_string()),
        }),
    }
}

#[cfg(test)]
mod tests;
