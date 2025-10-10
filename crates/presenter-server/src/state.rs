use crate::{
    ableset::{AbleSetBridge, AbleSetStatusSnapshot},
    android_stage::{AndroidStageDisplayStatusSnapshot, AndroidStageRegistry},
    live::{LiveEvent, LiveHub},
    osc::{OscBridge, OscStatusSnapshot},
    resolume::{
        BibleUpdate, ResolumeConnectionSnapshot, ResolumeRegistry, StageUpdate, TimerFrame,
    },
    stage_connections::{StageClientSnapshot, StageConnections, StageHeartbeatConfig},
};
use chrono::{DateTime, Utc};
use presenter_bible::BibleImportSummary;
use presenter_core::playlist::PlaylistEntryKind;
use presenter_core::{
    extract_song_prefix, AbleSetSettings, AbleSetSettingsDraft, AbleSetSongSnapshot,
    AndroidStageDisplay, AndroidStageDisplayDraft, AndroidStageDisplayId, BibleBroadcast,
    BibleReference, BibleTranslation, Library, LibraryId, LibrarySummary, OscSettings,
    OscSettingsDraft, Playlist, PlaylistEntry, PlaylistEntryId, PlaylistId, Presentation,
    PresentationId, ResolumeHost, ResolumeHostDraft, ResolumeHostId, SearchResult, Slide,
    SlideContent, SlideGroup, SlideId, SlideText, StageDisplayLayout, StageDisplaySlide,
    StageDisplaySnapshot, StagePlaylistEntry, StagePlaylistSummary, StageState, TimerCommand,
    TimerState, TimersOverview, TimersState,
};
use presenter_importer::bible::BibleIngestionService;
use presenter_persistence::{DatabaseSettings, Repository};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    sync::{atomic::AtomicBool, atomic::AtomicU16, atomic::Ordering, Arc},
};
use tokio::{
    net::TcpListener,
    sync::{oneshot, Mutex, RwLock},
    task::JoinHandle,
    time::{interval, Duration as TokioDuration, MissedTickBehavior},
};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

fn parse_bool_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn clamp_line_limit(value: u16) -> u16 {
    value.max(LINE_LIMIT_MIN).min(LINE_LIMIT_MAX)
}

const COMPANION_FEATURE_KEY: &str = "feature.companion.enabled";
const COMPANION_PORT_KEY: &str = "feature.companion.port";
const DEFAULT_COMPANION_PORT: u16 = 18_175;
const LINE_LIMIT_KEY: &str = "ui.line_limit";
pub const LINE_LIMIT_MIN: u16 = 10;
pub const LINE_LIMIT_MAX: u16 = 120;
pub const DEFAULT_LINE_LIMIT: u16 = 32;

#[derive(Clone, Default)]
struct CompanionServerManager {
    handle: Arc<Mutex<Option<CompanionServerHandle>>>,
}

struct CompanionServerHandle {
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
}

impl CompanionServerManager {
    async fn reconfigure(&self, state: AppState, enabled: bool, port: u16) -> anyhow::Result<()> {
        if !enabled {
            self.stop().await;
            return Ok(());
        }

        {
            let guard = self.handle.lock().await;
            if let Some(existing) = guard.as_ref() {
                if existing.port == port && !existing.join.is_finished() {
                    return Ok(());
                }
            }
        }

        // Attempt to bind before tearing down the current server so we can surface errors without
        // losing the previous listener when switching ports.
        let listener = TcpListener::bind(("0.0.0.0", port)).await.map_err(|err| {
            anyhow::anyhow!("failed to bind Companion websocket on port {port}: {err}")
        })?;

        let mut guard = self.handle.lock().await;
        if let Some(existing) = guard.take() {
            existing.shutdown().await;
        }

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let router = crate::companion::build_router(state.clone());
        let join = tokio::spawn(async move {
            let shutdown = async {
                let _ = shutdown_rx.await;
            };
            let server = axum::serve(listener, router);
            info!(port, "Companion websocket server listening");
            let result = server.with_graceful_shutdown(shutdown).await;
            if let Err(err) = result {
                error!(?err, port, "Companion websocket server exited with error");
            } else {
                debug!(port, "Companion websocket server stopped");
            }
        });

        *guard = Some(CompanionServerHandle {
            port,
            shutdown: Some(shutdown_tx),
            join,
        });

        Ok(())
    }

    async fn stop(&self) {
        let mut guard = self.handle.lock().await;
        if let Some(handle) = guard.take() {
            handle.shutdown().await;
        }
    }
}

