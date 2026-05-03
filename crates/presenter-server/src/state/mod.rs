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
pub(crate) mod bible;
mod broadcasting;
mod companion;
mod integrations;
mod presentations;
mod seed;
pub(crate) mod slides;
pub(crate) mod stage;
mod stage_display;
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
    osc::{OscBridge, OscStatusSnapshot},
    resolume::ResolumeRegistry,
    stage_connections::{StageConnections, StageHeartbeatConfig},
};
use chrono::Utc;
use presenter_core::{
    BibleBroadcast, BibleSlideOutput, OscSettings, OscSettingsDraft, PlaylistId, Presentation,
    PresentationId, Slide, SlideId, StageClientSnapshot, StageDisplayLayout, StageDisplaySlide,
    StageDisplaySnapshot, StageState, TimersOverview, API_STAGE_LAYOUT_CODE,
    DEFAULT_STAGE_LAYOUT_CODE,
};
use presenter_persistence::{DatabaseSettings, Repository};
use std::{
    collections::HashMap,
    env,
    sync::{atomic::AtomicBool, atomic::AtomicU16, atomic::Ordering, Arc},
};
use tokio::{
    sync::RwLock,
    time::{interval, Duration as TokioDuration, MissedTickBehavior},
};
use tracing::{instrument, warn};
use uuid::Uuid;

use ableset::AbleSetLibraryCache;
use companion::{
    parse_bool_flag, CompanionServerManager, COMPANION_FEATURE_KEY, COMPANION_PORT_KEY,
    DEFAULT_COMPANION_PORT,
};
#[cfg(test)]
pub(crate) use seed::seed_sample_library;
#[cfg(test)]
pub use seed::TestBibleIngestion;
use stage::{build_stage_playlist_entries, stage_resolution_from_presentation, StageResolution};

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
    bible_broadcast: Arc<RwLock<Option<BibleBroadcast>>>,
    /// New single-source-of-truth Bible slide output
    bible_slide_output: Arc<RwLock<Option<BibleSlideOutput>>>,
    companion_token: Option<String>,
    companion_enabled: Arc<AtomicBool>,
    companion_port: Arc<AtomicU16>,
    companion_server: CompanionServerManager,
    resolume_registry: ResolumeRegistry,
    android_stage_registry: AndroidStageRegistry,
    stage_connections: StageConnections,
    heartbeat_config: StageHeartbeatConfig,
    presentation_cache: Arc<RwLock<HashMap<PresentationId, Arc<Presentation>>>>,
    stage_layout: Arc<RwLock<String>>,
    osc_bridge: OscBridge,
    ableset_bridge: AbleSetBridge,
    ableset_cache: Arc<RwLock<AbleSetLibraryCache>>,
    broadcast_live: Arc<AtomicBool>,
    ai_conversation: Arc<RwLock<Vec<ChatMessage>>>,
    ai_proxy: Arc<ProxyManager>,
    ndi_manager: Option<Arc<presenter_ndi::NdiManager>>,
    group_color_cache: Arc<RwLock<HashMap<String, String>>>,
    api_stage: Arc<RwLock<ApiStageState>>,
    pub local_public_ip: Arc<Option<String>>,
    #[cfg(test)]
    bible_ingestion_override: Option<std::sync::Arc<dyn TestBibleIngestion + Send + Sync>>,
}

