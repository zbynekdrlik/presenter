use crate::{
    live::{LiveEvent, LiveHub},
    resolume::{BibleUpdate, ResolumeConnectionSnapshot, ResolumeRegistry, StageUpdate},
    stage_connections::{StageClientSnapshot, StageConnections, StageHeartbeatConfig},
};
use anyhow;
use chrono::{DateTime, Utc};
use presenter_bible::BibleImportSummary;
use presenter_core::{
    BibleBroadcast, BibleReference, BibleTranslation, Library, LibraryId, LibrarySummary, Playlist,
    PlaylistEntry, PlaylistId, Presentation, PresentationId, ResolumeHost, ResolumeHostDraft,
    ResolumeHostId, SearchResult, Slide, SlideContent, SlideGroup, SlideId, SlideText,
    StageDisplayLayout, StageDisplaySlide, StageDisplaySnapshot, StageState, TimerCommand,
    TimersOverview, TimersState,
};
use presenter_importer::bible::BibleIngestionService;
use presenter_persistence::{DatabaseSettings, Repository};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::RwLock;
use tracing::instrument;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    repository: Repository,
    live_hub: LiveHub,
    bible_broadcast: Arc<RwLock<Option<BibleBroadcast>>>,
    companion_token: Option<String>,
    resolume_registry: ResolumeRegistry,
    stage_connections: StageConnections,
    heartbeat_config: StageHeartbeatConfig,
    #[cfg(test)]
    bible_ingestion_override: Option<std::sync::Arc<dyn TestBibleIngestion + Send + Sync>>,
}