impl CompanionServerHandle {
    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Err(err) = self.join.await {
            error!(
                ?err,
                port = self.port,
                "Companion websocket task join error"
            );
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    repository: Repository,
    live_hub: LiveHub,
    bible_broadcast: Arc<RwLock<Option<BibleBroadcast>>>,
    companion_token: Option<String>,
    companion_enabled: Arc<AtomicBool>,
    companion_port: Arc<AtomicU16>,
    line_limit: Arc<AtomicU16>,
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
    #[cfg(test)]
    bible_ingestion_override: Option<std::sync::Arc<dyn TestBibleIngestion + Send + Sync>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlags {
    pub companion_enabled: bool,
    pub companion_port: u16,
    pub line_limit: u16,
}

impl AppState {
    pub fn new(
        repository: Repository,
        companion_token: Option<String>,
        companion_enabled: bool,
        companion_port: u16,
        line_limit: u16,
        resolume_registry: ResolumeRegistry,
        android_stage_registry: AndroidStageRegistry,
        osc_bridge: OscBridge,
        ableset_bridge: AbleSetBridge,
    ) -> Self {
        let stage_connections = StageConnections::new();
        let heartbeat_config = StageHeartbeatConfig::from_env();
        let default_layout = StageDisplayLayout::built_in()
            .into_iter()
            .map(|layout| layout.code)
            .find(|code| code == "worship-snv")
            .unwrap_or_else(|| "worship-snv".to_string());
        let ableset_cache = Arc::new(RwLock::new(AbleSetLibraryCache::default()));
        let state = Self {
            repository,
            live_hub: LiveHub::new(),
            bible_broadcast: Arc::new(RwLock::new(None)),
            companion_token,
            companion_enabled: Arc::new(AtomicBool::new(companion_enabled)),
            companion_port: Arc::new(AtomicU16::new(companion_port)),
            line_limit: Arc::new(AtomicU16::new(line_limit)),
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
    }

    #[instrument(skip_all)]
    pub async fn from_env() -> anyhow::Result<Self> {
        let db_url = env::var("PRESENTER_DB_URL")
            .unwrap_or_else(|_| "sqlite://presenter_dev.db".to_string());
        let repo = Repository::connect(&DatabaseSettings::new(&db_url)).await?;
        let companion_token = env::var("PRESENTER_COMPANION_TOKEN").ok();

        let stored_companion = repo
            .get_app_setting(COMPANION_FEATURE_KEY)
            .await?
            .and_then(|value| parse_bool_flag(&value))
            .unwrap_or(false);

        let env_override = env::var("PRESENTER_COMPANION_ENABLED")
            .ok()
            .and_then(|value| parse_bool_flag(&value));

        let companion_enabled = env_override.unwrap_or(stored_companion);

        if let Some(value) = env_override {
            repo.set_app_setting(COMPANION_FEATURE_KEY, if value { "1" } else { "0" })
                .await?;
        }

        let raw_port = repo.get_app_setting(COMPANION_PORT_KEY).await?;
        let mut persist_port = false;
        let stored_port = raw_port
            .as_deref()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port >= 1 && *port <= u16::MAX);
        let mut companion_port = stored_port.unwrap_or(DEFAULT_COMPANION_PORT);
        if stored_port.is_none() {
            persist_port = true;
        }

        let env_port_override = env::var("PRESENTER_COMPANION_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port >= 1 && *port <= u16::MAX);

        if let Some(port_override) = env_port_override {
            companion_port = port_override;
            persist_port = true;
        }

        if persist_port {
            repo.set_app_setting(COMPANION_PORT_KEY, &companion_port.to_string())
                .await?;
        }

        let raw_line_limit = repo.get_app_setting(LINE_LIMIT_KEY).await?;
        let parsed_line_limit = raw_line_limit
            .as_deref()
            .and_then(|value| value.parse::<u16>().ok());
        let mut line_limit = parsed_line_limit
            .map(clamp_line_limit)
            .unwrap_or(DEFAULT_LINE_LIMIT);
        let mut persist_line_limit_setting = raw_line_limit.is_none();
        if let Some(parsed) = parsed_line_limit {
            let clamped = clamp_line_limit(parsed);
            if clamped != parsed {
                line_limit = clamped;
                persist_line_limit_setting = true;
            }
        } else {
            persist_line_limit_setting = true;
        }
        if persist_line_limit_setting {
            repo.set_app_setting(LINE_LIMIT_KEY, &line_limit.to_string())
                .await?;
        }

        let registry = ResolumeRegistry::new();
        let android_stage_registry = AndroidStageRegistry::new();
        let osc_bridge = OscBridge::new();
        let ableset_bridge = AbleSetBridge::new();
        let state = Self::new(
            repo,
            companion_token,
            companion_enabled,
            companion_port,
            line_limit,
            registry,
            android_stage_registry,
            osc_bridge.clone(),
            ableset_bridge.clone(),
        );
        state.ensure_seed_library().await?;
        state.ensure_demo_playlist().await?;
        state.sync_resolume_hosts().await?;
        state.sync_android_stage_displays().await?;
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
        state
            .configure_companion_service(companion_enabled, companion_port)
            .await?;
        state.spawn_background_tasks();
        Ok(state)
    }
    #[cfg(test)]
    #[instrument(skip_all)]
    pub async fn in_memory() -> anyhow::Result<Self> {
        let repo = Repository::connect_in_memory().await?;
        let registry = ResolumeRegistry::new();
        let android_stage_registry = AndroidStageRegistry::new();
        let osc_bridge = OscBridge::new();
        let ableset_bridge = AbleSetBridge::new();
        let state = Self::new(
            repo,
            None,
            false,
            DEFAULT_COMPANION_PORT,
            DEFAULT_LINE_LIMIT,
            registry,
            android_stage_registry,
            osc_bridge.clone(),
            ableset_bridge.clone(),
        );
        state.ensure_seed_library().await?;
        state.ensure_demo_playlist().await?;
        state.sync_android_stage_displays().await?;
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
        state
            .configure_companion_service(false, DEFAULT_COMPANION_PORT)
            .await?;
        Ok(state)
    }

    pub async fn libraries(&self) -> anyhow::Result<Vec<Library>> {
        self.repository.fetch_libraries().await
    }

    pub async fn create_library(&self, name: &str) -> anyhow::Result<Library> {
        self.repository.create_library(name).await
    }

    pub async fn library_favorites(&self) -> anyhow::Result<Vec<LibraryId>> {
        self.repository.list_library_favorites().await
    }

    pub async fn set_library_favorite(
        &self,
        library_id: LibraryId,
        favorite: bool,
    ) -> anyhow::Result<()> {
        self.repository
            .set_library_favorite(library_id, favorite)
            .await
    }

    pub async fn rename_library(&self, library_id: LibraryId, name: &str) -> anyhow::Result<()> {
        self.repository.rename_library(library_id, name).await
    }

    pub async fn delete_library(&self, library_id: LibraryId) -> anyhow::Result<()> {
        self.repository.delete_library(library_id).await
    }

    pub async fn create_presentation(
        &self,
        library_id: LibraryId,
        name: &str,
    ) -> anyhow::Result<(LibraryId, String, Presentation, Option<LibrarySummary>)> {
        let (id, lib_name, presentation) = self
            .repository
            .create_presentation(library_id, name)
            .await?;
        self.cache_presentation_ref(&presentation).await;
        let summaries = self.repository.list_library_summaries(None).await?;
        let summary = summaries.into_iter().find(|summary| summary.id == id);
        Ok((id, lib_name, presentation, summary))
    }

    pub async fn rename_presentation(
        &self,
        presentation_id: PresentationId,
        name: &str,
    ) -> anyhow::Result<()> {
        self.repository
            .rename_presentation(presentation_id, name)
            .await?;
        {
            let mut guard = self.presentation_cache.write().await;
            if let Some(entry) = guard.get_mut(&presentation_id) {
                let pres = Arc::make_mut(entry);
                pres.name = name.to_string();
            }
        }
        Ok(())
    }

    pub async fn list_bible_translations(&self) -> anyhow::Result<Vec<BibleTranslation>> {
        self.repository.list_bible_translations().await
    }

    pub async fn library_summaries(
        &self,
        query: Option<&str>,
    ) -> anyhow::Result<Vec<LibrarySummary>> {
        self.repository.list_library_summaries(query).await
    }

    pub async fn search_presenter(
        &self,
        query: &str,
        limit: u64,
    ) -> anyhow::Result<Vec<SearchResult>> {
        self.repository.search_presenter(query, limit).await
    }

    pub async fn playlists(&self) -> anyhow::Result<Vec<Playlist>> {
        self.repository.list_playlists().await
    }

    pub async fn create_playlist(
        &self,
        name: &str,
        show_in_dashboard: bool,
    ) -> anyhow::Result<Playlist> {
        self.repository
            .create_playlist(name, show_in_dashboard)
            .await
    }

    pub async fn rename_playlist(&self, playlist_id: PlaylistId, name: &str) -> anyhow::Result<()> {
        self.repository.rename_playlist(playlist_id, name).await
    }

    pub async fn set_playlist_favorite(
        &self,
        playlist_id: PlaylistId,
        favorite: bool,
    ) -> anyhow::Result<()> {
        self.repository
            .set_playlist_favorite(playlist_id, favorite)
            .await
    }

    pub async fn delete_playlist(&self, playlist_id: PlaylistId) -> anyhow::Result<()> {
        self.repository.delete_playlist(playlist_id).await
    }

    pub async fn replace_playlist_entries(
        &self,
        playlist_id: PlaylistId,
        entries: Vec<PlaylistEntry>,
    ) -> anyhow::Result<Playlist> {
        self.repository
            .replace_playlist_entries(playlist_id, &entries)
            .await?;
        let playlists = self.repository.list_playlists().await?;
        playlists
            .into_iter()
            .find(|playlist| playlist.id == playlist_id)
            .ok_or_else(|| anyhow::anyhow!("playlist not found after update"))
    }

    pub async fn search_bible_passages(
        &self,
        translation_code: &str,
        query: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<presenter_core::BiblePassage>> {
        self.repository
            .search_bible_passages(translation_code, query, limit)
            .await
    }

    pub async fn find_bible_passage(
        &self,
        translation_code: &str,
        reference: &BibleReference,
    ) -> anyhow::Result<Option<presenter_core::BiblePassage>> {
        self.repository
            .find_bible_passage(translation_code, reference)
            .await
    }

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

    pub async fn ableset_settings(&self) -> anyhow::Result<AbleSetSettings> {
        self.repository.get_ableset_settings().await
    }

    pub async fn update_ableset_settings(
        &self,
        draft: AbleSetSettingsDraft,
    ) -> anyhow::Result<AbleSetSettings> {
        let settings = self.repository.upsert_ableset_settings(&draft).await?;
        self.ableset_bridge.apply_settings(settings.clone()).await?;
        {
            let mut cache = self.ableset_cache.write().await;
            cache.invalidate();
            cache.library_name = None;
            cache.song_prefix_length = settings.song_prefix_length;
        }
        Ok(settings)
    }

    pub async fn ableset_status_snapshot(&self) -> AbleSetStatusSnapshot {
        self.ableset_bridge.status_snapshot().await
    }

    pub async fn set_ableset_follow(&self, enabled: bool) -> AbleSetStatusSnapshot {
        self.ableset_bridge.set_follow_enabled(enabled).await
    }

    pub async fn current_ableset_song(&self) -> Option<AbleSetSongSnapshot> {
        self.ableset_bridge.song_snapshot().await
    }

    pub async fn resolve_ableset_presentation(
        &self,
        prefix: &str,
    ) -> anyhow::Result<Option<PresentationId>> {
        let key = prefix.trim();
        if key.is_empty() {
            return Ok(None);
        }
        let settings = self.ableset_bridge.status_snapshot().await;
        if !settings.enabled {
            return Ok(None);
        }
        self.ensure_ableset_cache(&settings).await?;
        let lookup = key.to_ascii_lowercase();
        let cache = self.ableset_cache.read().await;
        Ok(cache.entries.get(&lookup).copied())
    }

    async fn ensure_ableset_cache(&self, settings: &AbleSetStatusSnapshot) -> anyhow::Result<()> {
        let needs_refresh = {
            let cache = self.ableset_cache.read().await;
            !cache.matches(&settings.library_name, settings.song_prefix_length)
                || cache.entries.is_empty()
        };
        if needs_refresh {
            self.refresh_ableset_cache(settings).await?;
        }
        Ok(())
    }

    async fn refresh_ableset_cache(&self, settings: &AbleSetStatusSnapshot) -> anyhow::Result<()> {
        let summaries = self.repository.list_library_summaries(None).await?;
        let target = summaries
            .into_iter()
            .find(|summary| summary.name.eq_ignore_ascii_case(&settings.library_name));
        let mut cache = self.ableset_cache.write().await;
        cache.library_name = Some(settings.library_name.clone());
        cache.song_prefix_length = settings.song_prefix_length;
        cache.entries.clear();
        cache.last_updated = Some(Utc::now());
        cache.last_error = None;
        if let Some(summary) = target {
            for presentation in summary.presentations {
                if let Some(prefix) =
                    extract_song_prefix(&presentation.name, settings.song_prefix_length)
                {
                    cache
                        .entries
                        .insert(prefix.to_ascii_lowercase(), presentation.id);
                }
            }
            if cache.entries.is_empty() {
                cache.last_error = Some("no presentations with valid prefix".to_string());
            }
        } else {
            cache.last_error = Some("library not found".to_string());
        }
        Ok(())
    }

    pub async fn list_resolume_hosts(&self) -> anyhow::Result<Vec<ResolumeHost>> {
        self.repository.list_resolume_hosts().await
    }

    pub async fn list_android_stage_displays(&self) -> anyhow::Result<Vec<AndroidStageDisplay>> {
        self.repository.list_android_stage_displays().await
    }

    pub async fn resolume_status_snapshot(
        &self,
    ) -> HashMap<ResolumeHostId, ResolumeConnectionSnapshot> {
        self.resolume_registry.snapshot().await
    }

    pub async fn android_stage_status_snapshot(
        &self,
    ) -> HashMap<AndroidStageDisplayId, AndroidStageDisplayStatusSnapshot> {
        self.android_stage_registry.snapshot().await
    }

    pub async fn resolume_status_for(&self, id: ResolumeHostId) -> ResolumeConnectionSnapshot {
        self.resolume_registry.snapshot_for(id).await
    }

    pub async fn android_stage_status_for(
        &self,
        id: AndroidStageDisplayId,
    ) -> AndroidStageDisplayStatusSnapshot {
        self.android_stage_registry.snapshot_for(id).await
    }

    pub async fn create_resolume_host(
        &self,
        draft: ResolumeHostDraft,
    ) -> anyhow::Result<ResolumeHost> {
        let host = self.repository.create_resolume_host(&draft).await?;
        self.sync_resolume_hosts().await?;
        Ok(host)
    }

    pub async fn create_android_stage_display(
        &self,
        draft: AndroidStageDisplayDraft,
    ) -> anyhow::Result<AndroidStageDisplay> {
        let display = self.repository.create_android_stage_display(&draft).await?;
        self.sync_android_stage_displays().await?;
        Ok(display)
    }

    pub async fn update_resolume_host(
        &self,
        id: ResolumeHostId,
        draft: ResolumeHostDraft,
    ) -> anyhow::Result<ResolumeHost> {
        let host = self.repository.update_resolume_host(id, &draft).await?;
        self.sync_resolume_hosts().await?;
        Ok(host)
    }

    pub async fn update_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
        draft: AndroidStageDisplayDraft,
    ) -> anyhow::Result<AndroidStageDisplay> {
        let display = self
            .repository
            .update_android_stage_display(id, &draft)
            .await?;
        self.sync_android_stage_displays().await?;
        Ok(display)
    }

    pub async fn delete_resolume_host(&self, id: ResolumeHostId) -> anyhow::Result<()> {
        self.repository.delete_resolume_host(id).await?;
        self.sync_resolume_hosts().await
    }

    pub async fn delete_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
    ) -> anyhow::Result<()> {
        self.repository.delete_android_stage_display(id).await?;
        self.sync_android_stage_displays().await
    }

    pub async fn refresh_default_bible_translations(
        &self,
    ) -> anyhow::Result<Vec<BibleImportSummary>> {
        #[cfg(test)]
        if let Some(ingestion) = &self.bible_ingestion_override {
            return ingestion.ingest_default_translations().await;
        }

        let service = BibleIngestionService::with_http(&self.repository)?;
        service.ingest_default_translations().await
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    pub fn live_hub(&self) -> LiveHub {
        self.live_hub.clone()
    }

    pub fn stage_connections_handle(&self) -> StageConnections {
        self.stage_connections.clone()
    }

    pub fn heartbeat_config(&self) -> StageHeartbeatConfig {
        self.heartbeat_config
    }

    pub async fn stage_connections_snapshot(&self) -> Vec<StageClientSnapshot> {
        self.stage_connections.snapshot().await
    }

    pub fn companion_token(&self) -> Option<&str> {
        self.companion_token.as_deref()
    }

    pub fn companion_enabled(&self) -> bool {
        self.companion_enabled.load(Ordering::SeqCst)
    }

    pub fn companion_port(&self) -> u16 {
        self.companion_port.load(Ordering::SeqCst)
    }

    pub fn line_limit(&self) -> u16 {
        self.line_limit.load(Ordering::SeqCst)
    }

    pub async fn set_line_limit(&self, value: u16) -> anyhow::Result<()> {
        let clamped = clamp_line_limit(value);
        self.repository
            .set_app_setting(LINE_LIMIT_KEY, &clamped.to_string())
            .await?;
        self.line_limit.store(clamped, Ordering::SeqCst);
        Ok(())
    }

    pub fn feature_flags(&self) -> FeatureFlags {
        FeatureFlags {
            companion_enabled: self.companion_enabled(),
            companion_port: self.companion_port(),
            line_limit: self.line_limit(),
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
            let _ = self
                .configure_companion_service(previous_enabled, previous_port)
                .await;
            return Err(err);
        }

        if let Err(err) = self
            .repository
            .set_app_setting(COMPANION_FEATURE_KEY, if enabled { "1" } else { "0" })
            .await
        {
            let _ = self
                .repository
                .set_app_setting(COMPANION_PORT_KEY, &previous_port.to_string())
                .await;
            let _ = self
                .configure_companion_service(previous_enabled, previous_port)
                .await;
            return Err(err);
        }

        self.companion_enabled.store(enabled, Ordering::SeqCst);
        self.companion_port.store(port, Ordering::SeqCst);
        Ok(())
    }

    pub async fn active_bible_broadcast(&self) -> Option<BibleBroadcast> {
        self.bible_broadcast.read().await.clone()
    }

    pub async fn trigger_bible_passage(
        &self,
        translation_code: &str,
        reference: &BibleReference,
    ) -> anyhow::Result<BibleBroadcast> {
        let passage = self
            .repository
            .find_bible_passage(translation_code, reference)
            .await?
            .ok_or_else(|| anyhow::anyhow!("passage not found"))?;

        let broadcast = BibleBroadcast::new(passage, Utc::now());
        {
            let mut guard = self.bible_broadcast.write().await;
            *guard = Some(broadcast.clone());
        }
        self.live_hub.publish(LiveEvent::Bible {
            broadcast: broadcast.clone(),
        });
        self.resolume_registry
            .bible_update(BibleUpdate {
                passage: Some(broadcast.clone()),
            })
            .await;
        Ok(broadcast)
    }

    pub async fn clear_bible_broadcast(&self) {
        {
            let mut guard = self.bible_broadcast.write().await;
            *guard = None;
        }
        self.live_hub.publish(LiveEvent::BibleCleared);
        self.resolume_registry
            .bible_update(BibleUpdate { passage: None })
            .await;
    }

    #[cfg(test)]
    pub fn set_test_bible_ingestion(
        &mut self,
        ingestion: std::sync::Arc<dyn TestBibleIngestion + Send + Sync>,
    ) {
        self.bible_ingestion_override = Some(ingestion);
    }

    async fn ensure_demo_playlist(&self) -> anyhow::Result<()> {
        let existing = self.repository.list_playlists().await?;
        let mut seed_required = existing.is_empty();

        if !seed_required {
            seed_required = existing.iter().all(|playlist| {
                playlist
                    .entries
                    .iter()
                    .all(|entry| !matches!(entry.kind, PlaylistEntryKind::Presentation { .. }))
            });
        }

        if seed_required {
            for playlist in existing {
                let _ = self.repository.delete_playlist(playlist.id).await;
            }
        } else {
            return Ok(());
        }

        let libraries = self.repository.fetch_libraries().await?;
        let Some(library) = libraries.first() else {
            return Ok(());
        };

        let entries: Vec<PlaylistEntry> = library
            .presentations
            .iter()
            .take(5)
            .map(|presentation| PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Presentation {
                    presentation_id: presentation.id,
                    midi_binding: None,
                },
            })
            .collect();

        if entries.is_empty() {
            return Ok(());
        }

        let playlist = self
            .repository
            .create_playlist("Ableton Demo", true)
            .await?;
        self.repository
            .replace_playlist_entries(playlist.id, &entries)
            .await?;
        Ok(())
    }

    async fn ensure_seed_library(&self) -> anyhow::Result<()> {
        if self.repository.fetch_libraries().await?.is_empty() {
            self.repository.upsert_library(&sample_library()).await?;
        }
        Ok(())
    }

    async fn sync_resolume_hosts(&self) -> anyhow::Result<()> {
        let hosts = self.repository.list_resolume_hosts().await?;
        self.resolume_registry.set_hosts(hosts).await;
        Ok(())
    }

    async fn sync_android_stage_displays(&self) -> anyhow::Result<()> {
        let displays = self.repository.list_android_stage_displays().await?;
        self.android_stage_registry.set_displays(displays).await;
        Ok(())
    }

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

    async fn cache_presentation_ref(&self, presentation: &Presentation) {
        let mut guard = self.presentation_cache.write().await;
        guard.insert(presentation.id, Arc::new(presentation.clone()));
    }

    async fn cache_presentation_value(&self, presentation: Presentation) {
        let mut guard = self.presentation_cache.write().await;
        guard.insert(presentation.id, Arc::new(presentation));
    }

    pub async fn stage_display_snapshot(
        &self,
        layout_code: &str,
    ) -> anyhow::Result<Option<StageDisplaySnapshot>> {
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == layout_code);
        let Some(layout) = layout else {
            return Ok(None);
        };
        let Some(context) = self.build_stage_context().await? else {
            return Ok(None);
        };
        Ok(Some(build_stage_snapshot(layout, &context)))
    }

    pub async fn selected_stage_display_snapshot(
        &self,
    ) -> anyhow::Result<Option<StageDisplaySnapshot>> {
        let code = {
            let guard = self.stage_layout.read().await;
            guard.clone()
        };
        self.stage_display_snapshot(&code).await
    }

    pub async fn stage_layout_code(&self) -> String {
        self.stage_layout.read().await.clone()
    }

    pub async fn set_stage_layout_code(&self, code: &str) -> anyhow::Result<StageDisplayLayout> {
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == code)
            .ok_or_else(|| anyhow::anyhow!("unknown stage layout: {code}"))?;
        {
            let mut guard = self.stage_layout.write().await;
            if *guard == layout.code {
                return Ok(layout);
            }
            *guard = layout.code.clone();
        }
        self.live_hub.publish(LiveEvent::StageLayout {
            code: layout.code.clone(),
        });
        self.broadcast_stage_snapshots().await?;
        Ok(layout)
    }

