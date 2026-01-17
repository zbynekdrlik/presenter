use super::clip_map::ClipMapping;
use super::types::{
    apply_transforms, select_lane_targets, ClipTarget, LaneTarget, ResolvedEndpoint, SlotKind,
    SlotState,
};
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
    time::{sleep, Instant},
};
use tracing::{debug, error, warn};

#[cfg(not(test))]
const TRIGGER_DELAY: Duration = Duration::from_millis(35);
#[cfg(test)]
const TRIGGER_DELAY: Duration = Duration::from_millis(0);
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
    client: Client,
    config: ResolumeHost,
    mapping: Option<ClipMapping>,
    lane_state: SlotState,
    endpoint: Option<ResolvedEndpoint>,
    last_mapping_refresh: Option<Instant>,
    last_timer_payload: Option<String>,
    last_song_name_payload: Option<String>,
    last_band_name_payload: Option<String>,
}

#[derive(Clone, Copy, Debug)]
enum MetadataSlot {
    SongName,
    BandName,
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

    pub(super) async fn handle_stage(
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

            if let Some(ref song_name) = update.song_name {
                if mapping.song_name.is_empty() {
                    warn!(
                        host = %self.config.host,
                        port = self.config.port,
                        "Resolume mapping missing #song-name clip"
                    );
                } else {
                    self.update_metadata_targets(
                        &mapping.song_name,
                        song_name,
                        MetadataSlot::SongName,
                        status,
                    )
                    .await?;
                }
            } else {
                self.last_song_name_payload = None;
            }

            if let Some(ref band_name) = update.band_name {
                if mapping.band_name.is_empty() {
                    warn!(
                        host = %self.config.host,
                        port = self.config.port,
                        "Resolume mapping missing #band-name clip"
                    );
                } else {
                    self.update_metadata_targets(
                        &mapping.band_name,
                        band_name,
                        MetadataSlot::BandName,
                        status,
                    )
                    .await?;
                }
            } else {
                self.last_band_name_payload = None;
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

    pub(super) async fn handle_bible(
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

    pub(super) async fn handle_timer(
        &mut self,
        frame: TimerFrame,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            return Ok(());
        }
        self.ensure_mapping().await?;
        if let Some(mapping) = self.mapping.clone() {
            if mapping.timer.is_empty() {
                warn!(
                    host = %self.config.host,
                    port = self.config.port,
                    "Resolume mapping missing #timer clip"
                );
                return Ok(());
            }

            let text = frame.formatted;
            if self.last_timer_payload.as_deref() == Some(text.as_str()) {
                return Ok(());
            }

            let endpoint = self.endpoint().await?;
            let mut latency_recorded = None;
            for target in &mapping.timer {
                if let Some(duration) = self.update_clip_text(target, &text, &endpoint).await? {
                    if latency_recorded.is_none() {
                        latency_recorded = Some(duration);
                    }
                }
            }
            if let Some(latency) = latency_recorded {
                self.note_latency(status, latency).await;
            }
            self.last_timer_payload = Some(text);
        }
        Ok(())
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
            warn!(
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

    async fn update_metadata_targets(
        &mut self,
        targets: &[ClipTarget],
        text: &str,
        slot: MetadataSlot,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if targets.is_empty() {
            return Ok(());
        }

        let already_sent = match slot {
            MetadataSlot::SongName => self.last_song_name_payload.as_deref() == Some(text),
            MetadataSlot::BandName => self.last_band_name_payload.as_deref() == Some(text),
        };

        if already_sent {
            return Ok(());
        }

        let endpoint = self.endpoint().await?;
        let mut latency_recorded = None;
        for target in targets {
            if let Some(duration) = self.update_clip_text(target, text, &endpoint).await? {
                if latency_recorded.is_none() {
                    latency_recorded = Some(duration);
                }
            }
        }
        if let Some(latency) = latency_recorded {
            self.note_latency(status, latency).await;
        }
        match slot {
            MetadataSlot::SongName => self.last_song_name_payload = Some(text.to_string()),
            MetadataSlot::BandName => self.last_band_name_payload = Some(text.to_string()),
        }
        Ok(())
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

    pub(super) async fn mark_connected(&self, status: &Arc<RwLock<ResolumeConnectionSnapshot>>) {
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

    pub(super) async fn record_error(
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
        self.last_mapping_refresh = None;
        self.last_timer_payload = None;
    }
}