impl AppState {
    pub fn new(
        repository: Repository,
        companion_token: Option<String>,
        resolume_registry: ResolumeRegistry,
    ) -> Self {
        let stage_connections = StageConnections::new();
        let heartbeat_config = StageHeartbeatConfig::from_env();
        let state = Self {
            repository,
            live_hub: LiveHub::new(),
            bible_broadcast: Arc::new(RwLock::new(None)),
            companion_token,
            resolume_registry,
            stage_connections,
            heartbeat_config,
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

    #[instrument(skip_all)]
    pub async fn from_env() -> anyhow::Result<Self> {
        let db_url = env::var("PRESENTER_DB_URL")
            .unwrap_or_else(|_| "sqlite://presenter_dev.db".to_string());
        let repo = Repository::connect(&DatabaseSettings::new(&db_url)).await?;
        let companion_token = env::var("PRESENTER_COMPANION_TOKEN").ok();
        let registry = ResolumeRegistry::new();
        let state = Self::new(repo, companion_token, registry);
        state.ensure_seed_library().await?;
        state.sync_resolume_hosts().await?;
        Ok(state)
    }

    #[cfg(test)]
    #[instrument(skip_all)]
    pub async fn in_memory() -> anyhow::Result<Self> {
        let repo = Repository::connect_in_memory().await?;
        let registry = ResolumeRegistry::new();
        let state = Self::new(repo, None, registry);
        state.ensure_seed_library().await?;
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
            .await
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

    pub async fn list_resolume_hosts(&self) -> anyhow::Result<Vec<ResolumeHost>> {
        self.repository.list_resolume_hosts().await
    }

    pub async fn resolume_status_snapshot(
        &self,
    ) -> HashMap<ResolumeHostId, ResolumeConnectionSnapshot> {
        self.resolume_registry.snapshot().await
    }

    pub async fn resolume_status_for(&self, id: ResolumeHostId) -> ResolumeConnectionSnapshot {
        self.resolume_registry.snapshot_for(id).await
    }

    pub async fn create_resolume_host(
        &self,
        draft: ResolumeHostDraft,
    ) -> anyhow::Result<ResolumeHost> {
        let host = self.repository.create_resolume_host(&draft).await?;
        self.sync_resolume_hosts().await?;
        Ok(host)
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

    pub async fn delete_resolume_host(&self, id: ResolumeHostId) -> anyhow::Result<()> {
        self.repository.delete_resolume_host(id).await?;
        self.sync_resolume_hosts().await
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
        let detail = self
            .repository
            .fetch_presentation_detail(presentation_id)
            .await?;
        let Some((_, _, presentation)) = detail else {
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
        self.repository
            .fetch_presentation_detail(presentation_id)
            .await
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
        let (_, _, presentation) = self
            .presentation_detail(presentation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("presentation not found"))?;

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

        self.repository
            .update_slide_content(presentation_id, slide_id, &content)
            .await?;

        let updated_slide = Slide::new(existing_slide.order, content.clone()).with_id(slide_id);

        self.broadcast_stage_snapshots().await?;

        Ok(updated_slide)
    }

    pub async fn insert_blank_slide(
        &self,
        presentation_id: PresentationId,
        position: Option<u32>,
    ) -> anyhow::Result<Vec<Slide>> {
        let (_, _, presentation) = self
            .presentation_detail(presentation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("presentation not found"))?;
        let mut slides = presentation.slides;
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
        self.broadcast_stage_snapshots().await?;
        Ok(slides)
    }

    pub async fn duplicate_slide(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
    ) -> anyhow::Result<Vec<Slide>> {
        let (_, _, presentation) = self
            .presentation_detail(presentation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("presentation not found"))?;
        let mut slides = presentation.slides;
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
        self.broadcast_stage_snapshots().await?;
        Ok(slides)
    }

    pub async fn delete_slide(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
    ) -> anyhow::Result<Vec<Slide>> {
        let (_, _, presentation) = self
            .presentation_detail(presentation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("presentation not found"))?;
        let mut slides = presentation.slides;
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
        self.broadcast_stage_snapshots().await?;
        Ok(slides)
    }

    pub async fn reorder_slides(
        &self,
        presentation_id: PresentationId,
        order: Vec<SlideId>,
    ) -> anyhow::Result<Vec<Slide>> {
        let (_, _, presentation) = self
            .presentation_detail(presentation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("presentation not found"))?;
        let mut map = HashMap::new();
        for slide in presentation.slides {
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
        self.publish_stage_context(&context);
        Ok(())
    }

    async fn broadcast_stage_resolution(&self, resolution: StageResolution) -> anyhow::Result<()> {
        let now = Utc::now();
        let timers_state = self.load_or_init_timers(now).await?;
        let context = StageContext {
            generated_at: now,
            overview: timers_state.overview(now),
            resolution,
        };
        self.publish_stage_context(&context);
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
        let stage_update = StageUpdate {
            current_main: Some(current_main),
            current_translation: Some(current_translation),
        };
        self.resolume_registry.stage_update(stage_update).await;
        Ok(())
    }

    fn publish_stage_context(&self, context: &StageContext) {
        for layout in StageDisplayLayout::built_in() {
            let snapshot = build_stage_snapshot(layout, context);
            self.publish_stage_update(snapshot);
        }
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

        Ok(Some(StageContext {
            generated_at: now,
            overview,
            resolution,
        }))
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
        let Some((_, _library_name, presentation)) = detail else {
            return Ok(None);
        };
        let resolution = stage_resolution_from_presentation(
            &presentation,
            stage_state.current_slide_id,
            stage_state.next_slide_id,
        );
        Ok(Some(resolution))
    }

    async fn resolve_default_stage(&self) -> anyhow::Result<Option<StageResolution>> {
        let detail = self.repository.fetch_first_presentation_detail().await?;
        let Some((_, _presentation_name, presentation)) = detail else {
            return Ok(None);
        };
        let resolution = stage_resolution_from_presentation(&presentation, None, None);
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StageResolution {
    presentation_id: Option<PresentationId>,
    presentation_name: Option<String>,
    current_slide_id: Option<SlideId>,
    current: Option<StageDisplaySlide>,
    next_slide_id: Option<SlideId>,
    next: Option<StageDisplaySlide>,
}

impl StageResolution {
    fn cleared() -> Self {
        Self {
            presentation_id: None,
            presentation_name: None,
            current_slide_id: None,
            current: None,
            next_slide_id: None,
            next: None,
        }
    }
}

fn stage_resolution_from_presentation(
    presentation: &Presentation,
    current_slide_id: Option<SlideId>,
    next_slide_id: Option<SlideId>,
) -> StageResolution {
    let resolved = presentation.resolved_slides();

    if resolved.is_empty() {
        return StageResolution {
            presentation_id: Some(presentation.id),
            presentation_name: Some(presentation.name.clone()),
            current_slide_id: None,
            current: None,
            next_slide_id: None,
            next: None,
        };
    }

    let resolve_by_id = |id: SlideId| resolved.iter().find(|slide| slide.id == id);

    let current_resolved = current_slide_id.and_then(resolve_by_id);
    let fallback_current = current_resolved.or_else(|| resolved.first());

    let next_resolved = next_slide_id
        .and_then(resolve_by_id)
        .or_else(|| fallback_current.and_then(|current| find_next_resolved(&resolved, current)));

    StageResolution {
        presentation_id: Some(presentation.id),
        presentation_name: Some(presentation.name.clone()),
        current_slide_id: fallback_current.map(|slide| slide.id),
        current: fallback_current.map(StageDisplaySlide::from),
        next_slide_id: next_resolved.map(|slide| slide.id),
        next: next_resolved.map(StageDisplaySlide::from),
    }
}

fn find_next_resolved<'a>(
    slides: &'a [presenter_core::slide::ResolvedSlide],
    current: &presenter_core::slide::ResolvedSlide,
) -> Option<&'a presenter_core::slide::ResolvedSlide> {
    slides
        .iter()
        .filter(|slide| slide.order > current.order)
        .min_by_key(|slide| slide.order)
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
        context.resolution.current_slide_id,
        context.resolution.current.clone(),
        context.resolution.next_slide_id,
        context.resolution.next.clone(),
        context.overview.clone(),
    )
}

fn blank_slide_content() -> SlideContent {
    SlideContent::new(
        SlideText::new("").expect("empty main within limit"),
        SlideText::new("").expect("empty translation within limit"),
        SlideText::new("").expect("empty stage within limit"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::LiveEvent;
    use presenter_core::{bible::BibleIngestionBatch, BiblePassage, BibleTranslation, TimerState};

    #[tokio::test]
    async fn seeded_state_contains_library() {
        let state = AppState::in_memory().await.unwrap();
        let libraries = state.libraries().await.unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].name, "Sample Library");
    }

    #[tokio::test]
    async fn stage_updates_emit_live_event() {
        let state = AppState::in_memory().await.unwrap();
        let hub = state.live_hub();
        let mut rx = hub.subscribe();

        let libraries = state.libraries().await.unwrap();
        let presentation = &libraries[0].presentations[0];
        let current = presentation.slides[0].id;
        let next = presentation.slides.get(1).map(|slide| slide.id);

        state
            .update_stage_state(presentation.id, current, next)
            .await
            .unwrap();

        let mut seen_stage = false;
        for _ in 0..5 {
            match rx.recv().await.unwrap() {
                LiveEvent::Stage { snapshot } => {
                    seen_stage = true;
                    assert_eq!(snapshot.presentation_name.unwrap(), presentation.name);
                    break;
                }
                _ => continue,
            }
        }

        assert!(seen_stage, "expected stage event after update");
    }

    #[tokio::test]
    async fn clear_stage_emits_blank_snapshot() {
        let state = AppState::in_memory().await.unwrap();
        let hub = state.live_hub();
        let mut rx = hub.subscribe();

        state.clear_stage().await.unwrap();

        let stored = state
            .repository()
            .get_stage_state()
            .await
            .unwrap()
            .expect("stage state persisted");
        assert!(stored.presentation_id.is_none());
        assert!(stored.current_slide_id.is_none());

        let mut saw_stage = false;
        for _ in 0..5 {
            match rx.recv().await.unwrap() {
                LiveEvent::Stage { snapshot } => {
                    assert!(snapshot.presentation_id.is_none());
                    assert!(snapshot.current.is_none());
                    saw_stage = true;
                    break;
                }
                _ => continue,
            }
        }

        assert!(saw_stage, "expected stage event after clearing");

        let snapshot = state
            .stage_display_snapshot("worship-snv")
            .await
            .unwrap()
            .expect("snapshot available");
        assert!(snapshot.presentation_id.is_none());
        assert!(snapshot.current.is_none());
        assert!(snapshot.next.is_none());
    }

    #[tokio::test]
    async fn update_slide_content_updates_repository() {
        let state = AppState::in_memory().await.unwrap();
        let libraries = state.libraries().await.unwrap();
        let presentation = libraries[0].presentations[0].clone();
        let slide = presentation.slides[0].clone();

        let updated = state
            .update_slide_content(
                presentation.id,
                slide.id,
                "Tablet main".to_string(),
                "Tablet translation".to_string(),
                "Tablet stage".to_string(),
                Some("Tablet Group".to_string()),
            )
            .await
            .unwrap();

        assert_eq!(updated.id, slide.id);
        assert_eq!(updated.order, slide.order);
        assert_eq!(updated.content.main.value(), "Tablet main");
        assert_eq!(updated.content.translation.value(), "Tablet translation");
        assert_eq!(updated.content.stage.value(), "Tablet stage");
        assert_eq!(
            updated.content.group.as_ref().map(|group| group.name()),
            Some("Tablet Group")
        );

        let detail = state
            .presentation_detail(presentation.id)
            .await
            .unwrap()
            .expect("presentation detail");
        let stored = detail
            .2
            .slides
            .iter()
            .find(|candidate| candidate.id == slide.id)
            .expect("slide present");

        assert_eq!(stored.content.main.value(), "Tablet main");
        assert_eq!(stored.content.translation.value(), "Tablet translation");
        assert_eq!(stored.content.stage.value(), "Tablet stage");
    }

    #[tokio::test]
    async fn stage_snapshot_defaults_to_first_presentation() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .purge_presentation_content()
            .await
            .unwrap();

        let presentation = Presentation::new(
            "Primer",
            vec![Slide::new(
                0,
                SlideContent::new(
                    SlideText::new("Prvá veta").unwrap(),
                    SlideText::new("First sentence").unwrap(),
                    SlideText::new("Stage text").unwrap(),
                    None,
                ),
            )
            .with_id(SlideId::new())],
        )
        .unwrap();
        let library = Library::new("Fallback", vec![presentation.clone()])
            .unwrap()
            .with_id(LibraryId::new());
        state.repository().upsert_library(&library).await.unwrap();

        let snapshot = state
            .stage_display_snapshot("worship-snv")
            .await
            .unwrap()
            .expect("snapshot");
        assert_eq!(snapshot.presentation_name.unwrap(), "Primer");
        assert_eq!(snapshot.current.unwrap().main, "Prvá veta");
    }

    #[tokio::test]
    async fn stage_resolution_propagates_effective_group() {
        let state = AppState::in_memory().await.unwrap();
        let libraries = state.libraries().await.unwrap();
        let presentation = libraries[0].presentations[0].clone();
        let first_group = presentation
            .slides
            .first()
            .and_then(|slide| slide.content.group.as_ref())
            .map(|group| group.name().to_string())
            .expect("seed presentation should include group");
        let second_slide = presentation
            .slides
            .get(1)
            .map(|slide| slide.id)
            .expect("seed presentation should include multiple slides");

        let resolved = presentation.resolved_slides();
        assert_eq!(resolved.len(), 2);
        let second_resolved_group = resolved
            .get(1)
            .and_then(|slide| slide.effective_group.as_ref())
            .map(|group| group.name().to_string());
        assert_eq!(second_resolved_group, Some(first_group.clone()));

        let resolution =
            stage_resolution_from_presentation(&presentation, Some(second_slide), None);

        let current_group = resolution
            .current
            .as_ref()
            .and_then(|slide| slide.group.as_ref())
            .cloned();
        assert_eq!(current_group, Some(first_group.clone()));

        let next_group = resolution
            .next
            .as_ref()
            .and_then(|slide| slide.group.as_ref())
            .cloned();
        assert_eq!(next_group, None);
    }

    #[tokio::test]
    async fn timer_commands_emit_live_event() {
        let state = AppState::in_memory().await.unwrap();
        let hub = state.live_hub();
        let mut rx = hub.subscribe();

        let target = Utc::now() + chrono::Duration::minutes(15);
        state
            .execute_timer_command(TimerCommand::SetCountdownTarget { target })
            .await
            .unwrap();
        state
            .execute_timer_command(TimerCommand::StartCountdown)
            .await
            .unwrap();

        let mut seen_running = false;
        for _ in 0..8 {
            match rx.recv().await.unwrap() {
                LiveEvent::Timers { overview }
                    if overview.countdown_to_start.state == TimerState::Running =>
                {
                    seen_running = true;
                    break;
                }
                _ => continue,
            }
        }

        assert!(seen_running, "expected running timers event after command");
    }

    #[tokio::test]
    async fn trigger_bible_passage_publishes_event_and_state() {
        let state = AppState::in_memory().await.unwrap();
        let translation = BibleTranslation::new("test", "Test", "en");
        let reference = BibleReference::new("John", 3, 16, 16).unwrap();
        let passage = BiblePassage::new(
            reference.clone(),
            translation.clone(),
            "For God so loved".to_string(),
        );
        let batch = BibleIngestionBatch::new(translation, vec![passage]).unwrap();
        state
            .repository()
            .replace_bible_translation_passages(&batch)
            .await
            .unwrap();

        let mut rx = state.live_hub().subscribe();
        let broadcast = state
            .trigger_bible_passage("test", &reference)
            .await
            .unwrap();
        assert_eq!(broadcast.passage.reference, reference);
        assert!(state.active_bible_broadcast().await.is_some());

        match rx.recv().await.unwrap() {
            LiveEvent::Bible { broadcast: evt } => {
                assert_eq!(evt.passage.translation.code, "test");
            }
            other => panic!("unexpected live event: {other:?}"),
        }

        state.clear_bible_broadcast().await;
        match rx.recv().await.unwrap() {
            LiveEvent::BibleCleared => {}
            other => panic!("expected bible cleared event, got {other:?}"),
        }

        assert!(state.active_bible_broadcast().await.is_none());
    }
}
