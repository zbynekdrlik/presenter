//! AppState slide-edit operations: update content, insert blank, duplicate,
//! delete, and reorder slides. Each persists, reconciles stage state, updates
//! the presentation cache, and publishes a `BibleSlidesChanged` live event.

use presenter_core::slide::SlideMetadata;
use presenter_core::{PresentationId, Slide, SlideContent, SlideId, SlideText};
use std::collections::HashMap;

use super::super::stage::blank_slide_content;
use super::super::AppState;
use crate::live::LiveEvent;

/// Error surface for `paste_slides` so the router can distinguish a stale
/// clipboard (unknown ids → 422) from an internal failure (→ 500). (#554)
#[derive(Debug)]
pub enum PasteSlidesError {
    /// One or more requested slide ids are not in the presentation (stale
    /// clipboard) — the paste must fail loudly, never half-apply.
    UnknownSlides,
    /// Any other failure (cache read, persistence, stage reconcile).
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for PasteSlidesError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err)
    }
}

impl std::fmt::Display for PasteSlidesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSlides => write!(f, "one or more slides no longer exist"),
            Self::Internal(err) => write!(f, "{err}"),
        }
    }
}

impl AppState {
    /// Paste-of-COPY (#554): clone the named slides' full content and insert the
    /// clones as a contiguous block at `position` (clamped `0..=len`). A
    /// multi-slide generalization of `duplicate_slide`: same persist → reconcile
    /// stage → cache → broadcast → publish → nudge_sync pipeline, so it bumps
    /// `updated_at` (inside `replace_presentation_slides`) and propagates via
    /// #555 sync exactly like every other slide mutation. Unknown ids →
    /// `UnknownSlides` (router maps to 422).
    pub async fn paste_slides(
        &self,
        presentation_id: PresentationId,
        source_ids: Vec<SlideId>,
        position: u32,
    ) -> Result<Vec<Slide>, PasteSlidesError> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();

        if source_ids.is_empty() {
            return Err(PasteSlidesError::UnknownSlides);
        }
        let requested: std::collections::HashSet<SlideId> = source_ids.iter().copied().collect();
        let present: std::collections::HashSet<SlideId> =
            presentation.slides.iter().map(|slide| slide.id).collect();
        if !requested.iter().all(|id| present.contains(id)) {
            return Err(PasteSlidesError::UnknownSlides);
        }

        // Clone the selected slides in the presentation's OWN list order so the
        // pasted block is contiguous and ordered like the source. Each clone
        // gets a fresh id via `Slide::new`; remember (source, clone) to copy the
        // #515 stage-layout marker afterwards.
        let mut block: Vec<Slide> = Vec::new();
        let mut marker_pairs: Vec<(SlideId, SlideId)> = Vec::new();
        for slide in presentation
            .slides
            .iter()
            .filter(|slide| requested.contains(&slide.id))
        {
            let clone = Slide::new(0, slide.content.clone());
            marker_pairs.push((slide.id, clone.id));
            block.push(clone);
        }

        let mut slides = presentation.slides.clone();
        let insert_at = (position as usize).min(slides.len());
        let tail = slides.split_off(insert_at);
        slides.extend(block);
        slides.extend(tail);
        Self::reindex_slides(&mut slides);

        self.repository
            .replace_presentation_slides(presentation_id, &slides)
            .await
            .map_err(anyhow::Error::from)?;

        // #515: copy each source's stage-layout marker to its clone. Non-fatal —
        // the paste already committed (mirrors `duplicate_slide`).
        for (source_id, clone_id) in marker_pairs {
            match self.repository.get_slide_stage_layout(source_id).await {
                Ok(Some(code)) => {
                    if let Err(err) = self
                        .repository
                        .set_slide_stage_layout(presentation_id, clone_id, &code)
                        .await
                    {
                        tracing::warn!(?err, %source_id, "failed to copy stage-layout marker to pasted slide");
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(?err, %source_id, "failed to read stage-layout marker while pasting slide");
                }
            }
        }

        self.reconcile_stage_state_after_edit(presentation_id, &slides)
            .await?;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync();
        Ok(slides)
    }

