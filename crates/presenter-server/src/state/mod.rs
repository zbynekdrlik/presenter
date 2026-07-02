//! Application state management for the presenter server.
//!
//! # Lock Acquisition Policy
//!
//! This module uses several `RwLock` fields for shared state. To prevent deadlocks:
//!
//! 1. **Single lock acquisition**: Each operation acquires at most one lock at a time.
//!    Locks are released before acquiring another lock.
//!
//! 2. **Scoped guards**: Lock guards are always held within explicit scope blocks `{ ... }`
//!    and dropped before performing other async operations or acquiring other locks.
//!
//! 3. **Lock inventory**:
//!    - `bible_broadcast`: Current active Bible passage broadcast
//!    - `presentation_cache`: Cached presentation data for stage display
//!    - `stage_layout`: Selected stage display layout code
//!    - `ableset_cache`: Cached AbleSet library-to-playlist mapping
//!    - `group_color_cache`: Cached group name → hex color mapping
//!
//! If future changes require holding multiple locks, establish and document a consistent
//! acquisition order (e.g., alphabetical by field name) to prevent deadlocks.

mod ableset;
mod api_stage;
pub(crate) mod bible;
mod bible_manager;
mod broadcasting;
mod cache_manager;
mod companion;
mod companion_manager;
mod integrations;
mod ndi_control;
mod osc;
mod presentations;
mod seed;
mod slide_stage_layout;
pub(crate) mod slides;
pub(crate) mod stage;
mod stage_display;
mod stage_state;
#[cfg(test)]
mod tests;
mod timers;

#[cfg(test)]
use crate::config::OscConfig;
use crate::{
    ableset::AbleSetBridge,
    ai::{proxy::ProxyManager, ChatMessage},
    android_stage::AndroidStageRegistry,
    config::ServerConfig,
    live::{LiveEvent, LiveHub},
    osc::OscBridge,
    resolume::ResolumeRegistry,
    stage_connections::{StageConnections, StageHeartbeatConfig},
    turn::TurnService,
};
use chrono::Utc;
use presenter_core::{
    StageClientSnapshot, StageDisplayLayout, TimersOverview, DEFAULT_STAGE_LAYOUT_CODE,
};
use presenter_persistence::{DatabaseSettings, Repository};
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};
use tokio::{
    sync::RwLock,
    time::{interval, Duration as TokioDuration, MissedTickBehavior},
};
use tracing::{instrument, warn};
use uuid::Uuid;

use bible_manager::BibleManager;
use cache_manager::CacheManager;
use companion::{
    parse_bool_flag, COMPANION_FEATURE_KEY, COMPANION_PORT_KEY, DEFAULT_COMPANION_PORT,
};
use companion_manager::CompanionManager;
#[cfg(test)]
pub(crate) use seed::seed_sample_library;
#[cfg(test)]
pub use seed::TestBibleIngestion;

/// External API-driven stage state. All fields default to empty strings.
/// Missing or null JSON fields deserialize to "".
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiStageState {
    #[serde(default)]
    pub(crate) current_text: String,
    #[serde(default)]
    pub(crate) next_text: String,
    #[serde(default)]
    pub(crate) current_group: String,
    #[serde(default)]
    pub(crate) next_group: String,
    #[serde(default)]
    pub(crate) current_song: String,
    #[serde(default)]
    pub(crate) next_song: String,
}

#[derive(Clone)]
pub struct AppState {
    repository: Repository,
    live_hub: LiveHub,
    /// Bible broadcast / slide-output state (see [`BibleManager`]).
    bible: BibleManager,
    /// Companion (Bitfocus) integration state (see [`CompanionManager`]).
    companion: CompanionManager,
    resolume_registry: ResolumeRegistry,
    android_stage_registry: AndroidStageRegistry,
    stage_connections: StageConnections,
    heartbeat_config: StageHeartbeatConfig,
    /// In-memory caches: presentation / group-color / ableset (see [`CacheManager`]).
    caches: CacheManager,
    stage_layout: Arc<RwLock<String>>,
    osc_bridge: OscBridge,
    ableset_bridge: AbleSetBridge,
    broadcast_live: Arc<AtomicBool>,
    ai_conversation: Arc<RwLock<Vec<ChatMessage>>>,
    ai_proxy: Arc<ProxyManager>,
    ndi_manager: Option<ndi_control::NdiManagerHandle>,
    api_stage: Arc<RwLock<ApiStageState>>,
    pub local_public_ip: Arc<Option<String>>,
    /// Cloudflare Realtime TURN minting (#502). Disabled (no-op) when the
    /// `PRESENTER_TURN_KEY_*` env vars are unset — on-LAN WebRTC is unaffected.
    turn: TurnService,
}

