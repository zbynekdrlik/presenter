use crate::{
    ableset::{AbleSetBridge, DynAbleSetClient},
    android_stage::{AndroidStageRegistry, DynAndroidStageClient},
    config::{parse_bool_flag, ServerConfig},
    live::{LiveEvent, LiveHub},
    osc::{DynOscClient, OscBridge, OscStatusSnapshot},
    resolume::{BibleUpdate, DynResolumeClient, ResolumeRegistry},
    stage_connections::{StageClientSnapshot, StageConnections, StageHeartbeatConfig},
};
use chrono::Utc;
use presenter_bible::BibleImportSummary;
use presenter_core::playlist::PlaylistEntryKind;
use presenter_core::{
    BibleBroadcast, BibleReference, BibleTranslation, Library, LibraryId, LibrarySummary,
    OscSettings, OscSettingsDraft, Playlist, PlaylistEntry, PlaylistEntryId, PlaylistId,
    Presentation, PresentationId, SearchResult, Slide, SlideContent, SlideGroup, SlideId,
    SlideText, StageDisplayLayout,
};
use presenter_importer::bible::BibleIngestionService;
use presenter_persistence::{DatabaseSettings, Repository};
use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, atomic::AtomicU16, Arc},
};
use tokio::sync::RwLock;
use tokio::time::{interval, Duration as TokioDuration, MissedTickBehavior};
use tracing::{instrument, warn};
use uuid::Uuid;

use super::{AbleSetLibraryCache, CompanionServerManager};

pub(super) const COMPANION_FEATURE_KEY: &str = "feature.companion.enabled";
pub(super) const COMPANION_PORT_KEY: &str = "feature.companion.port";
pub(super) const DEFAULT_COMPANION_PORT: u16 = 18_175;

#[derive(Clone)]
pub struct AppState {
    pub(super) repository: Repository,
    pub(super) live_hub: LiveHub,
    pub(super) bible_broadcast: Arc<RwLock<Option<BibleBroadcast>>>,
    pub(super) companion_token: Option<String>,
    pub(super) companion_enabled: Arc<AtomicBool>,
    pub(super) companion_port: Arc<AtomicU16>,
    pub(super) companion_server: CompanionServerManager,
    pub(super) resolume_client: DynResolumeClient,
    pub(super) android_stage_client: DynAndroidStageClient,
    pub(super) stage_connections: StageConnections,
    pub(super) heartbeat_config: StageHeartbeatConfig,
    pub(super) presentation_cache: Arc<RwLock<HashMap<PresentationId, Arc<Presentation>>>>,
    pub(super) stage_layout: Arc<RwLock<String>>,
    pub(super) osc_client: DynOscClient,
    pub(super) ableset_client: DynAbleSetClient,
    pub(super) ableset_cache: Arc<RwLock<AbleSetLibraryCache>>,
    #[cfg(test)]
    pub(super) bible_ingestion_override:
        Option<std::sync::Arc<dyn TestBibleIngestion + Send + Sync>>,
}

