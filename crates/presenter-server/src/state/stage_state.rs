//! Stage-state mutation and slide-edit reconciliation for [`AppState`].
//!
//! Extracted from `state/mod.rs` (#486) to keep the central module under the
//! file-size cap. Behaviour is unchanged — these are the same `impl AppState`
//! methods, only relocated.

use super::stage::{
    build_stage_playlist_entries, stage_resolution_from_presentation, StageResolution,
};
use super::AppState;
use presenter_core::{PlaylistId, PresentationId, Slide, SlideId, StageState};
use std::time::Instant;
use uuid::Uuid;

impl AppState {
    pub async fn update_stage_state(
        &self,
        presentation_id: PresentationId,
        current_slide_id: SlideId,
        next_slide_id: Option<SlideId>,
        playlist_id: Option<PlaylistId>,
    ) -> anyhow::Result<()> {
        let correlation_id = Uuid::new_v4();
        let start = Instant::now();

        let validate_start = Instant::now();
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
        let t_validate_ms = validate_start.elapsed().as_secs_f64() * 1000.0;

        let stage_state = presenter_core::StageState::new(
            Some(presentation_id),
            Some(current_slide_id),
            next_slide_id,
            playlist_id,
        );
        let db_start = Instant::now();
        self.repository.upsert_stage_state(&stage_state).await?;
        let t_db_write_ms = db_start.elapsed().as_secs_f64() * 1000.0;

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

        let broadcast_start = Instant::now();
        self.broadcast_stage_resolution(resolution, Some(correlation_id))
            .await?;
        let t_broadcast_ms = broadcast_start.elapsed().as_secs_f64() * 1000.0;

        let t_total_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::info!(
            target: "presenter::stage::handler",
            correlation_id = %correlation_id,
            t_validate_ms,
            t_db_write_ms,
            t_broadcast_ms,
            t_total_ms,
            "stage handler timing"
        );

        Ok(())
    }

    pub async fn clear_stage(&self) -> anyhow::Result<()> {
        let cleared = StageState::cleared();
        self.repository.upsert_stage_state(&cleared).await?;
        self.broadcast_stage_resolution(StageResolution::cleared(), None)
            .await?;
        Ok(())
    }

    pub(super) fn reindex_slides(slides: &mut [Slide]) {
        for (index, slide) in slides.iter_mut().enumerate() {
            slide.order = index as u32;
        }
    }

    pub(super) async fn reconcile_stage_state_after_edit(
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