    pub async fn update_slide_content(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
        main: String,
        translation: String,
        stage: String,
        group: Option<String>,
        metadata_override: Option<SlideMetadata>,
    ) -> anyhow::Result<Slide> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();

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
                Some(presenter_core::SlideGroup::new(trimmed.to_string()))
            }
        });

        let content = SlideContent::new(
            main_text.clone(),
            translation_text.clone(),
            stage_text.clone(),
            group.clone(),
        );
        // Use provided metadata or preserve existing
        let final_metadata = metadata_override.or(existing_slide.metadata.clone());
        let updated_slide = Slide::new(existing_slide.order, content.clone())
            .with_id(slide_id)
            .with_metadata(final_metadata.clone());

        self.repository
            .update_slide_content_with_metadata(
                presentation_id,
                slide_id,
                &content,
                final_metadata.as_ref(),
            )
            .await?;

        let mut updated_presentation = presentation.clone();
        if let Some(slot) = updated_presentation
            .slides
            .iter_mut()
            .find(|slide| slide.id == slide_id)
        {
            *slot = updated_slide.clone();
        }
        self.cache_presentation_value(updated_presentation).await;

        self.broadcast_stage_snapshots().await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });

        self.nudge_sync();

        Ok(updated_slide)
    }

    pub async fn insert_blank_slide(
        &self,
        presentation_id: PresentationId,
        position: Option<u32>,
    ) -> anyhow::Result<Vec<Slide>> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();
        let mut slides = presentation.slides.clone();
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
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync();
        Ok(slides)
    }

    pub async fn duplicate_slide(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
    ) -> anyhow::Result<Vec<Slide>> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();
        let mut slides = presentation.slides.clone();
        let index = slides
            .iter()
            .position(|slide| slide.id == slide_id)
            .ok_or_else(|| anyhow::anyhow!("slide not found"))?;
        let source = slides[index].clone();
        let duplicate = Slide::new(0, source.content.clone());
        let duplicate_id = duplicate.id;
        slides.insert(index + 1, duplicate);
        Self::reindex_slides(&mut slides);
        self.repository
            .replace_presentation_slides(presentation_id, &slides)
            .await?;
        // #515: a duplicate copies ALL slide content — including the stage-
        // layout marker. Non-fatal: the duplicate itself already succeeded.
        match self.repository.get_slide_stage_layout(slide_id).await {
            Ok(Some(code)) => {
                if let Err(err) = self
                    .repository
                    .set_slide_stage_layout(presentation_id, duplicate_id, &code)
                    .await
                {
                    tracing::warn!(?err, %slide_id, "failed to copy stage-layout marker to duplicated slide");
                }
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(?err, %slide_id, "failed to read stage-layout marker while duplicating slide");
            }
        }
        self.reconcile_stage_state_after_edit(presentation_id, &slides)
            .await?;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync();
        Ok(slides)
    }

    pub async fn delete_slide(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
    ) -> anyhow::Result<Vec<Slide>> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();
        let mut slides = presentation.slides.clone();
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
        // #515: a deleted slide's stage-layout marker goes with it. Non-fatal
        // — the slide deletion already committed; a missed row is swept by
        // prune_orphan_slide_stage_layouts on the next library change.
        if let Err(err) = self.repository.clear_slide_stage_layout(slide_id).await {
            tracing::warn!(?err, %slide_id, "failed to clear stage-layout marker of deleted slide");
        }
        self.reconcile_stage_state_after_edit(presentation_id, &slides)
            .await?;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync();
        Ok(slides)
    }

    pub async fn reorder_slides(
        &self,
        presentation_id: PresentationId,
        order: Vec<SlideId>,
    ) -> anyhow::Result<Vec<Slide>> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();
        let mut map = HashMap::new();
        for slide in presentation.slides.clone() {
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
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync();
        Ok(slides)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use presenter_core::{Presentation, SlideText};

    fn blank_slide(order: u32, main: &str) -> Slide {
        Slide::new(
            order,
            SlideContent::new(
                SlideText::new(main).unwrap(),
                SlideText::new("").unwrap(),
                SlideText::new("").unwrap(),
                None,
            ),
        )
    }

    async fn presentation_with_slides(state: &AppState, main_texts: &[&str]) -> Presentation {
        let library = state.create_library("Test Library").await.unwrap();
        let slides: Vec<Slide> = main_texts
            .iter()
            .enumerate()
            .map(|(i, main)| blank_slide(i as u32, main))
            .collect();
        let (_, _, presentation, _) = state
            .create_presentation(library.id, "Test Presentation", Some(&slides))
            .await
            .unwrap();
        presentation
    }

    #[tokio::test]
    async fn reorder_slides_persists_the_exact_requested_order() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B", "C"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();

        // Move the first slide to the end: [A, B, C] -> [B, C, A].
        let requested_order = vec![ids[1], ids[2], ids[0]];
        let result = state
            .reorder_slides(presentation.id, requested_order.clone())
            .await
            .unwrap();

        assert_eq!(
            result.iter().map(|s| s.id).collect::<Vec<_>>(),
            requested_order,
            "the returned slides must be in exactly the requested order"
        );
        assert_eq!(
            result.iter().map(|s| s.order).collect::<Vec<_>>(),
            vec![0, 1, 2],
            "reorder must reindex the `order` field to match the new positions"
        );

        // The reorder must actually PERSIST — re-reading the presentation from
        // the repository (not the in-memory return value) must show the same
        // order, not just the return value of this one call.
        let reloaded = state.presentation_detail(presentation.id).await.unwrap();
        let (_, _, reloaded_presentation) = reloaded.expect("presentation must still exist");
        assert_eq!(
            reloaded_presentation
                .slides
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            requested_order,
            "the new order must be persisted, not just returned"
        );
    }

    #[tokio::test]
    async fn reorder_slides_rejects_a_length_mismatch_instead_of_silently_dropping_slides() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B", "C"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();

        // Only two of the three slide ids: this must fail loudly, not silently
        // drop the third slide from the presentation.
        let short_order = vec![ids[0], ids[1]];
        let result = state.reorder_slides(presentation.id, short_order).await;
        assert!(
            result.is_err(),
            "a slide-count mismatch must be rejected, not silently applied"
        );

        let reloaded = state.presentation_detail(presentation.id).await.unwrap();
        let (_, _, reloaded_presentation) = reloaded.expect("presentation must still exist");
        assert_eq!(
            reloaded_presentation.slides.len(),
            3,
            "a rejected reorder must not have mutated the persisted slide list"
        );
    }

    #[tokio::test]
    async fn paste_clones_selected_slides_as_a_contiguous_block_at_position() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B", "C"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();

        // Copy A and C, paste at gap 1 (after A). New block = clones of A,C in
        // list order → [A, A', C', B, C].
        let result = state
            .paste_slides(presentation.id, vec![ids[0], ids[2]], 1)
            .await
            .unwrap();

        let mains: Vec<String> = result
            .iter()
            .map(|s| s.content.main.value().to_string())
            .collect();
        assert_eq!(mains, vec!["A", "A", "C", "B", "C"]);
        assert_eq!(
            result.iter().map(|s| s.order).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4],
            "paste must reindex order"
        );
        // Clones carry FRESH ids (not the sources').
        assert!(!result[1..3]
            .iter()
            .any(|s| s.id == ids[0] || s.id == ids[2]));
    }

    #[tokio::test]
    async fn paste_clones_full_content_including_group() {
        let state = AppState::in_memory().await.unwrap();
        let library = state.create_library("L").await.unwrap();
        let src = Slide::new(
            0,
            SlideContent::new(
                SlideText::new("main").unwrap(),
                SlideText::new("trans").unwrap(),
                SlideText::new("stage").unwrap(),
                Some(presenter_core::SlideGroup::new("Chorus".to_string())),
            ),
        );
        let (_, _, presentation, _) = state
            .create_presentation(library.id, "P", Some(&[src]))
            .await
            .unwrap();
        let src_id = presentation.slides[0].id;

        let result = state
            .paste_slides(presentation.id, vec![src_id], 1)
            .await
            .unwrap();
        let clone = &result[1];
        assert_eq!(clone.content.main.value(), "main");
        assert_eq!(clone.content.translation.value(), "trans");
        assert_eq!(clone.content.stage.value(), "stage");
        assert_eq!(
            clone.content.group.as_ref().map(|g| g.name().to_string()),
            Some("Chorus".to_string()),
            "group must be cloned intact"
        );
    }

    #[tokio::test]
    async fn paste_clamps_position_past_the_end() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();
        // position 99 clamps to len (append).
        let result = state
            .paste_slides(presentation.id, vec![ids[0]], 99)
            .await
            .unwrap();
        let mains: Vec<String> = result
            .iter()
            .map(|s| s.content.main.value().to_string())
            .collect();
        assert_eq!(mains, vec!["A", "B", "A"]);
    }

    #[tokio::test]
    async fn paste_rejects_an_unknown_slide_id_with_unknownslides() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();
        let result = state
            .paste_slides(presentation.id, vec![ids[0], SlideId::new()], 0)
            .await;
        assert!(matches!(result, Err(PasteSlidesError::UnknownSlides)));
        // The presentation must be UNCHANGED (no half-paste).
        let reloaded = state.presentation_detail(presentation.id).await.unwrap();
        let (_, _, reloaded) = reloaded.expect("still exists");
        assert_eq!(reloaded.slides.len(), 1);
    }

    #[tokio::test]
    async fn paste_persists_and_is_reloadable() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();
        state
            .paste_slides(presentation.id, vec![ids[0]], 0)
            .await
            .unwrap();
        let reloaded = state.presentation_detail(presentation.id).await.unwrap();
        let (_, _, reloaded) = reloaded.expect("still exists");
        let mains: Vec<String> = reloaded
            .slides
            .iter()
            .map(|s| s.content.main.value().to_string())
            .collect();
        assert_eq!(
            mains,
            vec!["A", "A", "B"],
            "paste must persist, not just return"
        );
    }

    #[tokio::test]
    async fn reorder_slides_rejects_an_unknown_slide_id() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();

        let bogus_order = vec![ids[0], SlideId::new()];
        let result = state.reorder_slides(presentation.id, bogus_order).await;
        assert!(
            result.is_err(),
            "an order list naming a slide id that doesn't belong to this presentation must be rejected"
        );
    }
}
