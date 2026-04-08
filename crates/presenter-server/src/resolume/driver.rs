use super::clip_map::ClipMapping;
use super::types::{apply_transforms, ClipTarget, ResolvedEndpoint, SlotState};
use super::{
    BibleUpdate, ResolumeConnectionSnapshot, ResolumeConnectionState, StageUpdate, TimerFrame,
};
use anyhow::{anyhow, Context};
use chrono::Utc;
use futures_util::{stream::FuturesUnordered, StreamExt};
use presenter_core::ResolumeHost;
use reqwest::{header::HOST, Client, RequestBuilder};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::lookup_host,
    sync::{mpsc, RwLock},
    time::Instant,
};
use tracing::{debug, error};

#[cfg(not(test))]
pub(super) const TRIGGER_DELAY: Duration = Duration::from_millis(35);
#[cfg(test)]
pub(super) const TRIGGER_DELAY: Duration = Duration::from_millis(0);
const MAPPING_REFRESH_INTERVAL: Duration = Duration::from_secs(10);
const MAPPING_CACHE_TTL: Duration = Duration::from_secs(1);
const RESOLUTION_TTL: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub(super) enum HostCommand {
    Stage(StageUpdate),
    Bible(BibleUpdate),
    Timer(TimerFrame),
    RefreshConfig(ResolumeHost),
    Shutdown,
}

pub(super) async fn run_host_worker(
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
                    Some(HostCommand::Timer(frame)) => {
                        if let Err(err) = driver.handle_timer(frame, &status).await {
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
pub(super) struct HostDriver {
    pub(super) client: Client,
    pub(super) config: ResolumeHost,
    pub(super) mapping: Option<ClipMapping>,
    pub(super) lane_state: SlotState,
    pub(super) endpoint: Option<ResolvedEndpoint>,
    pub(super) last_mapping_refresh: Option<Instant>,
    pub(super) last_timer_payload: Option<String>,
    pub(super) last_song_name_payload: Option<String>,
    pub(super) last_band_name_payload: Option<String>,
}

impl HostDriver {
    pub(super) fn new(client: Client, config: ResolumeHost) -> Self {
        Self {
            client,
            config,
            mapping: None,
            lane_state: SlotState::default(),
            endpoint: None,
            last_mapping_refresh: None,
            last_timer_payload: None,
            last_song_name_payload: None,
            last_band_name_payload: None,
        }
    }

    pub(super) fn update_config(&mut self, config: ResolumeHost) {
        self.config = config;
        self.mapping = None;
        self.lane_state = SlotState::default();
        self.endpoint = None;
        self.last_mapping_refresh = None;
        self.last_timer_payload = None;
        self.last_song_name_payload = None;
        self.last_band_name_payload = None;
    }

    pub(super) async fn refresh_status(&self, status: &Arc<RwLock<ResolumeConnectionSnapshot>>) {
        let mut guard = status.write().await;
        if self.config.is_enabled {
            guard.state = ResolumeConnectionState::Connecting;
            guard.last_error = None;
        } else {
            *guard = ResolumeConnectionSnapshot::disabled();
        }
    }

    pub(super) async fn ensure_mapping(&mut self) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            self.mapping = None;
            return Ok(());
        }
        if self.mapping.is_none() {
            self.refresh_mapping().await?;
            return Ok(());
        }

        let stale = self
            .last_mapping_refresh
            .map(|instant| instant.elapsed() >= MAPPING_CACHE_TTL)
            .unwrap_or(true);
        if stale {
            self.refresh_mapping().await?;
        }
        Ok(())
    }

    pub(super) async fn refresh_mapping(&mut self) -> anyhow::Result<()> {
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
            tracing::warn!(
                host = %self.config.host,
                missing = ?missing,
                "Resolume mapping missing expected clips"
            );
        }
        self.mapping = Some(mapping);
        self.last_mapping_refresh = Some(Instant::now());
        self.last_timer_payload = None;
        Ok(())
    }

    pub(super) async fn update_clip_text(
        &self,
        target: &ClipTarget,
        text: &str,
        endpoint: &ResolvedEndpoint,
    ) -> anyhow::Result<Option<Duration>> {
        let Some(param_id) = target.text_param_id else {
            return Ok(None);
        };
        let url = format!("{}/parameter/by-id/{}", endpoint.base_url, param_id);
        let payload = apply_transforms(text, &target.transforms);
        debug!(clip_id = target.clip_id, payload = %payload, "resolume.update_text");
        let start = Instant::now();
        let response = self
            .apply_host_header(self.client.put(&url), endpoint)
            .json(&serde_json::json!({ "value": payload.as_ref() }))
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

    pub(super) async fn trigger_clips(&mut self, targets: &[ClipTarget]) -> anyhow::Result<()> {
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
            debug!(clip_id, "resolume.trigger_clip");

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

    pub(super) async fn endpoint(&mut self) -> anyhow::Result<ResolvedEndpoint> {
        if let Some(endpoint) = &self.endpoint {
            if endpoint.resolved_at.elapsed() < RESOLUTION_TTL {
                return Ok(endpoint.clone());
            }
        }
        let resolved = self.resolve_endpoint().await?;
        self.endpoint = Some(resolved.clone());
        Ok(resolved)
    }

    pub(super) async fn resolve_endpoint(&self) -> anyhow::Result<ResolvedEndpoint> {
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

    pub(super) fn apply_host_header(
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

    pub(super) async fn mark_connected(&self, status: &Arc<RwLock<ResolumeConnectionSnapshot>>) {
        let mut guard = status.write().await;
        guard.state = ResolumeConnectionState::Connected;
        let now = Utc::now();
        guard.last_success = Some(now);
        guard.last_attempt = Some(now);
        guard.last_error = None;
        guard.consecutive_failures = 0;
        guard.error_since = None;
    }

    pub(super) async fn note_latency(
        &self,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
        latency: Duration,
    ) {
        let mut guard = status.write().await;
        guard.last_latency_ms = Some(latency.as_secs_f64() * 1000.0);
    }

    pub(super) async fn record_error(
        &mut self,
        err: anyhow::Error,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        error!(host = %self.config.host, error = ?err, "resolume host error");
        let mut guard = status.write().await;
        let now = Utc::now();
        if guard.state != ResolumeConnectionState::Error {
            guard.error_since = Some(now);
        }
        guard.state = ResolumeConnectionState::Error;
        guard.last_error = Some(err.to_string());
        guard.consecutive_failures += 1;
        guard.last_attempt = Some(now);
        self.mapping = None;
        self.endpoint = None;
        self.last_mapping_refresh = None;
        self.last_timer_payload = None;
    }
}