pub use presenter_core::FeatureFlags;

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
        let ableset_cache = Arc::new(RwLock::new(AbleSetLibraryCache::default()));
        let ndi_manager = presenter_ndi::NdiManager::try_new().map(Arc::new);
        if ndi_manager.is_some() {
            tracing::info!("NDI SDK loaded successfully");
        } else {
            tracing::warn!("NDI SDK not found — NDI features disabled");
        }
        let state = Self {
            repository,
            live_hub: LiveHub::new(),
            bible_broadcast: Arc::new(RwLock::new(None)),
            bible_slide_output: Arc::new(RwLock::new(None)),
            companion_token,
            companion_enabled: Arc::new(AtomicBool::new(companion_enabled)),
            companion_port: Arc::new(AtomicU16::new(companion_port)),
            companion_server: CompanionServerManager::default(),
            resolume_registry,
            android_stage_registry,
            stage_connections,
            heartbeat_config,
            presentation_cache: Arc::new(RwLock::new(HashMap::new())),
            stage_layout: Arc::new(RwLock::new(default_layout)),
            osc_bridge,
            ableset_bridge,
            ableset_cache,
            broadcast_live: Arc::new(AtomicBool::new(false)),
            ai_conversation: Arc::new(RwLock::new(Vec::new())),
            ai_proxy: Arc::new(ProxyManager::new(crate::ai::proxy::detect_deploy_dir())),
            ndi_manager,
            group_color_cache: Arc::new(RwLock::new(HashMap::new())),
            api_stage: Arc::new(RwLock::new(ApiStageState::default())),
            local_public_ip,
            #[cfg(test)]
            bible_ingestion_override: None,
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
        if self.ndi_manager().is_some() {
            let ndi_state = self.clone();
            tokio::spawn(async move {
                let mut ticker = interval(TokioDuration::from_secs(30));
                ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
                loop {
                    ticker.tick().await;
                    if let Some(manager) = ndi_state.ndi_manager() {
                        if manager.is_streaming().await {
                            continue;
                        }
                        match ndi_state.repository.get_active_video_source().await {
                            Ok(Some(source)) => {
                                let ndi_name = source.ndi_name.clone();
                                if let Err(err) = ndi_state.activate_video_source(source.id).await {
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
        *state.group_color_cache.write().await = group_colors;
        state.ensure_demo_playlist().await?;
        state.sync_resolume_hosts().await?;
        state.sync_android_stage_displays().await?;

        // Restore active NDI video source from database
        if state.ndi_manager().is_some() {
            match state.repository.get_active_video_source().await {
                Ok(Some(source)) => {
                    let ndi_name = source.ndi_name.clone();
                    if let Err(err) = state.activate_video_source(source.id).await {
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
        }

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

        // Re-broadcast stage snapshots whenever AbleSet switches songs
        {
            let mut rx = ableset_bridge.subscribe_song_changes();
            let app = state.clone();
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

        state
            .configure_companion_service(companion_enabled, companion_port)
            .await?;
        state.spawn_background_tasks();
        state.ai_proxy.auto_start().await;
        Ok(state)
    }

    async fn apply_osc_settings(state: &Self, osc_bridge: &OscBridge) -> anyhow::Result<()> {
        let mut osc_settings = state.repository.get_osc_settings().await?;
        if let Ok(port_raw) = env::var("PRESENTER_OSC_LISTEN_PORT") {
            match port_raw.parse::<u16>() {
                Ok(port) if port != 0 && port != osc_settings.listen_port => {
                    let draft = OscSettingsDraft {
                        enabled: osc_settings.enabled,
                        listen_port: port,
                        address_pattern: osc_settings.address_pattern.clone(),
                        velocity_mode: osc_settings.velocity_mode,
                    };
                    osc_settings = state.repository.upsert_osc_settings(&draft).await?;
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(value = %port_raw, ?err, "invalid PRESENTER_OSC_LISTEN_PORT")
                }
            }
        }
        if let Err(err) = osc_bridge
            .apply_settings(osc_settings.clone(), state.clone())
            .await
        {
            tracing::warn!(?err, "failed to initialise OSC listener");
        }
        Ok(())
    }

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

    // OSC methods
    pub async fn osc_settings(&self) -> anyhow::Result<OscSettings> {
        self.repository.get_osc_settings().await
    }

    pub async fn update_osc_settings(
        &self,
        draft: OscSettingsDraft,
    ) -> anyhow::Result<OscSettings> {
        let settings = self.repository.upsert_osc_settings(&draft).await?;
        self.osc_bridge
            .apply_settings(settings.clone(), self.clone())
            .await?;
        Ok(settings)
    }

    pub async fn osc_status_snapshot(&self) -> OscStatusSnapshot {
        self.osc_bridge.status().await
    }

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

    pub fn ndi_manager(&self) -> Option<&Arc<presenter_ndi::NdiManager>> {
        self.ndi_manager.as_ref()
    }

    pub fn stage_connections_handle(&self) -> StageConnections {
        self.stage_connections.clone()
    }

    pub async fn stage_connections_snapshot(&self) -> Vec<StageClientSnapshot> {
        self.stage_connections.snapshot().await
    }

    // Companion methods
    pub fn companion_token(&self) -> Option<&str> {
        self.companion_token.as_deref()
    }

    pub fn companion_enabled(&self) -> bool {
        self.companion_enabled.load(Ordering::SeqCst)
    }

    pub fn companion_port(&self) -> u16 {
        self.companion_port.load(Ordering::SeqCst)
    }

    pub fn feature_flags(&self) -> FeatureFlags {
        FeatureFlags {
            companion_enabled: self.companion_enabled(),
            companion_port: self.companion_port(),
        }
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
        self.companion_server
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

        self.companion_enabled.store(enabled, Ordering::SeqCst);
        self.companion_port.store(port, Ordering::SeqCst);
        Ok(())
    }

    // Presentation cache methods
    async fn presentation_from_cache(
        &self,
        presentation_id: PresentationId,
    ) -> anyhow::Result<Arc<Presentation>> {
        if let Some(cached) = {
            let guard = self.presentation_cache.read().await;
            guard.get(&presentation_id).cloned()
        } {
            return Ok(cached);
        }
        let detail = self
            .repository
            .fetch_presentation_detail(presentation_id)
            .await?;
        let Some((_, _, presentation)) = detail else {
            return Err(anyhow::anyhow!("presentation not found"));
        };
        let arc = Arc::new(presentation);
        let mut guard = self.presentation_cache.write().await;
        guard.insert(presentation_id, arc.clone());
        Ok(arc)
    }

    pub(crate) async fn get_all_group_colors(&self) -> HashMap<String, String> {
        self.group_color_cache.read().await.clone()
    }

    pub(crate) async fn resolve_group_color(&self, name: &str) -> Option<String> {
        {
            let cache = self.group_color_cache.read().await;
            if let Some(color) = cache.get(name) {
                return Some(color.clone());
            }
        }
        match self.repository.resolve_group_color(name).await {
            Ok(color) => {
                let mut cache = self.group_color_cache.write().await;
                cache.insert(name.to_string(), color.clone());
                Some(color)
            }
            Err(_) => None,
        }
    }

    pub(crate) async fn update_api_stage(&self, state: ApiStageState) -> anyhow::Result<()> {
        let snapshot = self.build_api_stage_snapshot(&state).await;
        *self.api_stage.write().await = state;
        // Issue #281: only publish a Stage event when the operator's
        // current layout is "api". Otherwise the api state is stored but
        // does not affect the live preview, mirroring the existing inverse
        // gate in `broadcasting.rs::publish_stage_context` (which skips
        // non-api updates when api layout is selected).
        if self.stage_layout_code().await == API_STAGE_LAYOUT_CODE {
            self.live_hub.publish(LiveEvent::Stage { snapshot });
        }
        Ok(())
    }

    pub(crate) async fn api_stage_snapshot(&self) -> StageDisplaySnapshot {
        let state = self.api_stage.read().await;
        self.build_api_stage_snapshot(&state).await
    }

    async fn build_api_stage_snapshot(&self, state: &ApiStageState) -> StageDisplaySnapshot {
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|l| l.code == "api")
            .expect("api layout must exist in built_in");

        let current = self
            .build_api_slide(&state.current_text, &state.current_group)
            .await;
        let next = self
            .build_api_slide(&state.next_text, &state.next_group)
            .await;

        let song_name = if state.current_song.is_empty() {
            None
        } else {
            Some(state.current_song.clone())
        };
        let next_song_name = if state.next_song.is_empty() {
            None
        } else {
            Some(state.next_song.clone())
        };

        let now = Utc::now();
        let timers = self
            .load_or_init_timers(now)
            .await
            .map(|t| t.overview(now))
            .unwrap_or_else(|_| TimersOverview::demo(now));

        StageDisplaySnapshot::new(
            layout,
            now,
            None,           // presentation_id
            None,           // presentation_name
            None,           // library_name
            song_name,      // song_name
            None,           // song_number
            next_song_name, // next_song_name
            None,           // current_slide_id
            current,        // current
            None,           // next_slide_id
            next,           // next
            timers,         // timers
            None,           // latency_ms
            None,           // current_position
            None,           // total_slides
            None,           // playlist_id
            None,           // playlist_name
            None,           // playlist_entries
        )
    }

    async fn build_api_slide(&self, text: &str, group_name: &str) -> Option<StageDisplaySlide> {
        if text.is_empty() && group_name.is_empty() {
            return None;
        }
        let group = if group_name.is_empty() {
            None
        } else {
            Some(group_name.to_string())
        };
        let group_color = if let Some(ref name) = group {
            self.resolve_group_color(name).await
        } else {
            None
        };
        Some(StageDisplaySlide {
            main: text.to_string(),
            translation: String::new(),
            stage: String::new(),
            group,
            group_color,
        })
    }

    async fn cache_presentation_ref(&self, presentation: &Presentation) {
        let mut guard = self.presentation_cache.write().await;
        guard.insert(presentation.id, Arc::new(presentation.clone()));
    }

    async fn cache_presentation_value(&self, presentation: Presentation) {
        let mut guard = self.presentation_cache.write().await;
        guard.insert(presentation.id, Arc::new(presentation));
    }

    pub async fn timers_overview(&self) -> anyhow::Result<TimersOverview> {
        let now = Utc::now();
        let state = self.load_or_init_timers(now).await?;
        Ok(state.overview(now))
    }

    pub async fn update_stage_state(
        &self,
        presentation_id: PresentationId,
        current_slide_id: SlideId,
        next_slide_id: Option<SlideId>,
        playlist_id: Option<PlaylistId>,
    ) -> anyhow::Result<()> {
        let Some((_, library_name, presentation)) =
            self.presentation_detail(presentation_id).await?
        else {
            anyhow::bail!("presentation not found");
        };

        if !presentation
            .slides
            .iter()
            .any(|slide| slide.id == current_slide_id)
        {
            anyhow::bail!("current slide not found in presentation");
        }

        if let Some(next_slide_id) = next_slide_id {
            if !presentation
                .slides
                .iter()
                .any(|slide| slide.id == next_slide_id)
            {
                anyhow::bail!("next slide not found in presentation");
            }
        }

        let stage_state = presenter_core::StageState::new(
            Some(presentation_id),
            Some(current_slide_id),
            next_slide_id,
            playlist_id,
        );
        self.repository.upsert_stage_state(&stage_state).await?;
        let mut resolution = stage_resolution_from_presentation(
            &presentation,
            Some(library_name),
            Some(current_slide_id),
            next_slide_id,
        );
        if let Some(pid) = playlist_id {
            if let Some(playlist) = self.repository.fetch_playlist_by_id(pid).await? {
                let name_lookup = self
                    .repository
                    .fetch_presentation_names_for_playlist(&playlist)
                    .await?;
                resolution.playlist_id = Some(pid);
                resolution.playlist_name = Some(playlist.name.clone());
                resolution.playlist_entries = Some(build_stage_playlist_entries(
                    &playlist,
                    resolution.presentation_id,
                    &name_lookup,
                ));
            }
        }
        self.broadcast_stage_resolution(resolution).await?;
        Ok(())
    }

    pub async fn clear_stage(&self) -> anyhow::Result<()> {
        let cleared = StageState::cleared();
        self.repository.upsert_stage_state(&cleared).await?;
        self.broadcast_stage_resolution(StageResolution::cleared())
            .await?;
        Ok(())
    }

    // Stage display methods are in stage_display.rs
    // Slide editing methods are in slides.rs
    // Timer methods are in timers.rs
    // Broadcasting methods are in broadcasting.rs

    fn reindex_slides(slides: &mut [Slide]) {
        for (index, slide) in slides.iter_mut().enumerate() {
            slide.order = index as u32;
        }
    }

    async fn reconcile_stage_state_after_edit(
        &self,
        presentation_id: PresentationId,
        slides: &[Slide],
    ) -> anyhow::Result<()> {
        let Some(mut state) = self.repository.get_stage_state().await? else {
            return Ok(());
        };
        if state.presentation_id != Some(presentation_id) {
            return Ok(());
        }

        if slides.is_empty() {
            if state.current_slide_id.is_some() || state.next_slide_id.is_some() {
                state.current_slide_id = None;
                state.next_slide_id = None;
                self.repository.upsert_stage_state(&state).await?;
            }
            return Ok(());
        }

        let mut changed = false;
        let contains = |id: Option<SlideId>| {
            id.is_none_or(|target| slides.iter().any(|slide| slide.id == target))
        };
        if !contains(state.current_slide_id) {
            state.current_slide_id = Some(slides[0].id);
            state.next_slide_id = slides.get(1).map(|slide| slide.id);
            changed = true;
        } else if !contains(state.next_slide_id) {
            if let Some(current) = state.current_slide_id {
                if let Some(position) = slides.iter().position(|slide| slide.id == current) {
                    state.next_slide_id = slides.get(position + 1).map(|slide| slide.id);
                } else {
                    state.next_slide_id = slides.get(1).map(|slide| slide.id);
                }
            } else {
                state.next_slide_id = slides.get(1).map(|slide| slide.id);
            }
            changed = true;
        }

        if changed {
            self.repository.upsert_stage_state(&state).await?;
        }
        Ok(())
    }
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
