//! Per-slide stage-layout markers (#515) — `AppState` methods.
//!
//! The operator can mark a slide with a stage layout; when that slide becomes
//! current (`POST /stage/state`), the server switches the stage display to the
//! marked layout exactly like `POST /stage/layout` does (persist + broadcast
//! over /live/ws). Slides without a marker leave the layout untouched.

use super::AppState;
use presenter_core::{PresentationId, SlideId};
use std::collections::HashMap;

impl AppState {
    /// Assign (`Some(code)`) or clear (`None`) the stage-layout marker of a
    /// slide. The assignable set matches the operator layout picker
    /// (`set_stage_layout_code`): built-in layouts only, minus `camera-crew`.
    pub async fn assign_slide_stage_layout(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
        layout_code: Option<&str>,
    ) -> anyhow::Result<()> {
        let Some((_, _, presentation)) = self.presentation_detail(presentation_id).await? else {
            anyhow::bail!("presentation not found");
        };
        if !presentation.slides.iter().any(|slide| slide.id == slide_id) {
            anyhow::bail!("slide not found in presentation");
        }

        match layout_code {
            Some(code) => {
                // Same validation as POST /stage/layout — the two paths share
                // one operator-selectable source of truth and cannot drift.
                Self::validate_operator_selectable(code)?;
                self.repository
                    .set_slide_stage_layout(presentation_id, slide_id, code)
                    .await?;
                tracing::info!(
                    target: "presenter::stage::slide_layout",
                    presentation_id = %presentation_id,
                    slide_id = %slide_id,
                    layout_code = %code,
                    "slide stage-layout marker assigned"
                );
            }
            None => {
                self.repository.clear_slide_stage_layout(slide_id).await?;
                tracing::info!(
                    target: "presenter::stage::slide_layout",
                    presentation_id = %presentation_id,
                    slide_id = %slide_id,
                    "slide stage-layout marker cleared"
                );
            }
        }
        Ok(())
    }

    /// All markers of a presentation as `slide_id → layout_code` (operator UI
    /// indicators).
    pub async fn slide_stage_layouts(
        &self,
        presentation_id: PresentationId,
    ) -> anyhow::Result<HashMap<String, String>> {
        self.repository
            .list_slide_stage_layouts(presentation_id)
            .await
    }