impl AppState {
    pub fn new(
        repository: Repository,
        companion_token: Option<String>,
        companion_enabled: bool,
        companion_port: u16,
        resolume_client: DynResolumeClient,
        android_stage_client: DynAndroidStageClient,
        osc_client: DynOscClient,
        ableset_client: DynAbleSetClient,
        heartbeat_config: StageHeartbeatConfig,
    ) -> Self {
        let stage_connections = StageConnections::new();
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
            resolume_client,
            android_stage_client,
            stage_connections,
            heartbeat_config,
            presentation_cache: Arc::new(RwLock::new(HashMap::new())),
            stage_layout: Arc::new(RwLock::new(default_layout)),
            osc_client,
            ableset_client,
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
    pub async fn from_config(config: ServerConfig) -> anyhow::Result<Self> {
        let db_url = config.database.url.clone();
        let repository = Repository::connect(&DatabaseSettings::new(&db_url)).await?;
        Self::build_with_repository(repository, Arc::new(config), true).await
    }

    #[cfg(test)]
    #[instrument(skip_all)]
    pub async fn in_memory() -> anyhow::Result<Self> {
        let config = ServerConfig::load()?;
        Self::in_memory_with_config(config).await
    }

    #[cfg(test)]
    #[instrument(skip_all)]
    pub async fn in_memory_with_config(config: ServerConfig) -> anyhow::Result<Self> {
        let repository = Repository::connect_in_memory().await?;
        Self::build_with_repository(repository, Arc::new(config), false).await
    }

    async fn build_with_repository(
        repository: Repository,
        config: Arc<ServerConfig>,
        spawn_background_tasks: bool,
    ) -> anyhow::Result<Self> {
        let companion_token = config.companion.token.clone();
        let stored_companion = repository
            .get_app_setting(COMPANION_FEATURE_KEY)
            .await?
            .and_then(|value| parse_bool_flag(&value))
            .unwrap_or(false);
        let env_override = config.companion.enabled_override;
        let companion_enabled = env_override.unwrap_or(stored_companion);
        let persist_enabled = env_override.is_some();

        let raw_port = repository.get_app_setting(COMPANION_PORT_KEY).await?;
        let stored_port = raw_port
            .as_deref()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port >= 1 && *port <= u16::MAX);
        let mut companion_port = stored_port.unwrap_or(DEFAULT_COMPANION_PORT);
        let mut persist_port = stored_port.is_none();

        if let Some(port_override) = config.companion.port_override {
            companion_port = port_override;
            persist_port = true;
        }

        let heartbeat_config = config.stage.heartbeat;
        let resolume_client: DynResolumeClient = Arc::new(ResolumeRegistry::new());
        let android_stage_client: DynAndroidStageClient =
            Arc::new(AndroidStageRegistry::from_config(&config.android));
        let osc_client: DynOscClient = Arc::new(OscBridge::new(&config.osc));
        let ableset_client: DynAbleSetClient = Arc::new(AbleSetBridge::new());
        let state = Self::new(
            repository,
            companion_token,
            companion_enabled,
            companion_port,
            resolume_client,
            android_stage_client,
            osc_client,
            ableset_client,
            heartbeat_config,
        );

        if persist_enabled {
            state
                .repository
                .set_app_setting(
                    COMPANION_FEATURE_KEY,
                    if companion_enabled { "1" } else { "0" },
                )
                .await?;
        }

        if persist_port {
            state
                .repository
                .set_app_setting(COMPANION_PORT_KEY, &companion_port.to_string())
                .await?;
        }

        state.ensure_seed_library().await?;
        state.ensure_demo_playlist().await?;
        if spawn_background_tasks {
            state.sync_resolume_hosts().await?;
        }
        state.sync_android_stage_displays().await?;

        let mut osc_settings = state.repository.get_osc_settings().await?;
        if let Some(invalid) = config.osc.listen_port_invalid.as_ref() {
            tracing::warn!(value = %invalid, "invalid PRESENTER_OSC_LISTEN_PORT");
        }
        if let Some(port_override) = config.osc.listen_port_override {
            if port_override != 0 && port_override != osc_settings.listen_port {
                let draft = OscSettingsDraft {
                    enabled: osc_settings.enabled,
                    listen_port: port_override,
                    address_pattern: osc_settings.address_pattern.clone(),
                    velocity_mode: osc_settings.velocity_mode,
                };
                osc_settings = state.repository.upsert_osc_settings(&draft).await?;
            }
        }

        if let Err(err) = state
            .osc_client
            .apply_settings(osc_settings.clone(), state.clone())
            .await
        {
            tracing::warn!(?err, "failed to initialise OSC listener");
        }
        let ableset_settings = state.repository.get_ableset_settings().await?;
        match state
            .ableset_client
            .apply_settings(ableset_settings.clone())
            .await
        {
            Ok(()) => {
                let snapshot = state.ableset_client.status_snapshot().await;
                if let Err(err) = state.refresh_ableset_cache(&snapshot).await {
                    tracing::warn!(?err, "failed to seed AbleSet cache");
                }
            }
            Err(err) => tracing::warn!(?err, "failed to initialise AbleSet tracker"),
        }
        state
            .configure_companion_service(companion_enabled, companion_port)
            .await?;

        if spawn_background_tasks {
            state.spawn_background_tasks();
        }

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
        self.osc_client
            .apply_settings(settings.clone(), self.clone())
            .await?;
        Ok(settings)
    }

    pub async fn osc_status_snapshot(&self) -> OscStatusSnapshot {
        self.osc_client.status().await
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
        self.resolume_client
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
        self.resolume_client
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
