//! API-driven stage state, presentation cache, and group-color resolution for
//! [`AppState`].
//!
//! Extracted from `state/mod.rs` (#486) to keep the central module under the
//! file-size cap. #799 adds the optional translation lines, the `api-ambient`
//! layout (every API-layout check goes through `is_api_stage_layout`) and the
//! text mode stamped onto every api snapshot.

use super::AppState;
use crate::live::LiveEvent;
use chrono::Utc;
use presenter_core::{
    Presentation, StageDisplayLayout, StageDisplaySlide, StageDisplaySnapshot, TimersOverview,
};
use std::collections::HashMap;
use std::sync::Arc;

/// External API-driven stage state (`PUT /api/stage`). All fields default to
/// empty strings when missing. The translation lines (#799) are optional, so
/// older clients that never send them keep working unchanged.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiStageState {
    #[serde(default)]
    pub(crate) current_text: String,
    #[serde(default)]
    pub(crate) next_text: String,
    /// #799: translation of `current_text` (e.g. the Slovak line songplayer
    /// sends alongside the original). Empty = no translation.
    #[serde(default)]
    pub(crate) current_translation: String,
    /// #799: translation of `next_text`.
    #[serde(default)]
    pub(crate) next_translation: String,
    #[serde(default)]
    pub(crate) current_group: String,
    #[serde(default)]
    pub(crate) next_group: String,
    #[serde(default)]
    pub(crate) current_song: String,
    #[serde(default)]
    pub(crate) next_song: String,
}

impl AppState {
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
        // Issue #281: only publish a Stage event when the operator's current
        // layout is an API layout (`api` / `api-ambient`, #799). Otherwise the
        // api state is stored but does not affect the live preview, mirroring
        // the inverse gate in `broadcasting.rs::publish_stage_context`.
        self.publish_api_snapshot(snapshot).await;
        Ok(())
    }

    /// Re-publish the stored api snapshot when an API layout is selected —
    /// after a switch TO an API layout (#281) or a text-mode change (#799),
    /// so displays reflect it without waiting for the next `PUT /api/stage`.
    ///
    /// The `api_stage` READ guard is held across build + publish: a concurrent
    /// `update_api_stage` must take the WRITE lock before publishing its new
    /// text, so it can never publish first and then be overwritten by this
    /// (older) snapshot. Lock order: `api_stage` (read) → group-color cache /
    /// timers → `stage_layout` (read); no path acquires `api_stage` while
    /// holding either of the others, so this cannot deadlock.
    pub(super) async fn republish_api_snapshot(&self) {
        let state = self.api_stage.read().await;
        let snapshot = self.build_api_stage_snapshot(&state).await;
        self.publish_api_snapshot(snapshot).await;
        drop(state);
    }

    /// #793: check + publish under the layout READ lock (no await in between)
    /// so an api snapshot can never land after a switch away from the API
    /// layouts (displays adopt a snapshot's layout). The layout + text mode
    /// are stamped HERE, under the lock (#799), so a switch `api` <->
    /// `api-ambient` between build and publish can never publish the other
    /// layout's code.
    async fn publish_api_snapshot(&self, mut snapshot: StageDisplaySnapshot) {
        let layout = self.stage_layout.read().await;
        if self.stamp_api_snapshot(&mut snapshot, &layout) {
            self.live_hub.publish(LiveEvent::Stage { snapshot });
        }
    }

    /// Stamp the current text mode onto `snapshot` and, when `code` is an API
    /// layout, that layout. Returns whether `code` is an API layout. Sync (the
    /// text mode is an atomic), so it is safe under the stage-layout lock.
    fn stamp_api_snapshot(&self, snapshot: &mut StageDisplaySnapshot, code: &str) -> bool {
        snapshot.text_mode = Some(self.stage_text_mode());
        match StageDisplayLayout::api_layout_for(code) {
            Some(layout) => {
                snapshot.layout = layout;
                true
            }
            None => false,
        }
    }

    /// The api snapshot as served for layout `code` (`GET /stage/snapshot`).
    /// Callers pass an API layout code; any other code keeps the `api` layout.
    pub(crate) async fn api_stage_snapshot_for(&self, code: &str) -> StageDisplaySnapshot {
        let state = self.api_stage.read().await.clone();
        let mut snapshot = self.build_api_stage_snapshot(&state).await;
        self.stamp_api_snapshot(&mut snapshot, code);
        snapshot
    }

    async fn build_api_stage_snapshot(&self, state: &ApiStageState) -> StageDisplaySnapshot {
        // `publish_api_snapshot` / `api_stage_snapshot_for` stamp the actual
        // selected API layout + text mode; `api` is only the build default.
        let layout = StageDisplayLayout::api();

        let current = self
            .build_api_slide(
                &state.current_text,
                &state.current_translation,
                &state.current_group,
            )
            .await;
        let next = self
            .build_api_slide(&state.next_text, &state.next_translation, &state.next_group)
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

    async fn build_api_slide(
        &self,
        text: &str,
        translation: &str,
        group_name: &str,
    ) -> Option<StageDisplaySlide> {
        if text.is_empty() && translation.is_empty() && group_name.is_empty() {
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
            translation: translation.to_string(),
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
