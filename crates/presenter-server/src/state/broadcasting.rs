use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use presenter_core::{
    StageDisplayLayout, StageDisplaySnapshot, StageState, DEFAULT_STAGE_LAYOUT_CODE,
};
use uuid::Uuid;

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
        correlation_id: Option<Uuid>,
    ) -> anyhow::Result<()> {
        let correlation_id = correlation_id.unwrap_or_else(Uuid::new_v4);
        let start = Instant::now();

        let now = Utc::now();
        let timers_state = self.load_or_init_timers(now).await?;
        let t_load_timers_ms = elapsed_ms(start);

        let latency_ms = self.sample_resolume_latency().await;
        let context = StageContext {
            generated_at: now,
            overview: timers_state.overview(now),
            resolution,
            latency_ms,
        };
        let t_build_ctx_ms = elapsed_ms(start) - t_load_timers_ms;

        let publish_start = Instant::now();
        self.publish_stage_context(&context).await?;
        let t_live_publish_ms = elapsed_ms(publish_start);

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

        let enqueue_start = Instant::now();
        let stage_update = StageUpdate {
            current_main: Some(current_main),
            current_translation: Some(current_translation),
            song_name: Some(song_name),
            band_name: Some(band_name),
            enqueued_at: Some(Instant::now()),
            correlation_id: Some(correlation_id),
        };
        self.resolume_registry.stage_update(stage_update).await;
        let t_resolume_enqueue_ms = elapsed_ms(enqueue_start);

        let t_total_ms = elapsed_ms(start);
        tracing::info!(
            target: "presenter::stage::timing",
            correlation_id = %correlation_id,
            t_load_timers_ms,
            t_build_ctx_ms,
            t_live_publish_ms,
            t_resolume_enqueue_ms,
            t_total_ms,
            "stage click timing"
        );

        Ok(())
    }

    async fn publish_stage_context(&self, context: &StageContext) -> anyhow::Result<()> {
        let code = self.stage_layout_code().await;
        let context = self.enrich_stage_context(context).await;

        // Always publish camera-crew snapshot — its clients are pinned to /ui/camera
        // and must not be flipped by operator-side layout changes (including "api").
        if code != "camera-crew" {
            if let Some(camera_layout) = StageDisplayLayout::built_in()
                .into_iter()
                .find(|l| l.code == "camera-crew")
            {
                let camera_snapshot = build_stage_snapshot(camera_layout, &context);
                self.publish_stage_update(camera_snapshot);
            }
        }

        // The "api" layout is driven by PUT /api/stage, not by internal state.
        // Skip normal broadcasting for the operator-selected snapshot to avoid
        // overwriting API-pushed data.
        if code == "api" {
            return Ok(());
        }

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
        // Look up the presentation name and strip the number prefix so the
        // stage display / Companion variable matches the sanitized current
        // song_name (see #312, build_stage_snapshot fallback).
        let next_id = next_entry.presentation_id?;
        if let Ok(Some((_, _, presentation))) =
            self.repository.fetch_presentation_detail(next_id).await
        {
            Some(sanitize_song_title(&presentation.name))
        } else if !next_entry.name.is_empty() {
            Some(sanitize_song_title(&next_entry.name))
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

    /// Build a `StageContext` with a *cleared* (empty) resolution — no
    /// presentation, no slides — but a live timers overview. Used to serve a
    /// valid 200 empty snapshot for the stage display when the database has no
    /// presentations / no default stage (issue #383), instead of a 404 that the
    /// browser network layer logs as a console error. The timers + layout still
    /// render; the slide area is simply blank.
    pub(super) async fn empty_stage_context(&self) -> anyhow::Result<StageContext> {
        let now = Utc::now();
        let timers_state = self.load_or_init_timers(now).await?;
        let overview = timers_state.overview(now);
        let latency_ms = self.sample_resolume_latency().await;
        Ok(StageContext {
            generated_at: now,
            overview,
            resolution: StageResolution::cleared(),
            latency_ms,
        })
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
                    None,
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

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