/// Gate predicate for the startup NDI auto-restore branch.
///
/// Auto-restore must be skipped when either the NDI manager failed to load
/// (SDK missing) OR no hardware H264 encoder is registered. The latter
/// directly prevents the 2026-05-24 prod incident from recurring: even
/// with the registry rescan from item 1, encoder availability still has
/// to be confirmed before re-activating a source — otherwise the
/// supervisor would build a pipeline that immediately errors and the
/// browser Watchdog (PR #332) would retry-loop until the host wedges.
pub(crate) fn should_auto_restore_ndi(manager_loaded: bool, encoder_available: bool) -> bool {
    manager_loaded && encoder_available
}

impl AppState {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(
        repository: Repository,
        companion_token: Option<String>,
        companion_enabled: bool,
        companion_port: u16,
        resolume_registry: ResolumeRegistry,
        android_stage_registry: AndroidStageRegistry,
        osc_bridge: OscBridge,
        ableset_bridge: AbleSetBridge,
    ) -> Self {
        let heartbeat_config = StageHeartbeatConfig::default_values();
        Self::new_with_heartbeat(
            repository,
            companion_token,
            companion_enabled,
            companion_port,
            resolume_registry,
            android_stage_registry,
            osc_bridge,
            ableset_bridge,
            heartbeat_config,
            Arc::new(None),
        )
    }

    pub fn new_with_heartbeat(
        repository: Repository,
        companion_token: Option<String>,
        companion_enabled: bool,
        companion_port: u16,
        resolume_registry: ResolumeRegistry,
        android_stage_registry: AndroidStageRegistry,
        osc_bridge: OscBridge,
        ableset_bridge: AbleSetBridge,
        heartbeat_config: StageHeartbeatConfig,
        local_public_ip: Arc<Option<String>>,
    ) -> Self {
        let stage_connections = StageConnections::new();
        let default_layout = StageDisplayLayout::built_in()
            .into_iter()
            .map(|layout| layout.code)
            .find(|code| code == DEFAULT_STAGE_LAYOUT_CODE)
            .unwrap_or_else(|| DEFAULT_STAGE_LAYOUT_CODE.to_string());
        let ndi_manager = presenter_ndi::NdiManager::try_new()
            .map(Arc::new)
            .map(ndi_control::NdiManagerHandle::Real);
        if ndi_manager.is_some() {
            tracing::info!("NDI SDK loaded successfully");
        } else {
            tracing::warn!("NDI SDK not found — NDI features disabled");
        }
        let state = Self {
            repository,
            live_hub: LiveHub::new(),
            bible: BibleManager::new(),
            companion: CompanionManager::new(companion_token, companion_enabled, companion_port),
            resolume_registry,
            android_stage_registry,
            stage_connections,
            heartbeat_config,
            caches: CacheManager::new(),
            stage_layout: Arc::new(RwLock::new(default_layout)),
            osc_bridge,
            ableset_bridge,
            broadcast_live: Arc::new(AtomicBool::new(false)),
            ai_conversation: Arc::new(RwLock::new(Vec::new())),
            ai_proxy: Arc::new(ProxyManager::new(crate::ai::proxy::detect_deploy_dir())),
            ndi_manager,
            api_stage: Arc::new(RwLock::new(ApiStageState::default())),
            local_public_ip,
            turn: TurnService::from_env(),
        };
        state.spawn_heartbeat_tasks();
        state
    }

