use super::clip_map::ClipMapping;
use super::types::{ClipTarget, ResolvedEndpoint, SlotState};
use super::{
    BibleUpdate, ResolumeConnectionSnapshot, ResolumeConnectionState, StageUpdate, TimerFrame,
};
use anyhow::{anyhow, Context};
use chrono::Utc;
use futures_util::{stream::FuturesUnordered, StreamExt};
use presenter_core::ResolumeHost;
use presenter_persistence::ResolumePushAuditEntry;
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
const RESOLUTION_TTL: Duration = Duration::from_secs(300);
const COMPOSITION_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const ACTION_TIMEOUT: Duration = Duration::from_secs(2);
/// Spacing before the FIRST retry after a host enters `Error` (#484).
const BACKOFF_BASE: Duration = Duration::from_secs(1);
/// Backoff ceiling — a persistently-down host retries at most ~once per minute
/// instead of on every push + every 10 s mapping tick (#484).
const BACKOFF_CAP: Duration = Duration::from_secs(60);

/// Minimum spacing before the next retry for a host with `consecutive_failures`
/// recorded failures (1-based). Exponential (1 s, 2 s, 4 s, …) capped at
/// `BACKOFF_CAP`, so a down host stops hammering every push + every 10 s tick.
///
/// Pure + deterministic so the schedule is unit-tested without sleeping (#484).
pub(super) fn backoff_interval(consecutive_failures: u32) -> Duration {
    if consecutive_failures == 0 {
        return Duration::ZERO;
    }
    // 2^(n-1) seconds, saturating, then capped. Shift clamped well under 64.
    let shift = consecutive_failures.saturating_sub(1).min(32);
    let secs = BACKOFF_BASE.as_secs().saturating_mul(1u64 << shift);
    Duration::from_secs(secs).min(BACKOFF_CAP)
}

/// Whether `record_error` should emit the ERROR-level log line for the failure
/// with this 1-based `consecutive_failures` count. Logs on the first failure
/// (the transition into `Error`) and then only at power-of-two milestones, so a
/// host that fails N times in a row produces ~log2(N)+1 ERROR lines instead of
/// N — the #484 incident saw 163,943 identical lines from one down host.
pub(super) fn should_log_error(consecutive_failures: u32) -> bool {
    consecutive_failures > 0 && consecutive_failures.is_power_of_two()
}

/// Why `refresh_mapping` ran — logged on every composition fetch so a refetch
/// storm (e.g. an error loop) is distinguishable from the normal 10 s
/// background refresh (#483 logging requirement).
#[derive(Debug, Clone, Copy)]
pub(super) enum FetchReason {
    /// No mapping cached yet (cold start or config change).
    Missing,
    /// A prior push/refresh errored and invalidated the cached mapping.
    ErrorInvalidated,
    /// The periodic background refresh (every `MAPPING_REFRESH_INTERVAL`).
    BackgroundTimer,
}

impl FetchReason {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::ErrorInvalidated => "error-invalidated",
            Self::BackgroundTimer => "background-timer",
        }
    }
}