    pub async fn stage_displays(&self) -> anyhow::Result<Vec<StageDisplayLayout>> {
        Ok(StageDisplayLayout::built_in())
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
        );
        self.repository.upsert_stage_state(&stage_state).await?;
        let resolution = stage_resolution_from_presentation(
            &presentation,
            Some(library_name),
            Some(current_slide_id),
            next_slide_id,
        );
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

    pub async fn presentation_detail(
        &self,
        presentation_id: PresentationId,
    ) -> anyhow::Result<Option<(LibraryId, String, Presentation)>> {
        let detail = self
            .repository
            .fetch_presentation_detail(presentation_id)
            .await?;
        if let Some((library_id, library_name, presentation)) = detail {
            self.cache_presentation_ref(&presentation).await;
            Ok(Some((library_id, library_name, presentation)))
        } else {
            Ok(None)
        }
    }

    pub async fn update_slide_content(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
        main: String,
        translation: String,
        stage: String,
        group: Option<String>,
    ) -> anyhow::Result<Slide> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();

        let existing_slide = presentation
            .slides
            .iter()
            .find(|slide| slide.id == slide_id)
            .ok_or_else(|| anyhow::anyhow!("slide not found"))?
            .clone();

        let main_text = SlideText::new(main).map_err(|err| anyhow::anyhow!(err))?;
        let translation_text = SlideText::new(translation).map_err(|err| anyhow::anyhow!(err))?;
        let stage_text = SlideText::new(stage).map_err(|err| anyhow::anyhow!(err))?;
        let group = group.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(SlideGroup::new(trimmed.to_string()))
            }
        });

        let content = SlideContent::new(
            main_text.clone(),
            translation_text.clone(),
            stage_text.clone(),
            group.clone(),
        );
        let updated_slide = Slide::new(existing_slide.order, content.clone()).with_id(slide_id);

        self.repository
            .update_slide_content(presentation_id, slide_id, &content)
            .await?;

        let mut updated_presentation = presentation.clone();
        if let Some(slot) = updated_presentation
            .slides
            .iter_mut()
            .find(|slide| slide.id == slide_id)
        {
            *slot = updated_slide.clone();
        }
        self.cache_presentation_value(updated_presentation).await;

        self.broadcast_stage_snapshots().await?;

        Ok(updated_slide)
    }

    pub async fn insert_blank_slide(
        &self,
        presentation_id: PresentationId,
        position: Option<u32>,
    ) -> anyhow::Result<Vec<Slide>> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();
        let mut slides = presentation.slides.clone();
        let insert_at = position
            .map(|value| value as usize)
            .unwrap_or(slides.len())
            .min(slides.len());
        slides.insert(insert_at, Slide::new(0, blank_slide_content()));
        Self::reindex_slides(&mut slides);
        self.repository
            .replace_presentation_slides(presentation_id, &slides)
            .await?;
        self.reconcile_stage_state_after_edit(presentation_id, &slides)
            .await?;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
        Ok(slides)
    }

    pub async fn duplicate_slide(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
    ) -> anyhow::Result<Vec<Slide>> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();
        let mut slides = presentation.slides.clone();
        let index = slides
            .iter()
            .position(|slide| slide.id == slide_id)
            .ok_or_else(|| anyhow::anyhow!("slide not found"))?;
        let source = slides[index].clone();
        slides.insert(index + 1, Slide::new(0, source.content.clone()));
        Self::reindex_slides(&mut slides);
        self.repository
            .replace_presentation_slides(presentation_id, &slides)
            .await?;
        self.reconcile_stage_state_after_edit(presentation_id, &slides)
            .await?;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
        Ok(slides)
    }

    pub async fn delete_slide(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
    ) -> anyhow::Result<Vec<Slide>> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();
        let mut slides = presentation.slides.clone();
        let index = slides
            .iter()
            .position(|slide| slide.id == slide_id)
            .ok_or_else(|| anyhow::anyhow!("slide not found"))?;
        slides.remove(index);
        if slides.is_empty() {
            slides.push(Slide::new(0, blank_slide_content()));
        }
        Self::reindex_slides(&mut slides);
        self.repository
            .replace_presentation_slides(presentation_id, &slides)
            .await?;
        self.reconcile_stage_state_after_edit(presentation_id, &slides)
            .await?;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
        Ok(slides)
    }

    pub async fn reorder_slides(
        &self,
        presentation_id: PresentationId,
        order: Vec<SlideId>,
    ) -> anyhow::Result<Vec<Slide>> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();
        let mut map = HashMap::new();
        for slide in presentation.slides.clone() {
            map.insert(slide.id, slide);
        }
        if order.len() != map.len() {
            return Err(anyhow::anyhow!("slide order length mismatch"));
        }
        let mut slides = Vec::with_capacity(order.len());
        for id in order {
            let slide = map
                .remove(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown slide in reorder request"))?;
            slides.push(slide);
        }
        Self::reindex_slides(&mut slides);
        self.repository
            .replace_presentation_slides(presentation_id, &slides)
            .await?;
        self.reconcile_stage_state_after_edit(presentation_id, &slides)
            .await?;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
        Ok(slides)
    }

    fn publish_stage_update(&self, snapshot: StageDisplaySnapshot) {
        self.live_hub.publish(LiveEvent::Stage { snapshot });
    }

    pub async fn execute_timer_command(
        &self,
        command: TimerCommand,
    ) -> anyhow::Result<TimersOverview> {
        let now = Utc::now();
        let mut state = self.load_or_init_timers(now).await?;
        state.apply_command(&command, now)?;
        self.repository.upsert_timers_state(&state).await?;
        let overview = state.overview(now);
        self.live_hub.publish(LiveEvent::Timers {
            overview: overview.clone(),
        });
        self.broadcast_stage_snapshots().await?;
        Ok(overview)
    }

    pub async fn tick_timers(&self) -> anyhow::Result<()> {
        let now = Utc::now();
        let mut state = self.load_or_init_timers(now).await?;
        let mut state_changed = false;
        if state.countdown.state == TimerState::Running {
            let previous = state.countdown.state;
            state.countdown.update_state(now);
            state_changed = state.countdown.state != previous;
        }

        let overview = state.overview(now);

        if state_changed {
            self.repository.upsert_timers_state(&state).await?;
        }

        let formatted = format_countdown_text(overview.countdown_to_start.seconds_remaining);
        self.resolume_registry
            .timer_update(TimerFrame::new(formatted))
            .await;

        self.live_hub.publish(LiveEvent::Timers { overview });

        Ok(())
    }

    async fn load_or_init_timers(&self, now: DateTime<Utc>) -> anyhow::Result<TimersState> {
        if let Some(state) = self.repository.get_timers_state().await? {
            Ok(state)
        } else {
            let state = TimersState::default(now);
            self.repository.upsert_timers_state(&state).await?;
            self.live_hub.publish(LiveEvent::Timers {
                overview: state.overview(now),
            });
            Ok(state)
        }
    }

    async fn sample_resolume_latency(&self) -> Option<f64> {
        let snapshot = self.resolume_registry.snapshot().await;
        snapshot
            .values()
            .filter_map(|status| status.last_latency_ms)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    }

    fn reindex_slides(slides: &mut Vec<Slide>) {
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
            id.map_or(true, |target| slides.iter().any(|slide| slide.id == target))
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

    async fn broadcast_stage_snapshots(&self) -> anyhow::Result<()> {
        let Some(context) = self.build_stage_context().await? else {
            return Ok(());
        };
        self.publish_stage_context(&context).await?;
        Ok(())
    }

    async fn broadcast_stage_resolution(&self, resolution: StageResolution) -> anyhow::Result<()> {
        let now = Utc::now();
        let timers_state = self.load_or_init_timers(now).await?;
        let latency_ms = self.sample_resolume_latency().await;
        let playlist = self.resolve_stage_playlist(&resolution).await?;
        let context = StageContext {
            generated_at: now,
            overview: timers_state.overview(now),
            resolution,
            latency_ms,
            playlist,
        };
        self.publish_stage_context(&context).await?;
        let current_main = context
            .resolution
            .current
            .as_ref()
            .map(|slide| slide.main.clone())
            .unwrap_or_else(String::new);
        let current_translation = context
            .resolution
            .current
            .as_ref()
            .map(|slide| slide.translation.clone())
            .unwrap_or_else(String::new);
        let song_name = context
            .resolution
            .presentation_name
            .clone()
            .map(|name| sanitize_song_title(&name))
            .unwrap_or_default();
        let band_name = context.resolution.library_name.clone().unwrap_or_default();
        let stage_update = StageUpdate {
            current_main: Some(current_main),
            current_translation: Some(current_translation),
            song_name: Some(song_name),
            band_name: Some(band_name),
        };
        self.resolume_registry.stage_update(stage_update).await;
        Ok(())
    }

    async fn publish_stage_context(&self, context: &StageContext) -> anyhow::Result<()> {
        let code = self.stage_layout_code().await;
        let mut layouts = StageDisplayLayout::built_in()
            .into_iter()
            .map(|layout| (layout.code.clone(), layout))
            .collect::<HashMap<_, _>>();
        let Some(layout) = layouts
            .remove(&code)
            .or_else(|| layouts.remove("worship-snv"))
        else {
            return Ok(());
        };
        let snapshot = build_stage_snapshot(layout, context);
        self.publish_stage_update(snapshot);
        Ok(())
    }

    async fn build_stage_context(&self) -> anyhow::Result<Option<StageContext>> {
        let now = Utc::now();
        let timers_state = self.load_or_init_timers(now).await?;
        let overview = timers_state.overview(now);
        let stage_state = self.repository.get_stage_state().await?;

        let resolution = if let Some(state) = stage_state {
            match self.resolve_stage_from_state(&state).await? {
                Some(resolution) => resolution,
                None => match self.resolve_default_stage().await? {
                    Some(resolution) => resolution,
                    None => return Ok(None),
                },
            }
        } else if let Some(resolution) = self.resolve_default_stage().await? {
            resolution
        } else {
            return Ok(None);
        };

        let playlist = self.resolve_stage_playlist(&resolution).await?;
        let latency_ms = self.sample_resolume_latency().await;
        Ok(Some(StageContext {
            generated_at: now,
            overview,
            resolution,
            latency_ms,
            playlist,
        }))
    }
    async fn resolve_stage_playlist(
        &self,
        resolution: &StageResolution,
    ) -> anyhow::Result<Option<StagePlaylistSummary>> {
        let Some(presentation_id) = resolution.presentation_id else {
            return Ok(None);
        };
        let playlists = self.repository.list_playlists().await?;
        for playlist in playlists {
            let mut entries = Vec::new();
            let mut contains_current = false;
            for entry in playlist.entries {
                match entry.kind {
                    PlaylistEntryKind::Presentation {
                        presentation_id: entry_id,
                        ..
                    } => {
                        let presentation = self.presentation_from_cache(entry_id).await?;
                        let name = sanitize_song_title(&presentation.name);
                        let is_current = entry_id == presentation_id;
                        if is_current {
                            contains_current = true;
                        }
                        entries.push(StagePlaylistEntry {
                            presentation_id: entry_id,
                            name,
                            is_current,
                        });
                    }
                    PlaylistEntryKind::Separator { .. } => continue,
                }
            }
            if contains_current && !entries.is_empty() {
                return Ok(Some(StagePlaylistSummary {
                    name: playlist.name,
                    entries,
                }));
            }
        }
        Ok(None)
    }

    async fn resolve_stage_from_state(
        &self,
        stage_state: &StageState,
    ) -> anyhow::Result<Option<StageResolution>> {
        let Some(presentation_id) = stage_state.presentation_id else {
            return Ok(Some(StageResolution::cleared()));
        };
        let detail = self
            .repository
            .fetch_presentation_detail(presentation_id)
            .await?;
        let Some((_, library_name, presentation)) = detail else {
            return Ok(None);
        };
        let resolution = stage_resolution_from_presentation(
            &presentation,
            Some(library_name),
            stage_state.current_slide_id,
            stage_state.next_slide_id,
        );
        Ok(Some(resolution))
    }

    async fn resolve_default_stage(&self) -> anyhow::Result<Option<StageResolution>> {
        let detail = self.repository.fetch_first_presentation_detail().await?;
        let Some((_, library_name, presentation)) = detail else {
            return Ok(None);
        };
        let resolution =
            stage_resolution_from_presentation(&presentation, Some(library_name), None, None);
        Ok(Some(resolution))
    }
}