    fn spawn_heartbeat_tasks(&self) {
        let hub = self.live_hub.clone();
        let connections = self.stage_connections.clone();
        let config = self.heartbeat_config;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(config.interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let now = Utc::now();
                let heartbeat_id = Uuid::new_v4();
                connections.note_heartbeat_sent(heartbeat_id, now).await;
                hub.publish(LiveEvent::Heartbeat {
                    id: heartbeat_id,
                    timestamp: now,
                });
                let grace = config.grace_duration();
                let disconnect = config.disconnect_duration();
                let changed = connections.apply_timeouts(now, grace, disconnect).await;
                if !changed.is_empty() {
                    tracing::debug!(count = changed.len(), "stage connection statuses updated");
                    for snapshot in changed {
                        hub.publish(LiveEvent::StageConnection { snapshot });
                    }
                }
            }
        });
    }

    fn spawn_background_tasks(&self) {
        let timers_state = self.clone();
        tokio::spawn(async move {
            if let Err(err) = timers_state.tick_timers().await {
                warn!(?err, "timer tick failed");
            }
            let mut ticker = interval(TokioDuration::from_secs(1));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(err) = timers_state.tick_timers().await {
                    warn!(?err, "timer tick failed");
                }
            }
        });

        let wal_state = self.clone();
        tokio::spawn(async move {
            let mut ticker = interval(TokioDuration::from_secs(300));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(err) = wal_state.repository.wal_checkpoint().await {
                    warn!(?err, "periodic WAL checkpoint failed");
                }
            }
        });

        // NDI auto-reconnect: if a source is marked active in DB but no stream
        // is running, retry activation every 30 seconds. Handles the case where
        // the NDI source comes online after server startup.
        //
        // #333 item 6 (deep-review): the per-tick encoder gate. The one-shot
        // startup auto-restore is gated by should_auto_restore_ndi() above,
        // but this 30s loop was NOT gated and would re-trigger the wedge
        // state every 30 seconds on an encoder-missing host. We probe
        // hw_h264_encoder() PER TICK (a cheap instantiate-and-discard
        // loadability probe — #443; GStreamer element construction only
        // allocates the GObject, hardware opens at the READY transition) so a
        // host whose plugin registry self-heals can resume reconnect without a
        // process restart.
        if self.ndi_manager().is_some() {
            let ndi_state = self.clone();
            tokio::spawn(async move {
                let mut ticker = interval(TokioDuration::from_secs(30));
                ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
                loop {
                    ticker.tick().await;
                    let manager_loaded = ndi_state.ndi_manager().is_some();
                    let encoder_available =
                        manager_loaded && presenter_ndi::hw_h264_encoder().is_some();
                    if should_auto_restore_ndi(manager_loaded, encoder_available) {
                        match ndi_state.repository.get_active_video_source().await {
                            Ok(Some(source)) => {
                                let ndi_name = source.ndi_name.clone();
                                if let Err(err) = ndi_state
                                    .activate_video_source(
                                        source.id,
                                        presenter_persistence::SettingsAuditSource::StartupDefault,
                                        "system",
                                    )
                                    .await
                                {
                                    tracing::debug!(
                                        ?err,
                                        ndi_name = %ndi_name,
                                        "NDI auto-reconnect: source not yet available"
                                    );
                                } else {
                                    tracing::info!(
                                        ndi_name = %ndi_name,
                                        "NDI auto-reconnect: source restored"
                                    );
                                }
                            }
                            Ok(None) => {}
                            Err(err) => {
                                tracing::debug!(?err, "NDI auto-reconnect: DB query failed");
                            }
                        }
                    }
                }
            });
        }
    }

    #[instrument(skip_all)]
    pub async fn from_config(config: ServerConfig) -> anyhow::Result<Self> {
        let db_url = config.database.url;
        let repo = Repository::connect(&DatabaseSettings::new(&db_url)).await?;
        let companion_token = config.companion.token;

        let stored_companion = repo
            .get_app_setting(COMPANION_FEATURE_KEY)
            .await?
            .and_then(|value| parse_bool_flag(&value))
            .unwrap_or(false);

        let companion_enabled = config
            .companion
            .enabled_override
            .unwrap_or(stored_companion);

        if let Some(value) = config.companion.enabled_override {
            repo.set_app_setting(COMPANION_FEATURE_KEY, if value { "1" } else { "0" })
                .await?;
        }

        let raw_port = repo.get_app_setting(COMPANION_PORT_KEY).await?;
        let mut persist_port = false;
        let stored_port = raw_port
            .as_deref()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port >= 1);
        let mut companion_port = stored_port.unwrap_or(DEFAULT_COMPANION_PORT);
        if stored_port.is_none() {
            persist_port = true;
        }

        if let Some(port_override) = config.companion.port_override {
            companion_port = port_override;
            persist_port = true;
        }

        if persist_port {
            repo.set_app_setting(COMPANION_PORT_KEY, &companion_port.to_string())
                .await?;
        }

        let registry = ResolumeRegistry::new()?;
        let android_stage_registry = AndroidStageRegistry::new();
        let osc_bridge = OscBridge::new(&config.osc);
        let ableset_bridge = AbleSetBridge::new();
        let heartbeat_config = config.stage.heartbeat;
        let local_public_ip = Arc::new(config.network.local_public_ip);
        let mut state = Self::new_with_heartbeat(
            repo,
            companion_token,
            companion_enabled,
            companion_port,
            registry,
            android_stage_registry,
            osc_bridge.clone(),
            ableset_bridge.clone(),
            heartbeat_config,
            local_public_ip,
        );

        // Pre-load group color cache from database
        let group_colors = state
            .repository
            .load_all_group_colors()
            .await
            .unwrap_or_default();
        *state.caches.group_color.write().await = group_colors;

        // Restore the operator-selected stage layout from the database (#384).
        // Without this, every restart/deploy reset the layout to the default
        // (worship-snv), silently blanking NDI stage displays after each deploy.
        // This is a pure read — it writes nothing on an unchanged DB, preserving
        // the second-startup-no-audit invariant (CLAUDE.md).
        {
            let persisted = state.load_persisted_stage_layout().await;
            *state.stage_layout.write().await = persisted;
        }

        state.ensure_demo_playlist().await?;
        state.sync_resolume_hosts().await?;
        // #423: android stage displays are launched from `main` AFTER bind, not here (raced the listener) — see start_android_stage_displays.

        // Restore the active NDI video source from the database on startup
        // (encoder-gated, #333 item 6). Extracted to keep from_config under the
        // function-length cap.
        state.restore_active_ndi_source().await;

        // Auto-detect public IP for LAN/WAN classification via Cloudflare Tunnel.
        // Only runs if PRESENTER_LOCAL_PUBLIC_IP is not set (the env var is an
        // optional override, not a requirement). Mirrors the reaperiem pattern.
        if state.local_public_ip.is_none() {
            match detect_public_ip().await {
                Some(ip) => {
                    tracing::info!(ip = %ip, "auto-detected public IP for LAN/WAN detection");
                    state.local_public_ip = Arc::new(Some(ip));
                }
                None => {
                    tracing::warn!(
                        "could not auto-detect public IP; LAN/WAN detection will use private IP fallback"
                    );
                }
            }
        }

        Self::apply_osc_settings(&state, &osc_bridge).await?;
        Self::apply_ableset_settings(&state, &ableset_bridge).await?;

        // Re-broadcast stage snapshots whenever AbleSet switches songs.
        state.spawn_ableset_rebroadcast(&ableset_bridge);

        state
            .configure_companion_service(companion_enabled, companion_port)
            .await?;
        state.spawn_background_tasks();
        state.ai_proxy.auto_start().await;
        Ok(state)
    }

    /// Restore the active NDI video source on startup, gated on BOTH the NDI
    /// manager being available AND a hardware H264 encoder being registered
    /// (#333 item 6: without the encoder gate a stale GStreamer registry would
    /// re-trigger the 2026-05-24 wedge the moment auto-restore ran). On
    /// encoder-missing hosts the saved source is NOT re-activated and a
    /// structured warning is logged. Extracted from `from_config` to keep that
    /// constructor under the function-length cap.
    async fn restore_active_ndi_source(&self) {
        let manager_loaded = self.ndi_manager().is_some();
        // Encoder probe requires gstreamer::init() to have run. In production
        // main.rs calls presenter_ndi::init() before AppState is built; in tests
        // (AppState::in_memory) it doesn't, so gst::ElementFactory::find would
        // panic. We re-call init() here — idempotent via OnceLock — so the probe
        // is safe regardless of caller. If init fails the encoder is treated as
        // unavailable and the auto-restore branch is skipped (the safer default).
        let encoder_available = manager_loaded
            && presenter_ndi::init().is_ok()
            && presenter_ndi::hw_h264_encoder().is_some();
        if should_auto_restore_ndi(manager_loaded, encoder_available) {
            match self.repository.get_active_video_source().await {
                Ok(Some(source)) => {
                    let ndi_name = source.ndi_name.clone();
                    if let Err(err) = self
                        .activate_video_source(
                            source.id,
                            presenter_persistence::SettingsAuditSource::StartupDefault,
                            "system",
                        )
                        .await
                    {
                        tracing::warn!(
                            ?err,
                            ndi_name = %ndi_name,
                            "NDI source restore deferred — will connect when source appears"
                        );
                    } else {
                        tracing::info!(ndi_name = %ndi_name, "NDI source restored on startup");
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(?err, "failed to query active video source on startup");
                }
            }
        } else if manager_loaded && !encoder_available {
            tracing::warn!(
                "NDI auto-restore skipped: no hardware H264 encoder registered. \
                 Saved active source will NOT be re-activated on startup. \
                 Operator must investigate (likely stale GStreamer registry or \
                 missing VAAPI/NVENC packages) and explicitly re-activate via UI. \
                 See #333 item 6."
            );
        }
    }

    /// Spawn the background task that re-broadcasts stage snapshots whenever
    /// AbleSet switches songs. Extracted from `from_config` to keep that
    /// constructor under the function-length cap.
    fn spawn_ableset_rebroadcast(&self, ableset_bridge: &AbleSetBridge) {
        let mut rx = ableset_bridge.subscribe_song_changes();
        let app = self.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(()) => {
                        if let Err(err) = app.broadcast_stage_snapshots().await {
                            tracing::warn!(
                                ?err,
                                "failed to broadcast stage after AbleSet song change"
                            );
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // OSC settings application is in osc.rs (Self::apply_osc_settings)

    async fn apply_ableset_settings(
        state: &Self,
        ableset_bridge: &AbleSetBridge,
    ) -> anyhow::Result<()> {
        let ableset_settings = state.repository.get_ableset_settings().await?;
        match ableset_bridge
            .apply_settings(ableset_settings.clone())
            .await
        {
            Ok(()) => {
                let snapshot = state.ableset_bridge.status_snapshot().await;
                if let Err(err) = state.refresh_ableset_cache(&snapshot).await {
                    tracing::warn!(?err, "failed to seed AbleSet cache");
                }
            }
            Err(err) => tracing::warn!(?err, "failed to initialise AbleSet tracker"),
        }
        Ok(())
    }

    #[cfg(test)]
    #[instrument(skip_all)]
    pub async fn in_memory() -> anyhow::Result<Self> {
        let repo = Repository::connect_in_memory().await?;
        let registry = ResolumeRegistry::new()?;
        let android_stage_registry = AndroidStageRegistry::new();
        let osc_bridge = OscBridge::new(&OscConfig::default());
        let ableset_bridge = AbleSetBridge::new();
        let state = Self::new(
            repo,
            None,
            false,
            DEFAULT_COMPANION_PORT,
            registry,
            android_stage_registry,
            osc_bridge.clone(),
            ableset_bridge.clone(),
        );
        state.ensure_demo_playlist().await?;
        state.sync_android_stage_displays().await?;

        Self::apply_osc_settings(&state, &osc_bridge).await?;
        Self::apply_ableset_settings(&state, &ableset_bridge).await?;

        state
            .configure_companion_service(false, DEFAULT_COMPANION_PORT)
            .await?;
        Ok(state)
    }

    // OSC methods are in osc.rs
    // AbleSet methods are in ableset.rs
    // Resolume and Android stage methods are in integrations.rs

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    pub fn live_hub(&self) -> LiveHub {
        self.live_hub.clone()
    }

    pub fn ai_conversation(&self) -> &Arc<RwLock<Vec<ChatMessage>>> {
        &self.ai_conversation
    }

    pub fn ai_proxy(&self) -> &Arc<ProxyManager> {
        &self.ai_proxy
    }

    /// Cloudflare Realtime TURN service (#502): mints browser ICE servers for
    /// `GET /ndi/ice-servers` and the `webrtcbin` relay `turn://` URI.
    pub(crate) fn turn(&self) -> &TurnService {
        &self.turn
    }

    pub(crate) fn ndi_manager(&self) -> Option<&ndi_control::NdiManagerHandle> {
        self.ndi_manager.as_ref()
    }

    /// Test-only: replace the NDI manager handle with a recording fake.
    ///
    /// Used by the #406 activation-wiring tests to inject a libndi-free
    /// [`ndi_control::FakeNdiControl`] so the hardware-gated reap branch in
    /// [`Self::activate_video_source`] is reachable on CI hosts without libndi.
    #[cfg(test)]
    pub(crate) fn set_ndi_handle(&mut self, handle: ndi_control::NdiManagerHandle) {
        self.ndi_manager = Some(handle);
    }

    pub fn stage_connections_handle(&self) -> StageConnections {
        self.stage_connections.clone()
    }

    pub async fn stage_connections_snapshot(&self) -> Vec<StageClientSnapshot> {
        self.stage_connections.snapshot().await
    }

    // Companion methods
    pub fn companion_token(&self) -> Option<&str> {
        self.companion.token.as_deref()
    }

    pub fn companion_enabled(&self) -> bool {
        self.companion.enabled.load(Ordering::SeqCst)
    }

    pub fn companion_port(&self) -> u16 {
        self.companion.port.load(Ordering::SeqCst)
    }

    // Broadcast live state
    pub fn broadcast_live(&self) -> bool {
        self.broadcast_live.load(Ordering::SeqCst)
    }

    pub fn set_broadcast_live(&self, enabled: bool) {
        let previous = self.broadcast_live.swap(enabled, Ordering::SeqCst);
        if previous != enabled {
            self.live_hub.publish(LiveEvent::BroadcastLive { enabled });
        }
    }

    pub async fn configure_companion_service(
        &self,
        enabled: bool,
        port: u16,
    ) -> anyhow::Result<()> {
        self.companion
            .server
            .reconfigure(self.clone(), enabled, port)
            .await
    }

    pub async fn set_companion_settings(&self, enabled: bool, port: u16) -> anyhow::Result<()> {
        if port == 0 {
            return Err(anyhow::anyhow!(
                "companion port must be between 1 and 65535"
            ));
        }
        let previous_enabled = self.companion_enabled();
        let previous_port = self.companion_port();

        self.configure_companion_service(enabled, port).await?;

        if let Err(err) = self
            .repository
            .set_app_setting(COMPANION_PORT_KEY, &port.to_string())
            .await
        {
            if let Err(rollback_err) = self
                .configure_companion_service(previous_enabled, previous_port)
                .await
            {
                tracing::error!(
                    ?rollback_err,
                    "failed to rollback companion service after port setting error"
                );
            }
            return Err(err);
        }

        if let Err(err) = self
            .repository
            .set_app_setting(COMPANION_FEATURE_KEY, if enabled { "1" } else { "0" })
            .await
        {
            if let Err(rollback_err) = self
                .repository
                .set_app_setting(COMPANION_PORT_KEY, &previous_port.to_string())
                .await
            {
                tracing::error!(
                    ?rollback_err,
                    "failed to rollback port setting after enabled setting error"
                );
            }
            if let Err(rollback_err) = self
                .configure_companion_service(previous_enabled, previous_port)
                .await
            {
                tracing::error!(
                    ?rollback_err,
                    "failed to rollback companion service after enabled setting error"
                );
            }
            return Err(err);
        }

        self.companion.enabled.store(enabled, Ordering::SeqCst);
        self.companion.port.store(port, Ordering::SeqCst);
        Ok(())
    }

    // Presentation cache, group-color, and API-stage methods are in api_stage.rs

    pub async fn timers_overview(&self) -> anyhow::Result<TimersOverview> {
        let now = Utc::now();
        let state = self.load_or_init_timers(now).await?;
        Ok(state.overview(now))
    }

    // Stage-state mutation (update_stage_state / clear_stage) and slide-edit
    // reconciliation (reindex_slides / reconcile_stage_state_after_edit) are in
    // stage_state.rs
    // Stage display methods are in stage_display.rs
    // Slide editing methods are in slides.rs
    // Timer methods are in timers.rs
    // Broadcasting methods are in broadcasting.rs
}

/// Auto-detect the server's public IP by querying external services.
/// Tries multiple providers for reliability. Returns `None` if all fail
/// (e.g., no internet on startup — LAN/WAN detection falls back to the
/// private-range heuristic in that case).
async fn detect_public_ip() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?;
    let services = [
        "https://api.ipify.org",
        "https://ifconfig.me/ip",
        "https://icanhazip.com",
    ];
    for url in services {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(ip) = resp.text().await {
                    let ip = ip.trim().to_string();
                    if !ip.is_empty() && ip.len() < 46 {
                        return Some(ip);
                    }
                }
            }
            _ => continue,
        }
    }
    None
}