/// Convert a `Duration` to milliseconds (telemetry only). Pure + deterministic
/// so a unit test can pin the conversion (keeps the `* 1000.0` honest under the
/// mutation gate rather than living untested inside the timing path).
pub(super) fn duration_ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Result of `ensure_mapping`: whether the call had to fetch the composition
/// inline (true only on a cold/invalidated cache) — recorded in the push audit.
#[derive(Debug, Clone, Copy)]
pub(super) struct MappingFetchOutcome {
    pub refetched: bool,
}

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
    audit_tx: Option<mpsc::Sender<ResolumePushAuditEntry>>,
) -> anyhow::Result<()> {
    let mut driver = HostDriver::new(client, host.clone());
    driver.audit_tx = audit_tx;
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
                if driver.in_backoff() {
                    // #484: a down host is in its backoff window — skip this
                    // 10 s refresh instead of re-attempting (and re-logging).
                    debug!(host = %driver.config.host, "resolume host in backoff; skipping mapping refresh");
                } else if let Err(err) = driver.refresh_mapping().await {
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
    /// When the cached mapping was last fetched. After #483 this is no longer a
    /// staleness trigger on the push path — it is read only to log the served
    /// mapping's age (`mapping_age_ms`), the diagnostic the issue cares about.
    pub(super) last_mapping_refresh: Option<Instant>,
    /// True when the cache was cleared by `record_error` (vs a cold start), so
    /// the next inline fetch is logged as `error-invalidated`, not `missing`.
    pub(super) mapping_cleared_by_error: bool,
    /// #484: when the next retry is allowed while the host is in `Error`. While
    /// `Instant::now()` is before this, pushes and the 10 s mapping tick are
    /// skipped (exponential backoff keyed on `consecutive_failures`). `None`
    /// when the host is healthy.
    pub(super) next_retry_at: Option<Instant>,
    pub(super) last_timer_payload: Option<String>,
    pub(super) last_song_name_payload: Option<String>,
    pub(super) last_band_name_payload: Option<String>,
    /// Non-blocking sink for per-push audit rows (#483). `None` in unit tests
    /// and whenever no DB-backed writer is wired. Sent via `try_send` so a full
    /// channel drops the audit row rather than ever blocking the push.
    pub(super) audit_tx: Option<mpsc::Sender<ResolumePushAuditEntry>>,
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
            mapping_cleared_by_error: false,
            next_retry_at: None,
            last_timer_payload: None,
            last_song_name_payload: None,
            last_band_name_payload: None,
            audit_tx: None,
        }
    }

    pub(super) fn update_config(&mut self, config: ResolumeHost) {
        self.config = config;
        self.mapping = None;
        self.lane_state = SlotState::default();
        self.endpoint = None;
        self.last_mapping_refresh = None;
        self.mapping_cleared_by_error = false;
        self.next_retry_at = None;
        self.last_timer_payload = None;
        self.last_song_name_payload = None;
        self.last_band_name_payload = None;
    }

    /// #484: true while the host is within its post-error backoff window — the
    /// next retry is not yet due, so the worker skips this push / mapping tick.
    pub(super) fn in_backoff(&self) -> bool {
        matches!(self.next_retry_at, Some(at) if Instant::now() < at)
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

    /// Ensure a clip-mapping is available for the push path.
    ///
    /// #483: the push path is served from cache and is NEVER re-fetched inline
    /// on staleness. The only inline fetch is when there is no mapping at all
    /// (cold start, config change, or an error that invalidated it). The 10 s
    /// background timer (`run_host_worker`) and on-error invalidation are the
    /// only refresh triggers, so a lyric line is never blocked on a 300–620 ms
    /// composition fetch.
    pub(super) async fn ensure_mapping(&mut self) -> anyhow::Result<MappingFetchOutcome> {
        if !self.config.is_enabled {
            self.mapping = None;
            return Ok(MappingFetchOutcome { refetched: false });
        }
        if self.mapping.is_none() {
            let reason = if self.mapping_cleared_by_error {
                FetchReason::ErrorInvalidated
            } else {
                FetchReason::Missing
            };
            debug!(
                target: "presenter::resolume::timing",
                host = %self.config.host,
                mapping_cache = "miss",
                reason = reason.as_str(),
                "resolume mapping cache miss — fetching composition inline"
            );
            self.refresh_mapping_with_reason(reason).await?;
            self.mapping_cleared_by_error = false;
            return Ok(MappingFetchOutcome { refetched: true });
        }

        let mapping_age = self.last_mapping_refresh.map(|instant| instant.elapsed());
        debug!(
            target: "presenter::resolume::timing",
            host = %self.config.host,
            mapping_cache = "hit",
            mapping_age = ?mapping_age,
            "resolume mapping cache hit — serving cached mapping"
        );
        Ok(MappingFetchOutcome { refetched: false })
    }

    /// Periodic background refresh entry point (the `mapping_timer` tick and the
    /// direct test calls). Inline push-path fetches go through
    /// `refresh_mapping_with_reason` with their specific reason.
    pub(super) async fn refresh_mapping(&mut self) -> anyhow::Result<()> {
        self.refresh_mapping_with_reason(FetchReason::BackgroundTimer)
            .await
    }

    pub(super) async fn refresh_mapping_with_reason(
        &mut self,
        reason: FetchReason,
    ) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            self.mapping = None;
            return Ok(());
        }
        let endpoint = self.endpoint().await?;
        let url = format!("{}/composition", endpoint.base_url);
        let fetch_start = Instant::now();
        let response = self
            .apply_host_header(self.client.get(&url), &endpoint)
            .timeout(COMPOSITION_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("failed to fetch composition from {}", url))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("failed to read composition body from {}", url))?;
        let fetch_ms = duration_ms(fetch_start.elapsed());
        if !status.is_success() {
            return Err(anyhow!("composition request failed with status {status}"));
        }
        let parse_start = Instant::now();
        let body: serde_json::Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid composition JSON from {}", url))?;
        let mapping = ClipMapping::from_composition(&body)?;
        let parse_ms = duration_ms(parse_start.elapsed());
        let clip_count = count_clips(&body);

        // #483: a dedicated line so "we re-fetch a huge composition" is a single
        // grep. `reason` distinguishes a background refresh from an error- or
        // cold-start-driven inline fetch.
        tracing::info!(
            target: "presenter::resolume::timing",
            host = %self.config.host,
            reason = reason.as_str(),
            bytes = bytes.len(),
            fetch_ms,
            parse_ms,
            clip_count,
            "resolume composition fetched"
        );

        let missing = mapping.missing_tokens().to_vec();
        if !missing.is_empty() {
            tracing::warn!(
                host = %self.config.host,
                missing = ?missing,
                "Resolume mapping missing expected clips"
            );
        }

        // #267: only reset dedup state when the #timer param IDs actually
        // changed. A network blip that does not change the mapping must
        // preserve last_timer_payload so the next equal-valued tick is
        // skipped (no flicker).
        let timer_param_ids_changed = self
            .mapping
            .as_ref()
            .map(|old| old.timer_param_ids() != mapping.timer_param_ids())
            .unwrap_or(true);
        if timer_param_ids_changed {
            self.last_timer_payload = None;
        }

        self.mapping = Some(mapping);
        self.last_mapping_refresh = Some(Instant::now());
        Ok(())
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
                    .timeout(ACTION_TIMEOUT)
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

    pub(super) async fn mark_connected(
        &mut self,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        let recovered_from = {
            let mut guard = status.write().await;
            let prior_failures = guard.consecutive_failures;
            let was_in_error = prior_failures > 0 || guard.state == ResolumeConnectionState::Error;
            guard.state = ResolumeConnectionState::Connected;
            let now = Utc::now();
            guard.last_success = Some(now);
            guard.last_attempt = Some(now);
            guard.last_error = None;
            guard.consecutive_failures = 0;
            guard.error_since = None;
            was_in_error.then_some(prior_failures)
        };
        // #484: clear the backoff window on recovery and log the state change
        // ONCE (symmetric with the error-transition log), so a recovery is
        // greppable without per-attempt spam.
        self.next_retry_at = None;
        if let Some(prior_failures) = recovered_from {
            tracing::info!(
                host = %self.config.host,
                recovered_after_failures = prior_failures,
                "resolume host recovered"
            );
        }
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
        let failures = {
            let mut guard = status.write().await;
            let now = Utc::now();
            if guard.state != ResolumeConnectionState::Error {
                guard.error_since = Some(now);
            }
            guard.state = ResolumeConnectionState::Error;
            guard.last_error = Some(err.to_string());
            guard.consecutive_failures = guard.consecutive_failures.saturating_add(1);
            guard.last_attempt = Some(now);
            guard.consecutive_failures
        };

        // #484: open/extend the backoff window so a persistently-down host stops
        // retrying on every push + every 10 s tick. Spacing grows with the
        // failure count and caps at ~1/min.
        self.next_retry_at = Some(Instant::now() + backoff_interval(failures));

        // #484: dedup the ERROR log — emit once on the transition into Error and
        // then only at widening milestones, so a down host produces O(log N)
        // lines, not one per attempt (163,943 in the incident). Suppressed
        // failures keep a DEBUG trace so the detail isn't lost.
        if should_log_error(failures) {
            error!(
                host = %self.config.host,
                consecutive_failures = failures,
                error = ?err,
                "resolume host error"
            );
        } else {
            debug!(
                target: "presenter::resolume",
                host = %self.config.host,
                consecutive_failures = failures,
                error = ?err,
                "resolume host error (suppressed; backing off)"
            );
        }

        // #267: preserve last_timer_payload, last_song_name_payload,
        // last_band_name_payload across transient errors. They will only
        // be reset when refresh_mapping detects a real param-ID change.
        // The mapping itself is invalidated so the next operation re-fetches.
        self.mapping = None;
        self.endpoint = None;
        self.last_mapping_refresh = None;
        // #483: the next inline fetch is error-driven, not a cold start.
        self.mapping_cleared_by_error = true;
    }
}

/// Total number of clips across all layers in a Resolume `/composition` body —
/// the composition "size" logged on every fetch (#483).
pub(super) fn count_clips(body: &serde_json::Value) -> usize {
    body.get("layers")
        .and_then(|layers| layers.as_array())
        .map(|layers| {
            layers
                .iter()
                .map(|layer| {
                    layer
                        .get("clips")
                        .and_then(|clips| clips.as_array())
                        .map(|clips| clips.len())
                        .unwrap_or(0)
                })
                .sum()
        })
        .unwrap_or(0)
}
