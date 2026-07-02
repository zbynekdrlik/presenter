//! AppState slide-edit operations: update content, insert blank, duplicate,
//! delete, and reorder slides. Each persists, reconciles stage state, updates
//! the presentation cache, and publishes a `BibleSlidesChanged` live event.

use presenter_core::slide::SlideMetadata;
use presenter_core::{PresentationId, Slide, SlideContent, SlideId, SlideText};
use std::collections::HashMap;

use super::super::stage::blank_slide_content;
use super::super::AppState;
use crate::live::LiveEvent;

impl AppState {
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
        Ok(slides)
    }
}
