use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use futures_util::{stream::FuturesUnordered, StreamExt};
use presenter_core::{BibleBroadcast, ResolumeHost, ResolumeHostId};
use reqwest::{header::HOST, Client, RequestBuilder};
use serde::Serialize;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::sync::RwLock;
use tokio::{
    net::lookup_host,
    sync::mpsc,
    task::JoinHandle,
    time::{sleep, Instant},
};
use tracing::{debug, error, warn};

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(not(test))]
const TRIGGER_DELAY: Duration = Duration::from_millis(35);
#[cfg(test)]
const TRIGGER_DELAY: Duration = Duration::from_millis(0);
const HOST_COMMAND_CAPACITY: usize = 16;
const MAPPING_REFRESH_INTERVAL: Duration = Duration::from_secs(10);
const RESOLUTION_TTL: Duration = Duration::from_secs(300);

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
}

impl ResolumeConnectionSnapshot {
    pub fn disabled() -> Self {
        Self {
            state: ResolumeConnectionState::Disabled,
            last_success: None,
            last_latency_ms: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StageUpdate {
    pub current_main: Option<String>,
    pub current_translation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BibleUpdate {
    pub passage: Option<BibleBroadcast>,
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

#[derive(Debug)]
enum HostCommand {
    Stage(StageUpdate),
    Bible(BibleUpdate),
    RefreshConfig(ResolumeHost),
    Shutdown,
}

use clip_map::ClipMapping;

impl ResolumeRegistry {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("failed to build reqwest client");
        Self {
            client,
            hosts: Arc::new(RwLock::new(HashMap::new())),
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
                    let _ = entry.command_tx.try_send(HostCommand::Shutdown);
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
                            .try_send(HostCommand::RefreshConfig(host.clone()));
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

    pub async fn stage_update(&self, update: StageUpdate) {
        for entry in self.hosts.read().await.values() {
            let _ = entry
                .command_tx
                .try_send(HostCommand::Stage(update.clone()));
        }
    }

    pub async fn bible_update(&self, update: BibleUpdate) {
        for entry in self.hosts.read().await.values() {
            let _ = entry
                .command_tx
                .try_send(HostCommand::Bible(update.clone()));
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

async fn run_host_worker(
    client: Client,
    mut host: ResolumeHost,
    status: Arc<RwLock<ResolumeConnectionSnapshot>>,
    mut commands: mpsc::Receiver<HostCommand>,
) -> anyhow::Result<()> {
    let mut driver = HostDriver::new(client, host.clone());
    driver.refresh_status(&status).await;

    let mut mapping_timer = tokio::time::interval(MAPPING_REFRESH_INTERVAL);
    loop {
        tokio::select! {
            maybe_cmd = commands.recv() => {
                match maybe_cmd {
                    Some(HostCommand::Stage(payload)) => {
                        if let Err(err) = driver.handle_stage(payload, &status).await {
                            driver.record_error(err, &status).await;
                        }
                    }
                    Some(HostCommand::Bible(payload)) => {
                        if let Err(err) = driver.handle_bible(payload, &status).await {
                            driver.record_error(err, &status).await;
                        }
                    }
                    Some(HostCommand::RefreshConfig(new_config)) => {
                        host = new_config.clone();
                        driver.update_config(new_config);
                        driver.refresh_status(&status).await;
                    }
                    Some(HostCommand::Shutdown) | None => {
                        debug!(host_id = %host.id, "resolume host worker shutting down");
                        break;
                    }
                }
            }
            _ = mapping_timer.tick() => {
                if let Err(err) = driver.refresh_mapping().await {
                    driver.record_error(err, &status).await;
                } else {
                    driver.mark_connected(&status).await;
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
struct HostDriver {
    client: Client,
    config: ResolumeHost,
    mapping: Option<ClipMapping>,
    lane_state: SlotState,
    endpoint: Option<ResolvedEndpoint>,
}

impl HostDriver {
    fn new(client: Client, config: ResolumeHost) -> Self {
        Self {
            client,
            config,
            mapping: None,
            lane_state: SlotState::default(),
            endpoint: None,
        }
    }

    fn update_config(&mut self, config: ResolumeHost) {
        self.config = config;
        self.mapping = None;
        self.lane_state = SlotState::default();
        self.endpoint = None;
    }

    async fn refresh_status(&self, status: &Arc<RwLock<ResolumeConnectionSnapshot>>) {
        let mut guard = status.write().await;
        if self.config.is_enabled {
            guard.state = ResolumeConnectionState::Connecting;
            guard.last_error = None;
        } else {
            *guard = ResolumeConnectionSnapshot::disabled();
        }
    }

    async fn handle_stage(
        &mut self,
        update: StageUpdate,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            return Ok(());
        }
        self.ensure_mapping().await?;
        if let Some(mapping) = self.mapping.clone() {
            let main_lane = self.lane_state.current(SlotKind::Main);
            let translation_lane = self.lane_state.current(SlotKind::Translation);

            let mut to_trigger = Vec::new();
            let mut main_lane_filled = false;
            if let Some(ref main_text) = update.current_main {
                let mut main_targets = self
                    .update_lane_text(
                        main_lane,
                        &mapping.main_a,
                        &mapping.main_b,
                        Some(main_text),
                        status,
                    )
                    .await?;
                if !main_targets.is_empty() {
                    to_trigger.append(&mut main_targets);
                    main_lane_filled = true;
                }
            }

            let mut translation_lane_filled = false;
            if let Some(ref translation_text) = update.current_translation {
                let mut translation_targets = self
                    .update_lane_text(
                        translation_lane,
                        &mapping.translation_a,
                        &mapping.translation_b,
                        Some(translation_text),
                        status,
                    )
                    .await?;
                if !translation_targets.is_empty() {
                    to_trigger.append(&mut translation_targets);
                    translation_lane_filled = true;
                }
            }

            if !to_trigger.is_empty() {
                if TRIGGER_DELAY.as_millis() > 0 {
                    sleep(TRIGGER_DELAY).await;
                }
                self.trigger_clips(&to_trigger).await?;
            }

            if main_lane_filled {
                self.lane_state.flip(SlotKind::Main);
                if !translation_lane_filled
                    && !mapping.translation_a.is_empty()
                    && !mapping.translation_b.is_empty()
                {
                    self.lane_state.flip(SlotKind::Translation);
                }
            }

            if translation_lane_filled {
                self.lane_state.flip(SlotKind::Translation);
            }
        }
        self.mark_connected(status).await;
        Ok(())
    }

    async fn handle_bible(
        &mut self,
        update: BibleUpdate,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            return Ok(());
        }
        self.ensure_mapping().await?;
        if let Some(mapping) = self.mapping.clone() {
            let bible_lane = self.lane_state.current(SlotKind::Bible);
            let bible_translation_lane = self.lane_state.current(SlotKind::BibleTranslation);
            let mut to_trigger = Vec::new();
            let (bible_lane_filled, bible_translation_lane_filled) = match update.passage {
                Some(ref passage) => {
                    let verse_text = passage.passage.text.clone();
                    let translation = passage.passage.translation.name.clone();
                    let reference = passage.passage.reference.to_human_readable();
                    let combined = format!("{}\n{}", reference, verse_text);
                    let bible_targets = self
                        .update_lane_text(
                            bible_lane,
                            &mapping.bible_a,
                            &mapping.bible_b,
                            Some(&combined),
                            status,
                        )
                        .await?;
                    let bible_lane_filled = !bible_targets.is_empty();
                    if bible_lane_filled {
                        to_trigger.extend(bible_targets.into_iter());
                    }
                    let bible_translation_targets = self
                        .update_lane_text(
                            bible_translation_lane,
                            &mapping.bible_translation_a,
                            &mapping.bible_translation_b,
                            Some(&translation),
                            status,
                        )
                        .await?;
                    let bible_translation_lane_filled = !bible_translation_targets.is_empty();
                    if bible_translation_lane_filled {
                        to_trigger.extend(bible_translation_targets.into_iter());
                    }
                    (bible_lane_filled, bible_translation_lane_filled)
                }
                None => {
                    let blank = String::new();
                    let bible_targets = self
                        .update_lane_text(
                            bible_lane,
                            &mapping.bible_a,
                            &mapping.bible_b,
                            Some(&blank),
                            status,
                        )
                        .await?;
                    let bible_lane_filled = !bible_targets.is_empty();
                    if bible_lane_filled {
                        to_trigger.extend(bible_targets.into_iter());
                    }
                    let bible_translation_targets = self
                        .update_lane_text(
                            bible_translation_lane,
                            &mapping.bible_translation_a,
                            &mapping.bible_translation_b,
                            Some(&blank),
                            status,
                        )
                        .await?;
                    let bible_translation_lane_filled = !bible_translation_targets.is_empty();
                    if bible_translation_lane_filled {
                        to_trigger.extend(bible_translation_targets.into_iter());
                    }
                    to_trigger.extend(mapping.bible_clear.iter().cloned());
                    (bible_lane_filled, bible_translation_lane_filled)
                }
            };

            if !to_trigger.is_empty() {
                if TRIGGER_DELAY.as_millis() > 0 {
                    sleep(TRIGGER_DELAY).await;
                }
                self.trigger_clips(&to_trigger).await?;
            }
            if bible_lane_filled {
                self.lane_state.flip(SlotKind::Bible);
            }
            if bible_translation_lane_filled {
                self.lane_state.flip(SlotKind::BibleTranslation);
            }
        }
        self.mark_connected(status).await;
        Ok(())
    }

    async fn ensure_mapping(&mut self) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            self.mapping = None;
            return Ok(());
        }
        if self.mapping.is_none() {
            self.refresh_mapping().await?;
        }
        Ok(())
    }

    async fn refresh_mapping(&mut self) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            self.mapping = None;
            return Ok(());
        }
        let endpoint = self.endpoint().await?;
        let url = format!("{}/composition", endpoint.base_url);
        let response = self
            .apply_host_header(self.client.get(&url), &endpoint)
            .send()
            .await
            .with_context(|| format!("failed to fetch composition from {}", url))?;
        let status = response.status();
        let body = response
            .json::<serde_json::Value>()
            .await
            .with_context(|| format!("invalid composition JSON from {}", url))?;
        if !status.is_success() {
            return Err(anyhow!("composition request failed with status {status}"));
        }
        let mapping = ClipMapping::from_composition(&body)?;
        let missing = mapping.missing_tokens().to_vec();
        if !missing.is_empty() {
            let joined = missing.join(", ");
            warn!(
                host = %self.config.host,
                "Resolume mapping missing expected clips: {joined}"
            );
            return Err(anyhow!("missing Resolume clips: {}", joined));
        }
        let previous = self.mapping.clone();
        let changed = previous.as_ref().map(|old| old != &mapping).unwrap_or(true);
        self.mapping = Some(mapping);
        if changed {
            self.lane_state = SlotState::default();
        }
        Ok(())
    }

    async fn update_lane_text(
        &mut self,
        lane: LaneTarget,
        lane_a: &[ClipTarget],
        lane_b: &[ClipTarget],
        text: Option<&String>,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<Vec<ClipTarget>> {
        let (primary, alternate) = select_lane_targets(lane, lane_a, lane_b);
        if primary.is_empty() {
            if !alternate.is_empty() {
                warn!(
                    host = %self.config.host,
                    port = self.config.port,
                    lane = %lane.label(),
                    "Resolume lane missing clips; skipping update"
                );
            } else {
                warn!(
                    host = %self.config.host,
                    port = self.config.port,
                    lane = %lane.label(),
                    "Resolume has no clips configured for lane"
                );
            }
            return Ok(Vec::new());
        }
        let selected = primary;

        if let Some(payload) = text {
            let endpoint = self.endpoint().await?;
            let mut latency_recorded = None;
            for target in selected {
                if let Some(duration) = self.update_clip_text(target, payload, &endpoint).await? {
                    if latency_recorded.is_none() {
                        latency_recorded = Some(duration);
                    }
                }
            }
            if let Some(latency) = latency_recorded {
                self.note_latency(status, latency).await;
            }
        }

        Ok(selected.to_vec())
    }

    async fn update_clip_text(
        &self,
        target: &ClipTarget,
        text: &str,
        endpoint: &ResolvedEndpoint,
    ) -> anyhow::Result<Option<Duration>> {
        let Some(param_id) = target.text_param_id else {
            return Ok(None);
        };
        let url = format!("{}/parameter/by-id/{}", endpoint.base_url, param_id);
        let start = Instant::now();
        let response = self
            .apply_host_header(self.client.put(&url), endpoint)
            .json(&serde_json::json!({ "value": text }))
            .send()
            .await
            .with_context(|| format!("failed to update text parameter {}", param_id))?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "text parameter update failed with status {}",
                response.status()
            ));
        }
        Ok(Some(start.elapsed()))
    }

    async fn trigger_clips(&mut self, targets: &[ClipTarget]) -> anyhow::Result<()> {
        if targets.is_empty() {
            return Ok(());
        }

        let endpoint = self.endpoint().await?;
        let mut futures = FuturesUnordered::new();

        for target in targets {
            let client = self.client.clone();
            let clip_id = target.clip_id;
            let url = format!(
                "{}/composition/clips/by-id/{}/connect",
                &endpoint.base_url, clip_id
            );
            let host_header = endpoint.host_header.clone();

            futures.push(async move {
                let mut request = client.post(&url);
                if let Some(host) = host_header {
                    request = request.header(HOST, host);
                }
                let response = request
                    .send()
                    .await
                    .with_context(|| format!("failed to trigger clip {}", clip_id))?;
                if !response.status().is_success() {
                    Err(anyhow!(
                        "clip trigger failed with status {}",
                        response.status()
                    ))
                } else {
                    Ok(())
                }
            });
        }

        while let Some(result) = futures.next().await {
            result?;
        }

        Ok(())
    }

    async fn endpoint(&mut self) -> anyhow::Result<ResolvedEndpoint> {
        if let Some(endpoint) = &self.endpoint {
            if endpoint.resolved_at.elapsed() < RESOLUTION_TTL {
                return Ok(endpoint.clone());
            }
        }
        let resolved = self.resolve_endpoint().await?;
        self.endpoint = Some(resolved.clone());
        Ok(resolved)
    }

    async fn resolve_endpoint(&self) -> anyhow::Result<ResolvedEndpoint> {
        let host = self.config.host.trim();
        if host.is_empty() {
            return Err(anyhow!("Resolume host cannot be empty"));
        }
        let port = self.config.port;

        if host.parse::<IpAddr>().is_ok() {
            let base_url = format!("http://{}:{}/api/v1", host, port);
            return Ok(ResolvedEndpoint::new(base_url, None));
        }

        let mut candidates: Vec<SocketAddr> = lookup_host((host, port))
            .await
            .with_context(|| format!("failed to resolve Resolume host {host}"))?
            .collect();

        if candidates.is_empty() {
            return Err(anyhow!("no socket addresses resolved for {host}"));
        }

        candidates.sort_by(|a, b| match (a, b) {
            (SocketAddr::V4(_), SocketAddr::V6(_)) => std::cmp::Ordering::Less,
            (SocketAddr::V6(_), SocketAddr::V4(_)) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        });

        let addr = candidates[0];
        let ip = match addr.ip() {
            IpAddr::V4(v4) => v4.to_string(),
            IpAddr::V6(v6) => format!("[{}]", v6),
        };
        let base_url = format!("http://{}:{}/api/v1", ip, addr.port());
        Ok(ResolvedEndpoint::new(base_url, Some(host.to_string())))
    }

    fn apply_host_header(
        &self,
        builder: RequestBuilder,
        endpoint: &ResolvedEndpoint,
    ) -> RequestBuilder {
        if let Some(host) = &endpoint.host_header {
            builder.header(HOST, host.clone())
        } else {
            builder
        }
    }

    async fn mark_connected(&self, status: &Arc<RwLock<ResolumeConnectionSnapshot>>) {
        let mut guard = status.write().await;
        guard.state = ResolumeConnectionState::Connected;
        guard.last_success = Some(Utc::now());
        guard.last_error = None;
    }

    async fn note_latency(
        &self,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
        latency: Duration,
    ) {
        let mut guard = status.write().await;
        guard.last_latency_ms = Some(latency.as_secs_f64() * 1000.0);
    }

    async fn record_error(
        &mut self,
        err: anyhow::Error,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        error!(host = %self.config.host, error = ?err, "resolume host error");
        let mut guard = status.write().await;
        guard.state = ResolumeConnectionState::Error;
        guard.last_error = Some(err.to_string());
        self.mapping = None;
        self.endpoint = None;
    }
}

#[derive(Debug, Clone)]
struct ResolvedEndpoint {
    base_url: String,
    host_header: Option<String>,
    resolved_at: Instant,
}

impl ResolvedEndpoint {
    fn new(base_url: String, host_header: Option<String>) -> Self {
        Self {
            base_url,
            host_header,
            resolved_at: Instant::now(),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct SlotState {
    main_next: LaneTarget,
    translation_next: LaneTarget,
    bible_next: LaneTarget,
    bible_translation_next: LaneTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaneTarget {
    A,
    B,
}

impl Default for LaneTarget {
    fn default() -> Self {
        LaneTarget::A
    }
}

impl LaneTarget {
    fn label(&self) -> &'static str {
        match self {
            LaneTarget::A => "A",
            LaneTarget::B => "B",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SlotKind {
    Main,
    Translation,
    Bible,
    BibleTranslation,
}

impl SlotState {
    fn current(&self, slot: SlotKind) -> LaneTarget {
        match slot {
            SlotKind::Main => self.main_next,
            SlotKind::Translation => self.translation_next,
            SlotKind::Bible => self.bible_next,
            SlotKind::BibleTranslation => self.bible_translation_next,
        }
    }

    fn flip(&mut self, slot: SlotKind) {
        let target = match slot {
            SlotKind::Main => &mut self.main_next,
            SlotKind::Translation => &mut self.translation_next,
            SlotKind::Bible => &mut self.bible_next,
            SlotKind::BibleTranslation => &mut self.bible_translation_next,
        };
        *target = match *target {
            LaneTarget::A => LaneTarget::B,
            LaneTarget::B => LaneTarget::A,
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipTarget {
    pub clip_id: i64,
    pub text_param_id: Option<i64>,
}

fn select_lane_targets<'a>(
    lane: LaneTarget,
    lane_a: &'a [ClipTarget],
    lane_b: &'a [ClipTarget],
) -> (&'a [ClipTarget], &'a [ClipTarget]) {
    match lane {
        LaneTarget::A => (lane_a, lane_b),
        LaneTarget::B => (lane_b, lane_a),
    }
}

mod clip_map {
    use super::ClipTarget;
    use anyhow::{anyhow, Result};
    use serde_json::Value;

    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct ClipMapping {
        pub main_a: Vec<ClipTarget>,
        pub main_b: Vec<ClipTarget>,
        pub translation_a: Vec<ClipTarget>,
        pub translation_b: Vec<ClipTarget>,
        pub bible_a: Vec<ClipTarget>,
        pub bible_b: Vec<ClipTarget>,
        pub bible_translation_a: Vec<ClipTarget>,
        pub bible_translation_b: Vec<ClipTarget>,
        pub bible_clear: Vec<ClipTarget>,
        missing_tokens: Vec<&'static str>,
    }

    impl ClipMapping {
        pub fn from_composition(value: &Value) -> Result<Self> {
            let layers = value
                .get("layers")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("composition has no layers"))?;

            let mut mapping = ClipMapping::default();
            for layer in layers {
                if let Some(clips) = layer.get("clips").and_then(Value::as_array) {
                    for clip in clips {
                        ingest_clip(&mut mapping, clip);
                    }
                }
            }
            mapping.missing_tokens = compute_missing_tokens(&mapping);
            Ok(mapping)
        }

        pub fn missing_tokens(&self) -> &[&'static str] {
            &self.missing_tokens
        }
    }

    fn ingest_clip(mapping: &mut ClipMapping, clip: &Value) {
        let clip_id = clip.get("id").and_then(Value::as_i64);
        let Some(clip_id) = clip_id else {
            return;
        };
        let name = clip
            .get("name")
            .and_then(|v| v.get("value"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let name_lower = name.to_ascii_lowercase();
        let text_param = extract_text_param_id(clip);
        let target = ClipTarget {
            clip_id,
            text_param_id: text_param,
        };

        if name_lower.contains("#main-a") {
            mapping.main_a.push(target.clone());
        }
        if name_lower.contains("#main-b") {
            mapping.main_b.push(target.clone());
        }
        if name_lower.contains("#translate-a") || name_lower.contains("#translation-a") {
            mapping.translation_a.push(target.clone());
        }
        if name_lower.contains("#translate-b") || name_lower.contains("#translation-b") {
            mapping.translation_b.push(target.clone());
        }
        if name_lower.contains("#bible-a") {
            mapping.bible_a.push(target.clone());
        }
        if name_lower.contains("#bible-b") {
            mapping.bible_b.push(target.clone());
        }
        if name_lower.contains("#bible-translate-a") {
            mapping.bible_translation_a.push(target.clone());
        }
        if name_lower.contains("#bible-translate-b") {
            mapping.bible_translation_b.push(target.clone());
        }
        if name_lower.contains("#bible-clear") {
            mapping.bible_clear.push(ClipTarget {
                clip_id,
                text_param_id: None,
            });
        }
    }

    fn compute_missing_tokens(mapping: &ClipMapping) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if mapping.main_a.is_empty() {
            missing.push("#main-a");
        }
        if mapping.main_b.is_empty() {
            missing.push("#main-b");
        }
        if mapping.translation_a.is_empty() {
            missing.push("#translate-a");
        }
        if mapping.translation_b.is_empty() {
            missing.push("#translate-b");
        }
        if mapping.bible_a.is_empty() {
            missing.push("#bible-a");
        }
        if mapping.bible_b.is_empty() {
            missing.push("#bible-b");
        }
        if mapping.bible_translation_a.is_empty() {
            missing.push("#bible-translate-a");
        }
        if mapping.bible_translation_b.is_empty() {
            missing.push("#bible-translate-b");
        }
        if mapping.bible_clear.is_empty() {
            missing.push("#bible-clear");
        }
        missing
    }

    fn extract_text_param_id(clip: &Value) -> Option<i64> {
        let sourceparams = clip.get("video")?.get("sourceparams")?.as_object()?;
        for param in sourceparams.values() {
            let valuetype = param.get("valuetype").and_then(Value::as_str)?;
            if valuetype.eq_ignore_ascii_case("paramtext") {
                if let Some(id) = param.get("id").and_then(Value::as_i64) {
                    return Some(id);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::ResolumeHostId;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_host(host: &str) -> ResolumeHost {
        let now = Utc::now();
        ResolumeHost::new(
            ResolumeHostId::new(),
            "Test Resolume".to_string(),
            host.to_string(),
            8090,
            true,
            now,
            now,
        )
    }

    fn clip(id: i64, name: &str, param_id: Option<i64>) -> serde_json::Value {
        let sourceparams = match param_id {
            Some(value) => serde_json::json!({
                "text": {
                    "valuetype": "ParamText",
                    "id": value,
                }
            }),
            None => serde_json::json!({}),
        };
        serde_json::json!({
            "id": id,
            "name": { "value": name },
            "video": { "sourceparams": sourceparams },
        })
    }

    fn count_requests(requests: &[wiremock::Request], method_name: &str, path_name: &str) -> usize {
        requests
            .iter()
            .filter(|req| req.method.as_str() == method_name && req.url.path() == path_name)
            .count()
    }

    #[tokio::test]
    async fn resolve_endpoint_with_ip_literal_skips_host_header() {
        let host = sample_host("127.0.0.1");
        let driver = HostDriver::new(Client::new(), host);
        let endpoint = driver.resolve_endpoint().await.unwrap();
        assert_eq!(endpoint.base_url, "http://127.0.0.1:8090/api/v1");
        assert!(endpoint.host_header.is_none());
    }

    #[tokio::test]
    async fn resolve_endpoint_with_hostname_sets_header() {
        let host = sample_host("localhost");
        let driver = HostDriver::new(Client::new(), host);
        let endpoint = driver.resolve_endpoint().await.unwrap();
        assert!(
            endpoint.base_url.contains("127.0.0.1:8090")
                || endpoint.base_url.contains("[::1]:8090")
        );
        assert_eq!(endpoint.host_header.as_deref(), Some("localhost"));
    }

    #[tokio::test]
    async fn stage_updates_alternate_main_and_translation_lanes() {
        let server = MockServer::start().await;

        let composition = serde_json::json!({
            "layers": [
                {
                    "clips": [
                        clip(100, "#main-a", Some(1)),
                        clip(101, "#main-b", Some(2)),
                        clip(200, "#translate-a", Some(10)),
                        clip(201, "#translate-b", Some(20)),
                        clip(300, "#bible-a", Some(30)),
                        clip(301, "#bible-b", Some(31)),
                        clip(400, "#bible-translate-a", Some(40)),
                        clip(401, "#bible-translate-b", Some(41)),
                        clip(500, "#bible-clear", None),
                    ],
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/api/v1/composition"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&composition))
            .mount(&server)
            .await;

        for endpoint in &[1, 2, 10, 20, 30, 31, 40, 41] {
            let route = format!("/api/v1/parameter/by-id/{endpoint}");
            Mock::given(method("PUT"))
                .and(path(route.as_str()))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
        }

        for clip_id in &[100, 101, 200, 201, 300, 301, 400, 401, 500] {
            let route = format!("/api/v1/composition/clips/by-id/{clip_id}/connect");
            Mock::given(method("POST"))
                .and(path(route.as_str()))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
        }

        let addr = server.address();
        let host = addr.ip().to_string();
        let port = addr.port();
        let now = Utc::now();
        let config = ResolumeHost::new(
            ResolumeHostId::new(),
            "Mock".into(),
            host,
            port,
            true,
            now,
            now,
        );

        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("client build");
        let mut driver = HostDriver::new(client, config);
        let status = Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled()));

        driver.refresh_status(&status).await;

        let stage_first = StageUpdate {
            current_main: Some("Line 1".to_string()),
            current_translation: Some("Trans 1".to_string()),
        };
        driver
            .handle_stage(stage_first, &status)
            .await
            .expect("first stage");

        let stage_second = StageUpdate {
            current_main: Some("Line 2".to_string()),
            current_translation: Some("Trans 2".to_string()),
        };
        driver
            .handle_stage(stage_second, &status)
            .await
            .expect("second stage");

        let requests = server.received_requests().await.expect("received requests");

        assert_eq!(count_requests(&requests, "GET", "/api/v1/composition"), 1);
        assert_eq!(
            count_requests(&requests, "PUT", "/api/v1/parameter/by-id/1"),
            1
        );
        assert_eq!(
            count_requests(&requests, "PUT", "/api/v1/parameter/by-id/2"),
            1
        );
        assert_eq!(
            count_requests(&requests, "PUT", "/api/v1/parameter/by-id/10"),
            1
        );
        assert_eq!(
            count_requests(&requests, "PUT", "/api/v1/parameter/by-id/20"),
            1
        );

        assert_eq!(
            count_requests(
                &requests,
                "POST",
                "/api/v1/composition/clips/by-id/100/connect",
            ),
            1
        );
        assert_eq!(
            count_requests(
                &requests,
                "POST",
                "/api/v1/composition/clips/by-id/200/connect",
            ),
            1
        );
        assert_eq!(
            count_requests(
                &requests,
                "POST",
                "/api/v1/composition/clips/by-id/101/connect",
            ),
            1
        );
        assert_eq!(
            count_requests(
                &requests,
                "POST",
                "/api/v1/composition/clips/by-id/201/connect",
            ),
            1
        );
    }
}
