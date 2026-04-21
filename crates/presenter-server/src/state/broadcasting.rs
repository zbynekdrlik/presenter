use std::collections::HashMap;

use chrono::Utc;
use presenter_core::{
    StageDisplayLayout, StageDisplaySnapshot, StageState, DEFAULT_STAGE_LAYOUT_CODE,
};

use super::stage::{
    build_stage_snapshot, sanitize_song_title, stage_resolution_from_presentation, StageContext,
    StageResolution,
};
use super::AppState;
use crate::live::LiveEvent;
use crate::resolume::StageUpdate;

impl AppState {
    pub(super) fn publish_stage_update(&self, snapshot: StageDisplaySnapshot) {
        self.live_hub.publish(LiveEvent::Stage { snapshot });
    }

    pub(super) async fn sample_resolume_latency(&self) -> Option<f64> {
        let snapshot = self.resolume_registry.snapshot().await;
        snapshot
            .values()
            .filter_map(|status| status.last_latency_ms)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    }

    pub(super) async fn broadcast_stage_snapshots(&self) -> anyhow::Result<()> {
        let Some(context) = self.build_stage_context().await? else {
            return Ok(());
        };
        self.publish_stage_context(&context).await?;
        Ok(())
    }

    pub(super) async fn broadcast_stage_resolution(
        &self,
        resolution: StageResolution,
    ) -> anyhow::Result<()> {
        let now = Utc::now();
        let timers_state = self.load_or_init_timers(now).await?;
        let latency_ms = self.sample_resolume_latency().await;
        let context = StageContext {
            generated_at: now,
            overview: timers_state.overview(now),
            resolution,
            latency_ms,
        };
        self.publish_stage_context(&context).await?;
        let current_main = context
            .resolution
            .current
            .as_ref()
            .map(|slide| slide.main.clone())
            .unwrap_or_default();
        let current_translation = context
            .resolution
            .current
            .as_ref()
            .map(|slide| slide.translation.clone())
            .unwrap_or_default();
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
            .or_else(|| layouts.remove(DEFAULT_STAGE_LAYOUT_CODE))
        else {
            return Ok(());
        };

        let context = self.enrich_stage_context(context).await;
        let snapshot = build_stage_snapshot(layout, &context);
        self.publish_stage_update(snapshot);
        Ok(())
    }

    /// Populates AbleSet song names on a stage context if not already set.
    pub(super) async fn enrich_stage_context(&self, context: &StageContext) -> StageContext {
        let mut context = context.clone();
        if context.resolution.override_song_name.is_none() {
            context.resolution.override_song_name = self.resolve_current_song_name().await;
        }
        if context.resolution.next_song_name.is_none() {
            context.resolution.next_song_name =
                self.resolve_next_song_name(&context.resolution).await;
        }
        // Resolve group colors
        if let Some(ref mut slide) = context.resolution.current {
            if let Some(ref name) = slide.group {
                if slide.group_color.is_none() {
                    slide.group_color = self.resolve_group_color(name).await;
                }
            }
        }
        if let Some(ref mut slide) = context.resolution.next {
            if let Some(ref name) = slide.group {
                if slide.group_color.is_none() {
                    slide.group_color = self.resolve_group_color(name).await;
                }
            }
        }
        context
    }

    async fn resolve_current_song_name(&self) -> Option<String> {
        let snapshot = self.ableset_bridge.song_snapshot().await?;
        Some(snapshot.name.clone())
    }

    pub(super) async fn resolve_next_song_name(
        &self,
        resolution: &StageResolution,
    ) -> Option<String> {
        // Try AbleSet first
        if let Some(name) = self.ableset_bridge.next_song_name().await {
            return Some(name);
        }

        // Fall back to playlist: find next presentation after current
        let current_id = resolution.presentation_id?;
        let entries = resolution.playlist_entries.as_ref()?;
        let current_idx = entries
            .iter()
            .position(|e| e.presentation_id == Some(current_id))?;
        let next_entry = entries.get(current_idx + 1)?;
        if next_entry.entry_type != "presentation" {
            return None;
        }
        // Look up the raw presentation name (with number prefix)
        let next_id = next_entry.presentation_id?;
        if let Ok(Some((_, _, presentation))) =
            self.repository.fetch_presentation_detail(next_id).await
        {
            Some(presentation.name.clone())
        } else if !next_entry.name.is_empty() {
            Some(next_entry.name.clone())
        } else {
            None
        }
    }

    pub(super) async fn build_stage_context(&self) -> anyhow::Result<Option<StageContext>> {
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

        let latency_ms = self.sample_resolume_latency().await;
        Ok(Some(StageContext {
            generated_at: now,
            overview,
            resolution,
            latency_ms,
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
        let Some((_, library_name, presentation)) = detail else {
            return Ok(None);
        };
        let mut resolution = stage_resolution_from_presentation(
            &presentation,
            Some(library_name),
            stage_state.current_slide_id,
            stage_state.next_slide_id,
        );
        if let Some(playlist_id) = stage_state.playlist_id {
            if let Some(playlist) = self.repository.fetch_playlist_by_id(playlist_id).await? {
                let name_lookup = self
                    .repository
                    .fetch_presentation_names_for_playlist(&playlist)
                    .await?;
                resolution.playlist_id = Some(playlist_id);
                resolution.playlist_name = Some(playlist.name.clone());
                resolution.playlist_entries = Some(super::stage::build_stage_playlist_entries(
                    &playlist,
                    resolution.presentation_id,
                    &name_lookup,
                ));
            }
        }
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