#[cfg(test)]
#[async_trait::async_trait]
pub trait TestBibleIngestion {
    async fn ingest_default_translations(
        &self,
    ) -> anyhow::Result<Vec<presenter_bible::BibleImportSummary>>;
}

fn sample_library() -> Library {
    let presentation = Presentation::new(
        "Welcome",
        vec![
            Slide::new(
                0,
                SlideContent::new(
                    SlideText::new("Welcome to service").unwrap(),
                    SlideText::new("Vitajte").unwrap(),
                    SlideText::new("Stage cue").unwrap(),
                    Some(SlideGroup::new("Intro")),
                ),
            )
            .with_id(SlideId::new()),
            Slide::new(
                1,
                SlideContent::new(
                    SlideText::new("Let's worship").unwrap(),
                    SlideText::new("Poďme chváliť").unwrap(),
                    SlideText::new("Cue").unwrap(),
                    None,
                ),
            )
            .with_id(SlideId::new()),
        ],
    )
    .unwrap()
    .with_id(PresentationId::new());

    Library::new("Sample Library", vec![presentation])
        .unwrap()
        .with_id(LibraryId::new())
}

#[derive(Debug, Clone)]
struct StageContext {
    generated_at: DateTime<Utc>,
    overview: TimersOverview,
    resolution: StageResolution,
    latency_ms: Option<f64>,
    playlist: Option<StagePlaylistSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StageResolution {
    presentation_id: Option<PresentationId>,
    presentation_name: Option<String>,
    library_name: Option<String>,
    current_slide_id: Option<SlideId>,
    current: Option<StageDisplaySlide>,
    next_slide_id: Option<SlideId>,
    next: Option<StageDisplaySlide>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_slides: Option<u32>,
}

impl StageResolution {
    fn cleared() -> Self {
        Self {
            presentation_id: None,
            presentation_name: None,
            library_name: None,
            current_slide_id: None,
            current: None,
            next_slide_id: None,
            next: None,
            current_index: None,
            total_slides: None,
        }
    }
}

fn stage_resolution_from_presentation(
    presentation: &Presentation,
    library_name: Option<String>,
    current_slide_id: Option<SlideId>,
    next_slide_id: Option<SlideId>,
) -> StageResolution {
    #[derive(Clone)]
    struct SlideCtx<'a> {
        slide: &'a Slide,
        effective_group: Option<String>,
    }

