mod ableset;
mod broadcasting;
mod companion;
mod integrations;
mod seed;
mod slides;
mod stage;
mod timers;
#[cfg(test)]
mod tests;

use crate::{
    ableset::AbleSetBridge,
    android_stage::AndroidStageRegistry,
    live::{LiveEvent, LiveHub},
    osc::{OscBridge, OscStatusSnapshot},
    resolume::{BibleUpdate, ResolumeRegistry},
    stage_connections::{StageClientSnapshot, StageConnections, StageHeartbeatConfig},
};
use chrono::Utc;
use presenter_bible::BibleImportSummary;
use presenter_core::playlist::PlaylistEntryKind;
use presenter_core::{
    BibleBroadcast, BibleReference, BibleTranslation, Library, LibraryId, LibrarySummary,
    OscSettings, OscSettingsDraft, Playlist, PlaylistEntry, PlaylistEntryId, PlaylistId,
    Presentation, PresentationId, SearchResult, Slide, SlideId, StageDisplayLayout,
    StageDisplaySnapshot, StageState, TimersOverview,
};
use presenter_importer::bible::BibleIngestionService;
use presenter_persistence::{DatabaseSettings, Repository};
use serde::Serialize;
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
pub use seed::TestBibleIngestion;
use seed::sample_library;
use stage::{build_stage_snapshot, stage_resolution_from_presentation, StageResolution};

#[derive(Clone)]
pub struct AppState {
    repository: Repository,
    live_hub: LiveHub,
    bible_broadcast: Arc<RwLock<Option<BibleBroadcast>>>,
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
    #[cfg(test)]
    bible_ingestion_override: Option<std::sync::Arc<dyn TestBibleIngestion + Send + Sync>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlags {
    pub companion_enabled: bool,
    pub companion_port: u16,
}

impl AppState {
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

        let registry = ResolumeRegistry::new();
        let android_stage_registry = AndroidStageRegistry::new();
        let osc_bridge = OscBridge::new();
        let ableset_bridge = AbleSetBridge::new();
        let state = Self::new(
            repo,
            companion_token,
            companion_enabled,
            companion_port,
            registry,
            android_stage_registry,
            osc_bridge.clone(),
            ableset_bridge.clone(),
        );
        state.ensure_seed_library().await?;
        state.ensure_demo_playlist().await?;
        state.sync_resolume_hosts().await?;
        state.sync_android_stage_displays().await?;

        Self::apply_osc_settings(&state, &osc_bridge).await?;
        Self::apply_ableset_settings(&state, &ableset_bridge).await?;

        state
            .configure_companion_service(companion_enabled, companion_port)
            .await?;
        state.spawn_background_tasks();
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
        let registry = ResolumeRegistry::new();
        let android_stage_registry = AndroidStageRegistry::new();
        let osc_bridge = OscBridge::new();
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
        state.ensure_seed_library().await?;
        state.ensure_demo_playlist().await?;
        state.sync_android_stage_displays().await?;

        Self::apply_osc_settings(&state, &osc_bridge).await?;
        Self::apply_ableset_settings(&state, &ableset_bridge).await?;

        state
            .configure_companion_service(false, DEFAULT_COMPANION_PORT)
            .await?;
        Ok(state)
    }

    // Library methods
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

    // Playlist methods
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

    // Bible methods
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

    // Bible ingestion
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

    // Bible broadcast methods
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

    // Demo/seed data
    async fn ensure_demo_playlist(&self) -> anyhow::Result<()> {
        if !self.repository.list_playlists().await?.is_empty() {
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

    async fn cache_presentation_ref(&self, presentation: &Presentation) {
        let mut guard = self.presentation_cache.write().await;
        guard.insert(presentation.id, Arc::new(presentation.clone()));
    }

    async fn cache_presentation_value(&self, presentation: Presentation) {
        let mut guard = self.presentation_cache.write().await;
        guard.insert(presentation.id, Arc::new(presentation));
    }

    // Stage display methods
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

    // Slide editing methods are in slides.rs
    // Timer methods are in timers.rs
    // Broadcasting methods are in broadcasting.rs

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
}
