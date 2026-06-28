//! API-driven stage state, presentation cache, and group-color resolution for
//! [`AppState`].
//!
//! Extracted from `state/mod.rs` (#486) to keep the central module under the
//! file-size cap. Behaviour is unchanged — these are the same `impl AppState`
//! methods, only relocated.

use super::{ApiStageState, AppState};
use crate::live::LiveEvent;
use chrono::Utc;
use presenter_core::{
    Presentation, PresentationId, StageDisplayLayout, StageDisplaySlide, StageDisplaySnapshot,
    TimersOverview, API_STAGE_LAYOUT_CODE,
};
use std::collections::HashMap;
use std::sync::Arc;

impl AppState {
    // Presentation cache methods
    pub(super) async fn presentation_from_cache(
        &self,
        presentation_id: PresentationId,
    ) -> anyhow::Result<Arc<Presentation>> {
        if let Some(cached) = {
            let guard = self.caches.presentation.read().await;
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
        let mut guard = self.caches.presentation.write().await;
        guard.insert(presentation_id, arc.clone());
        Ok(arc)
    }

    pub(crate) async fn get_all_group_colors(&self) -> HashMap<String, String> {
        self.caches.group_color.read().await.clone()
    }

    pub(crate) async fn resolve_group_color(&self, name: &str) -> Option<String> {
        {
            let cache = self.caches.group_color.read().await;
            if let Some(color) = cache.get(name) {
                return Some(color.clone());
            }
        }
        match self.repository.resolve_group_color(name).await {
            Ok(color) => {
                let mut cache = self.caches.group_color.write().await;
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
        let layout = StageDisplayLayout::api();

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
            Vec::new(),     // upcoming_groups (api layout has no upcoming context)
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

    pub(super) async fn cache_presentation_ref(&self, presentation: &Presentation) {
        let mut guard = self.caches.presentation.write().await;
        guard.insert(presentation.id, Arc::new(presentation.clone()));
    }

    pub(super) async fn cache_presentation_value(&self, presentation: Presentation) {
        let mut guard = self.caches.presentation.write().await;
        guard.insert(presentation.id, Arc::new(presentation));
    }
}