    fn to_stage_display(ctx: &SlideCtx<'_>) -> StageDisplaySlide {
        StageDisplaySlide {
            main: ctx.slide.content.main.value().to_string(),
            translation: ctx.slide.content.translation.value().to_string(),
            stage: ctx.slide.content.stage.value().to_string(),
            group: ctx.effective_group.clone(),
        }
    }

    let total_slides = presentation.slides.len() as u32;

    if presentation.slides.is_empty() {
        return StageResolution {
            presentation_id: Some(presentation.id),
            presentation_name: Some(presentation.name.clone()),
            library_name,
            current_slide_id: None,
            current: None,
            next_slide_id: None,
            next: None,
            current_index: None,
            total_slides: Some(total_slides),
        };
    }

    let mut effective_group: Option<String> = None;
    let mut first: Option<SlideCtx<'_>> = None;
    let mut second: Option<SlideCtx<'_>> = None;
    let mut current_ctx: Option<SlideCtx<'_>> = None;
    let mut current_order: Option<u32> = None;
    let mut next_by_id: Option<SlideCtx<'_>> = None;
    let mut next_after_current: Option<SlideCtx<'_>> = None;

    for slide in &presentation.slides {
        if let Some(group) = slide.content.group.as_ref() {
            effective_group = Some(group.name().to_string());
        }
        let ctx = SlideCtx {
            slide,
            effective_group: effective_group.clone(),
        };
        if first.is_none() {
            first = Some(ctx.clone());
        } else if second.is_none() {
            second = Some(ctx.clone());
        }

        if let Some(target_next) = next_slide_id {
            if slide.id == target_next {
                next_by_id = Some(ctx.clone());
            }
        }

        if current_ctx.is_none() {
            if let Some(target_current) = current_slide_id {
                if slide.id == target_current {
                    current_order = Some(slide.order);
                    current_ctx = Some(ctx.clone());
                }
            }
        } else if next_after_current.is_none() {
            if let Some(order) = current_order {
                if slide.order > order {
                    next_after_current = Some(ctx.clone());
                }
            }
        }
    }

