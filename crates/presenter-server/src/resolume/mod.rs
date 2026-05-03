mod clip_map;
mod driver;
mod handlers;
mod types;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use presenter_core::{BibleBroadcast, BibleSlideOutput, ResolumeHost, ResolumeHostId};
use reqwest::Client;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tracing::error;
use uuid::Uuid;

use driver::{run_host_worker, HostCommand};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const HOST_COMMAND_CAPACITY: usize = 16;

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
    #[allow(dead_code)]
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
        })
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
        let handle = tokio::spawn(async move {
            if let Err(err) = run_host_worker(client, config_clone, status_clone, command_rx).await
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
