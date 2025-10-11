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
use presenter_core::bible::{
    BibleBookChapterSummary, BiblePreferences, BiblePreferencesDraft, BibleReference,
    BibleTranslation,
};
use presenter_core::playlist::PlaylistEntryKind;
use presenter_core::slide::{BibleSlideMetadata, BibleSlideVerseRef, SlideMetadata};
use presenter_core::{
    BibleBroadcast, Library, LibraryId, LibrarySummary, OscSettings, OscSettingsDraft, Playlist,
    PlaylistEntry, PlaylistEntryId, PlaylistId, Presentation, PresentationId, PresentationSummary,
    SearchResult, Slide, SlideContent, SlideGroup, SlideId, SlideText, StageDisplayLayout,
};
use presenter_importer::bible::BibleIngestionService;
use presenter_persistence::{DatabaseSettings, Repository};
use serde::Serialize;
use std::{
    collections::HashMap,
    env,
    sync::{atomic::AtomicBool, atomic::AtomicU16, atomic::Ordering, Arc},
};
use tokio::sync::RwLock;
use tokio::time::{interval, Duration as TokioDuration, MissedTickBehavior};
use tracing::{instrument, warn};
use uuid::Uuid;

use super::{AbleSetLibraryCache, CompanionServerManager};
fn parse_bool_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

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
    pub(super) resolume_registry: ResolumeRegistry,
    pub(super) android_stage_registry: AndroidStageRegistry,
    pub(super) stage_connections: StageConnections,
    pub(super) heartbeat_config: StageHeartbeatConfig,
    pub(super) presentation_cache: Arc<RwLock<HashMap<PresentationId, Arc<Presentation>>>>,
    pub(super) stage_layout: Arc<RwLock<String>>,
    pub(super) osc_bridge: OscBridge,
    pub(super) ableset_bridge: AbleSetBridge,
    pub(super) ableset_cache: Arc<RwLock<AbleSetLibraryCache>>,
    #[cfg(test)]
    pub(super) bible_ingestion_override:
        Option<std::sync::Arc<dyn TestBibleIngestion + Send + Sync>>,
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
        // Default new libraries to the Worship category unless a specific flow
        // (e.g., Bible) chooses otherwise.
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

    // Bible preferences and summaries
    pub async fn bible_preferences(&self) -> anyhow::Result<BiblePreferences> {
        let main = self
            .repository
            .get_app_setting("bible.main_translation")
            .await?;
        let secondary = self
            .repository
            .get_app_setting("bible.secondary_translation")
            .await?;
        let char_str = self
            .repository
            .get_app_setting("bible.character_limit")
            .await?;
        let mut prefs = BiblePreferences::default();
        if let Some(m) = main {
            if !m.trim().is_empty() {
                prefs = prefs.with_main_translation(Some(m));
            }
        }
        if let Some(s) = secondary {
            if !s.trim().is_empty() {
                prefs = prefs.with_secondary_translation(Some(s));
            }
        }
        if let Some(c) = char_str {
            if let Ok(v) = c.parse::<u32>() {
                prefs = prefs.with_character_limit(v);
            }
        }
        Ok(prefs)
    }

    pub async fn update_bible_preferences(
        &self,
        draft: BiblePreferencesDraft,
    ) -> anyhow::Result<BiblePreferences> {
        let current = self.bible_preferences().await?;
        let next = draft.apply(current);
        if let Some(ref m) = next.main_translation {
            self.repository
                .set_app_setting("bible.main_translation", m)
                .await?;
        } else {
            self.repository
                .set_app_setting("bible.main_translation", "")
                .await?;
        }
        if let Some(ref s) = next.secondary_translation {
            self.repository
                .set_app_setting("bible.secondary_translation", s)
                .await?;
        } else {
            self.repository
                .set_app_setting("bible.secondary_translation", "")
                .await?;
        }
        self.repository
            .set_app_setting("bible.character_limit", &next.character_limit.to_string())
            .await?;
        Ok(next)
    }

    pub async fn bible_book_chapter_summaries(
        &self,
        translation_code: &str,
    ) -> anyhow::Result<Vec<BibleBookChapterSummary>> {
        self.repository
            .bible_book_chapter_summaries(translation_code)
            .await
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

    pub fn feature_flags(&self) -> FeatureFlags {
        FeatureFlags {
            companion_enabled: self.companion_enabled(),
            companion_port: self.companion_port(),
        }
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

    // Bible presentations API used by bible router
    pub async fn list_bible_presentations(&self) -> anyhow::Result<Vec<PresentationSummary>> {
        let libraries = self.repository.fetch_libraries().await?;
        if let Some(bible_lib) = libraries
            .into_iter()
            .find(|l| l.name.eq_ignore_ascii_case("Bible"))
        {
            let summaries = bible_lib
                .presentations
                .into_iter()
                .map(|p| PresentationSummary::new(p.id, p.name))
                .collect();
            Ok(summaries)
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn bible_presentation_detail(
        &self,
        presentation_id: PresentationId,
    ) -> anyhow::Result<Option<Presentation>> {
        Ok(self
            .presentation_detail(presentation_id)
            .await?
            .map(|(_, _, p)| p))
    }

    pub async fn create_bible_presentation(&self, name: &str) -> anyhow::Result<Presentation> {
        // Ensure a Bible library exists
        let libraries = self.repository.fetch_libraries().await?;
        let library = if let Some(existing) = libraries
            .into_iter()
            .find(|l| l.name.eq_ignore_ascii_case("Bible"))
        {
            existing
        } else {
            self.repository.create_library("Bible").await?
        };

        let (_lib_id, _lib_name, presentation) = self
            .repository
            .create_presentation(library.id, name)
            .await?;
        // Cache and return
        self.cache_presentation_ref(&presentation).await;
        Ok(presentation)
    }

    pub async fn rename_bible_presentation(
        &self,
        presentation_id: PresentationId,
        name: &str,
    ) -> anyhow::Result<()> {
        self.repository
            .rename_presentation(presentation_id, name)
            .await
    }

    pub async fn append_bible_presentation_slides(
        &self,
        presentation_id: PresentationId,
        mut new_slides: Vec<Slide>,
    ) -> anyhow::Result<Presentation> {
        let arc = self.presentation_from_cache(presentation_id).await?;
        let mut presentation = arc.as_ref().clone();
        if presentation.slides.len() == 1 {
            let s = &presentation.slides[0];
            let is_blank = s.content.main.is_empty()
                && s.content.translation.is_empty()
                && s.content.stage.is_empty()
                && s.metadata.is_none();
            if is_blank {
                presentation.slides.clear();
            }
        }
        presentation.slides.extend(new_slides.drain(..));
        Self::reindex_slides(&mut presentation.slides);
        self.repository
            .replace_presentation_slides(presentation_id, &presentation.slides)
            .await?;
        self.cache_presentation_value(presentation.clone()).await;
        self.broadcast_stage_snapshots().await?;
        Ok(presentation)
    }

    pub async fn generate_bible_slides(
        &self,
        main_translation_code: &str,
        secondary_translation_code: Option<&str>,
        book: &str,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
        character_limit: u32,
    ) -> anyhow::Result<(BibleTranslation, Option<BibleTranslation>, Vec<Slide>)> {
        let translations = self.list_bible_translations().await?;
        let main_translation = translations
            .iter()
            .find(|t| t.code.eq_ignore_ascii_case(main_translation_code))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown main translation"))?;

        let secondary_translation = if let Some(code) = secondary_translation_code {
            translations
                .into_iter()
                .find(|t| t.code.eq_ignore_ascii_case(code))
        } else {
            None
        };

        let main_passages = self
            .repository
            .bible_passage_range(
                &main_translation.code,
                book,
                None,
                chapter,
                verse_start,
                verse_end,
            )
            .await?;

        let canonical_book_code = main_passages
            .first()
            .and_then(|p| p.reference.book_code.clone());

        let secondary_lookup: HashMap<u16, presenter_core::BiblePassage> =
            if let Some(ref tr) = secondary_translation {
                let passages = self
                    .repository
                    .bible_passage_range(
                        &tr.code,
                        book,
                        canonical_book_code.as_deref(),
                        chapter,
                        verse_start,
                        verse_end,
                    )
                    .await?;
                passages
                    .into_iter()
                    .map(|p| (p.reference.verse_start, p))
                    .collect()
            } else {
                HashMap::new()
            };

        let slides = compose_bible_slides(
            &main_translation,
            secondary_translation.as_ref(),
            &main_passages,
            &secondary_lookup,
            character_limit,
        )?;

        Ok((main_translation, secondary_translation, slides))
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

// Compose Bible slides from a passage sequence, batching contiguous verses
// up to a character limit.
pub(crate) fn compose_bible_slides(
    main_translation: &BibleTranslation,
    secondary_translation: Option<&BibleTranslation>,
    main_passages: &[presenter_core::BiblePassage],
    secondary_lookup: &HashMap<u16, presenter_core::BiblePassage>,
    character_limit: u32,
) -> anyhow::Result<Vec<Slide>> {
    let mut slides: Vec<Slide> = Vec::new();
    if main_passages.is_empty() {
        return Ok(slides);
    }

    let book = main_passages[0].reference.book.clone();
    let book_code = main_passages[0].reference.book_code.clone();
    let book_number = main_passages[0].reference.book_number;
    let chapter = main_passages[0].reference.chapter;

    let mut current_main = String::new();
    let mut current_tr = String::new();
    let mut verses_meta: Vec<(u16, u16)> = Vec::new();

    let push_slide = |slides: &mut Vec<Slide>,
                      main: String,
                      tr: String,
                      verses: &[(u16, u16)]|
     -> anyhow::Result<()> {
        if main.trim().is_empty() {
            return Ok(());
        }
        let content = SlideContent::new(
            SlideText::new(&main)?,
            SlideText::new(&tr)?,
            SlideText::new(&main)?,
            None,
        );
        let metadata = SlideMetadata::new().with_bible(BibleSlideMetadata {
            translation_code: main_translation.code.clone(),
            secondary_translation_code: secondary_translation.map(|t| t.code.clone()),
            book: book.clone(),
            book_code: book_code.clone(),
            book_number,
            chapter,
            verses: verses
                .iter()
                .map(|(s, e)| BibleSlideVerseRef::new(*s, *e))
                .collect(),
            main_reference_label: None,
            translation_reference_label: None,
        });
        slides.push(Slide::new(slides.len() as u32, content).with_metadata(Some(metadata)));
        Ok(())
    };

    for p in main_passages {
        let label = format!("{}. ", p.reference.verse_start);
        let line = format!("{}{}", label, p.text);
        let prospective_len =
            current_main.len() + if current_main.is_empty() { 0 } else { 1 } + line.len();
        if prospective_len > character_limit as usize && !current_main.is_empty() {
            // flush
            push_slide(
                &mut slides,
                current_main.clone(),
                current_tr.clone(),
                &verses_meta,
            )?;
            current_main.clear();
            current_tr.clear();
            verses_meta.clear();
        }
        if !current_main.is_empty() {
            current_main.push('\n');
        }
        current_main.push_str(&line);
        // translation (secondary)
        if let Some(sec) = secondary_lookup.get(&p.reference.verse_start) {
            let tr_label = format!("{}. ", sec.reference.verse_start);
            let tr_line = format!("{}{}", tr_label, sec.text);
            if !current_tr.is_empty() {
                current_tr.push('\n');
            }
            current_tr.push_str(&tr_line);
        }
        verses_meta.push((p.reference.verse_start, p.reference.verse_end));
    }

    // final flush
    if !current_main.is_empty() {
        push_slide(&mut slides, current_main, current_tr, &verses_meta)?;
    }

    Ok(slides)
}