    /// Trigger hook (#515): when the now-current slide carries a marker and
    /// the stage is on a different layout, switch — same path as
    /// `POST /stage/layout` (persist + `LiveEvent::StageLayout` + snapshot
    /// broadcast). A stale marker (layout removed in a later release) must
    /// NEVER fail the slide trigger: log and continue.
    pub(super) async fn apply_slide_stage_layout_marker(&self, slide_id: SlideId) {
        let code = match self.repository.get_slide_stage_layout(slide_id).await {
            Ok(Some(code)) => code,
            Ok(None) => return,
            Err(err) => {
                tracing::warn!(
                    target: "presenter::stage::slide_layout",
                    ?err,
                    slide_id = %slide_id,
                    "failed to look up slide stage-layout marker — leaving layout unchanged"
                );
                return;
            }
        };
        if self.stage_layout_code().await == code {
            return;
        }
        // publish_snapshots = false: update_stage_state broadcasts the fresh
        // resolution right after this call, so the full-context snapshot fan-
        // out inside the switch would reach every /live/ws client twice.
        match self.switch_stage_layout(&code, false).await {
            Ok(layout) => {
                tracing::info!(
                    target: "presenter::stage::slide_layout",
                    slide_id = %slide_id,
                    layout_code = %layout.code,
                    "stage layout switched by slide marker"
                );
            }
            Err(err) => {
                // Two failure modes share this arm: an unusable marker code
                // (validation failed — layout genuinely unchanged) and a
                // broadcast error AFTER the switch was committed (clients
                // already received StageLayout). Do not claim "unchanged".
                tracing::warn!(
                    target: "presenter::stage::slide_layout",
                    ?err,
                    slide_id = %slide_id,
                    layout_code = %code,
                    "slide stage-layout marker switch failed — invalid marker, or the switch committed but its broadcast errored"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::state::AppState;
    use presenter_core::{Presentation, SlideId, DEFAULT_STAGE_LAYOUT_CODE};

    async fn seeded_state() -> (AppState, Presentation) {
        let state = AppState::in_memory().await.unwrap();
        crate::state::seed_sample_library(&state).await.unwrap();
        let libraries = state.libraries().await.unwrap();
        let presentation = libraries[0].presentations[0].clone();
        (state, presentation)
    }

    #[tokio::test]
    async fn assign_then_list_roundtrips() {
        let (state, presentation) = seeded_state().await;
        let slide_id = presentation.slides[0].id;

        state
            .assign_slide_stage_layout(presentation.id, slide_id, Some("fulltext"))
            .await
            .unwrap();

        let map = state.slide_stage_layouts(presentation.id).await.unwrap();
        assert_eq!(
            map.get(&slide_id.to_string()).map(String::as_str),
            Some("fulltext")
        );
    }

    #[tokio::test]
    async fn assign_none_clears_marker() {
        let (state, presentation) = seeded_state().await;
        let slide_id = presentation.slides[0].id;

        state
            .assign_slide_stage_layout(presentation.id, slide_id, Some("timer"))
            .await
            .unwrap();
        state
            .assign_slide_stage_layout(presentation.id, slide_id, None)
            .await
            .unwrap();

        assert!(state
            .slide_stage_layouts(presentation.id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn assign_rejects_unknown_layout() {
        let (state, presentation) = seeded_state().await;
        let slide_id = presentation.slides[0].id;

        let err = state
            .assign_slide_stage_layout(presentation.id, slide_id, Some("no-such-layout"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown stage layout"));
    }

    #[tokio::test]
    async fn assign_rejects_camera_crew() {
        let (state, presentation) = seeded_state().await;
        let slide_id = presentation.slides[0].id;

        let err = state
            .assign_slide_stage_layout(presentation.id, slide_id, Some("camera-crew"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("camera-crew"));
    }

    #[tokio::test]
    async fn assign_rejects_unknown_slide() {
        let (state, presentation) = seeded_state().await;

        let err = state
            .assign_slide_stage_layout(presentation.id, SlideId::new(), Some("fulltext"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("slide not found"));
    }

    /// The core #515 behavior: triggering a slide that carries a marker
    /// switches the stage layout, exactly like `POST /stage/layout` would.
    #[tokio::test]
    async fn triggering_marked_slide_switches_stage_layout() {
        let (state, presentation) = seeded_state().await;
        let current = presentation.slides[0].id;
        let next = presentation.slides.get(1).map(|slide| slide.id);

        state
            .set_stage_layout_code(DEFAULT_STAGE_LAYOUT_CODE)
            .await
            .unwrap();
        state
            .assign_slide_stage_layout(presentation.id, current, Some("fulltext"))
            .await
            .unwrap();

        state
            .update_stage_state(presentation.id, current, next, None, None)
            .await
            .unwrap();

        assert_eq!(state.stage_layout_code().await, "fulltext");
    }

    /// A slide WITHOUT a marker leaves the operator-chosen layout untouched —
    /// markers switch, absence never reverts.
    #[tokio::test]
    async fn triggering_unmarked_slide_keeps_current_layout() {
        let (state, presentation) = seeded_state().await;
        let marked = presentation.slides[0].id;
        let unmarked = presentation.slides[1].id;

        state
            .set_stage_layout_code(DEFAULT_STAGE_LAYOUT_CODE)
            .await
            .unwrap();
        state
            .assign_slide_stage_layout(presentation.id, marked, Some("fulltext"))
            .await
            .unwrap();

        // Marked slide switches …
        state
            .update_stage_state(presentation.id, marked, Some(unmarked), None, None)
            .await
            .unwrap();
        assert_eq!(state.stage_layout_code().await, "fulltext");

        // … the following unmarked slide does NOT revert.
        state
            .update_stage_state(presentation.id, unmarked, None, None, None)
            .await
            .unwrap();
        assert_eq!(state.stage_layout_code().await, "fulltext");
    }

    /// Duplicating a marked slide copies the marker to the duplicate (#515
    /// review finding) — the copy behaves identically when triggered.
    #[tokio::test]
    async fn duplicating_marked_slide_copies_marker() {
        let (state, presentation) = seeded_state().await;
        let source = presentation.slides[0].id;
        state
            .assign_slide_stage_layout(presentation.id, source, Some("fulltext"))
            .await
            .unwrap();

        let slides = state
            .duplicate_slide(presentation.id, source)
            .await
            .unwrap();
        let duplicate = slides[1].id; // inserted right after the source
        assert_ne!(duplicate, source);

        let map = state.slide_stage_layouts(presentation.id).await.unwrap();
        assert_eq!(
            map.get(&duplicate.to_string()).map(String::as_str),
            Some("fulltext"),
            "duplicate must carry the source slide's marker"
        );
        // The source keeps its own marker too.
        assert_eq!(
            map.get(&source.to_string()).map(String::as_str),
            Some("fulltext")
        );
    }

    /// A stale marker (its layout no longer usable) must never fail the slide
    /// trigger — the trigger succeeds and the layout stays unchanged.
    #[tokio::test]
    async fn stale_marker_does_not_fail_slide_trigger() {
        let (state, presentation) = seeded_state().await;
        let current = presentation.slides[0].id;

        // Write an invalid marker directly through the repository, bypassing
        // the assignment validation (simulates a layout removed in a later
        // release while the DB row survived).
        state
            .repository()
            .set_slide_stage_layout(presentation.id, current, "removed-layout")
            .await
            .unwrap();
        state
            .set_stage_layout_code(DEFAULT_STAGE_LAYOUT_CODE)
            .await
            .unwrap();

        state
            .update_stage_state(presentation.id, current, None, None, None)
            .await
            .unwrap();

        assert_eq!(state.stage_layout_code().await, DEFAULT_STAGE_LAYOUT_CODE);
    }
}