    let resolved_current = current_ctx.or_else(|| first.clone());
    let resolved_next = if let Some(next_ctx) = next_by_id {
        Some(next_ctx)
    } else if current_order.is_some() {
        next_after_current.clone()
    } else {
        second.clone()
    };

    let current_slide_id_value = resolved_current.as_ref().map(|ctx| ctx.slide.id);
    let next_slide_id_value = resolved_next.as_ref().map(|ctx| ctx.slide.id);
    let current_slide = resolved_current.as_ref().map(to_stage_display);
    let next_slide = resolved_next.as_ref().map(to_stage_display);

    let current_index_value = resolved_current
        .as_ref()
        .and_then(|ctx| {
            presentation
                .slides
                .iter()
                .position(|slide| slide.id == ctx.slide.id)
        })
        .map(|index| index as u32 + 1);

    StageResolution {
        presentation_id: Some(presentation.id),
        presentation_name: Some(presentation.name.clone()),
        library_name,
        current_slide_id: current_slide_id_value,
        current: current_slide,
        next_slide_id: next_slide_id_value,
        next: next_slide,
        current_index: current_index_value,
        total_slides: Some(total_slides),
    }
}

fn build_stage_snapshot(
    layout: StageDisplayLayout,
    context: &StageContext,
) -> StageDisplaySnapshot {
    StageDisplaySnapshot::new(
        layout,
        context.generated_at,
        context.resolution.presentation_id,
        context.resolution.presentation_name.clone(),
        context.resolution.library_name.clone(),
        context
            .resolution
            .presentation_name
            .clone()
            .map(|name| sanitize_song_title(&name)),
        context.resolution.current_slide_id,
        context.resolution.current.clone(),
        context.resolution.next_slide_id,
        context.resolution.next.clone(),
        context.overview.clone(),
        context.latency_ms,
        context.resolution.current_index,
        context.resolution.total_slides,
        context.playlist.clone(),
    )
}

fn sanitize_song_title(name: &str) -> String {
    let trimmed = name.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 4
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_whitespace()
    {
        let remainder = trimmed[4..].trim_start();
        remainder.to_string()
    } else {
        trimmed.to_string()
    }
}

fn blank_slide_content() -> SlideContent {
    SlideContent::new(
        SlideText::new("").expect("empty main within limit"),
        SlideText::new("").expect("empty translation within limit"),
        SlideText::new("").expect("empty stage within limit"),
        None,
    )
}

#[derive(Default)]
struct AbleSetLibraryCache {
    library_name: Option<String>,
    song_prefix_length: u8,
    entries: HashMap<String, PresentationId>,
    last_updated: Option<DateTime<Utc>>,
    last_error: Option<String>,
}

impl AbleSetLibraryCache {
    #[allow(dead_code)]
    fn invalidate(&mut self) {
        self.entries.clear();
        self.library_name = None;
        self.song_prefix_length = 0;
        self.last_updated = None;
        self.last_error = None;
    }

    #[allow(dead_code)]
    fn matches(&self, library_name: &str, prefix_len: u8) -> bool {
        if let Some(current) = &self.library_name {
            current.eq_ignore_ascii_case(library_name) && self.song_prefix_length == prefix_len
        } else {
            false
        }
    }
}

fn format_countdown_text(seconds_remaining: i64) -> String {
    let total = seconds_remaining.max(0);
    if total < 60 {
        total.to_string()
    } else {
        let minutes = total / 60;
        let seconds = total % 60;
        format!("{minutes:02}:{seconds:02}")
    }
}
#[cfg(test)]
mod tests;
