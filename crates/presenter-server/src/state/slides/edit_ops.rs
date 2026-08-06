//! AppState slide-edit operations: update content, insert blank, duplicate,
//! delete, and reorder slides. Each persists, reconciles stage state, updates
//! the presentation cache, and publishes a `BibleSlidesChanged` live event.

use presenter_core::slide::SlideMetadata;
use presenter_core::{Presentation, PresentationId, Slide, SlideContent, SlideId, SlideText};
use presenter_persistence::RepositoryError;
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
    /// The request's `anchorSlideId` no longer names a slide in this
    /// presentation — a concurrent structural edit (another tab's
    /// delete/reorder/paste) removed it between when the client read the
    /// list and when this request landed (#558 V8). The router maps this
    /// to 409 so the client refreshes instead of guessing a position.
    AnchorVanished,
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
            Self::AnchorVanished => write!(f, "the paste anchor slide no longer exists"),
            Self::Internal(err) => write!(f, "{err}"),
        }
    }
}

impl AppState {
    /// #558 V3: `reconcile_stage_state_after_edit` runs AFTER the mutation
    /// has already committed to the DB — a failure here must NEVER turn a
    /// SUCCESSFUL edit into a client-visible 500 (the client would then
    /// retry the whole request, duplicating an already-applied paste /
    /// insert / duplicate / delete / reorder). Log WARN and swallow; the
    /// next trigger/snapshot self-heals any stage drift.
    async fn reconcile_stage_after_edit_best_effort(
        &self,
        presentation_id: PresentationId,
        slides: &[Slide],
    ) {
        if let Err(err) = self
            .reconcile_stage_state_after_edit(presentation_id, slides)
            .await
        {
            tracing::warn!(
                ?err,
                %presentation_id,
                "post-commit stage reconcile failed after a slide edit — the edit already \
                 committed, continuing"
            );
        }
    }

    /// #558 V3: same rationale as `reconcile_stage_after_edit_best_effort`,
    /// for `broadcast_stage_snapshots` — a failure here must never turn a
    /// committed edit into a client-visible 500 either.
    async fn broadcast_stage_best_effort(&self, presentation_id: PresentationId) {
        if let Err(err) = self.broadcast_stage_snapshots().await {
            tracing::warn!(
                ?err,
                %presentation_id,
                "post-commit stage broadcast failed after a slide edit — the edit already \
                 committed, continuing"
            );
        }
    }

    /// #558 V2/W3/W7: acquire the shared per-presentation lock (the SAME
    /// registry a concurrent sync apply takes — `state/sync.rs`) and read
    /// the presentation straight from the DATABASE, never through the
    /// in-memory cache. This is the shared opening for every
    /// read-modify-write slide-edit op (snapshot-replace AND
    /// content-update alike): hold the returned guard for the ENTIRE
    /// read + write + cache-refresh sequence.
    ///
    /// #558 W7: reading through the cache here (even right after an
    /// eviction) has a narrow but real race — an UNRELATED, unlocked reader
    /// (anything that also calls the cache-populating read path without
    /// holding this lock) can repopulate the cache in the gap between our
    /// own eviction and our own read with a snapshot from whenever ITS OWN
    /// read happened to start, which is not bounded by our critical
    /// section. Reading the DB directly sidesteps the shared cache
    /// entirely for this op's OWN view of the data, so no concurrent
    /// reader's cache write can poison it. The cache is still refreshed
    /// (by the caller) AFTER the mutation commits, for every OTHER reader's
    /// benefit.
    async fn lock_and_read_presentation_for_edit(
        &self,
        presentation_id: PresentationId,
    ) -> anyhow::Result<(tokio::sync::OwnedMutexGuard<()>, Presentation)> {
        let guard = self.presentation_locks.lock(presentation_id).await;
        let (_, _, presentation) = self
            .repository
            .fetch_presentation_detail(presentation_id)
            .await?
            .ok_or(RepositoryError::NotFound("presentation not found"))?;
        Ok((guard, presentation))
    }

    /// Paste-of-COPY (#554): clone the named slides' full content and insert the
    /// clones as a contiguous block anchored at `anchor_slide_id` (the slide the
    /// gap precedes; `None` = end — #558 V8). A multi-slide generalization of
    /// `duplicate_slide`: same persist → reconcile stage → cache → broadcast →
    /// publish → nudge_sync pipeline, so it bumps `updated_at` (inside
    /// `replace_presentation_slides`) and propagates via #555 sync exactly like
    /// every other slide mutation. Unknown source ids → `UnknownSlides`; an
    /// anchor that no longer exists → `AnchorVanished` (router maps both to a
    /// client-visible error, 422 / 409 respectively).
    pub async fn paste_slides(
        &self,
        presentation_id: PresentationId,
        source_ids: Vec<SlideId>,
        anchor_slide_id: Option<SlideId>,
    ) -> Result<Vec<Slide>, PasteSlidesError> {
        // #558 V2/W7: hold the per-presentation lock across the ENTIRE
        // read + write + cache-refresh sequence — a concurrent sync apply of
        // this SAME presentation takes the same lock (see `state/sync.rs`),
        // so the two can never interleave. The read is straight from the
        // DB, never the cache (see `lock_and_read_presentation_for_edit`).
        let (_guard, presentation) = self
            .lock_and_read_presentation_for_edit(presentation_id)
            .await?;

        if source_ids.is_empty() {
            return Err(PasteSlidesError::UnknownSlides);
        }
        let requested: std::collections::HashSet<SlideId> = source_ids.iter().copied().collect();
        let present: std::collections::HashSet<SlideId> =
            presentation.slides.iter().map(|slide| slide.id).collect();
        if !requested.iter().all(|id| present.contains(id)) {
            return Err(PasteSlidesError::UnknownSlides);
        }

        // #558 V8: resolve the insertion index from the ANCHOR SLIDE ID,
        // inside the SAME lock as the read above — never a raw index a
        // concurrent structural edit (another tab's delete/reorder) could
        // have shifted underneath the client between when it read the list
        // and when this request landed. `None` = insert at the end.
        let insert_at = match anchor_slide_id {
            None => presentation.slides.len(),
            Some(anchor_id) => presentation
                .slides
                .iter()
                .position(|slide| slide.id == anchor_id)
                .ok_or(PasteSlidesError::AnchorVanished)?,
        };

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
        let tail = slides.split_off(insert_at);
        slides.extend(block);
        slides.extend(tail);
        Self::reindex_slides(&mut slides);

        self.repository
            .replace_presentation_slides(presentation_id, &slides)
            .await?;

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

        self.reconcile_stage_after_edit_best_effort(presentation_id, &slides)
            .await;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_best_effort(presentation_id).await;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync().await;
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
        // #558 W3/W7: a content update is a read-modify-write against the
        // cache exactly like the snapshot-replace ops below — it takes the
        // SAME shared per-presentation lock (so it can never interleave
        // with a concurrent sync apply on this presentation either) and
        // reads straight from the DB, never the cache.
        let (_guard, presentation) = self
            .lock_and_read_presentation_for_edit(presentation_id)
            .await?;

        let existing_slide = presentation
            .slides
            .iter()
            .find(|slide| slide.id == slide_id)
            .ok_or(RepositoryError::NotFound("slide not found"))?
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

        self.broadcast_stage_best_effort(presentation_id).await;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });

        self.nudge_sync().await;

        Ok(updated_slide)
    }

    pub async fn insert_blank_slide(
        &self,
        presentation_id: PresentationId,
        position: Option<u32>,
    ) -> anyhow::Result<Vec<Slide>> {
        // #558 V2/W7: see `paste_slides` — same shared per-presentation lock,
        // read straight from the DB.
        let (_guard, presentation) = self
            .lock_and_read_presentation_for_edit(presentation_id)
            .await?;
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
        self.reconcile_stage_after_edit_best_effort(presentation_id, &slides)
            .await;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_best_effort(presentation_id).await;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync().await;
        Ok(slides)
    }

    pub async fn duplicate_slide(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
    ) -> anyhow::Result<Vec<Slide>> {
        // #558 V2/W7: see `paste_slides` — same shared per-presentation lock,
        // read straight from the DB.
        let (_guard, presentation) = self
            .lock_and_read_presentation_for_edit(presentation_id)
            .await?;
        let mut slides = presentation.slides.clone();
        let index = slides
            .iter()
            .position(|slide| slide.id == slide_id)
            .ok_or(RepositoryError::NotFound("slide not found"))?;
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
        self.reconcile_stage_after_edit_best_effort(presentation_id, &slides)
            .await;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_best_effort(presentation_id).await;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync().await;
        Ok(slides)
    }

    pub async fn delete_slide(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
    ) -> anyhow::Result<Vec<Slide>> {
        // #558 V2/W7: see `paste_slides` — same shared per-presentation lock,
        // read straight from the DB.
        let (_guard, presentation) = self
            .lock_and_read_presentation_for_edit(presentation_id)
            .await?;
        let mut slides = presentation.slides.clone();
        let index = slides
            .iter()
            .position(|slide| slide.id == slide_id)
            .ok_or(RepositoryError::NotFound("slide not found"))?;
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
        self.reconcile_stage_after_edit_best_effort(presentation_id, &slides)
            .await;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_best_effort(presentation_id).await;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync().await;
        Ok(slides)
    }

    pub async fn reorder_slides(
        &self,
        presentation_id: PresentationId,
        order: Vec<SlideId>,
    ) -> anyhow::Result<Vec<Slide>> {
        // #558 V2/W7: see `paste_slides` — same shared per-presentation lock,
        // read straight from the DB.
        let (_guard, presentation) = self
            .lock_and_read_presentation_for_edit(presentation_id)
            .await?;
        let mut map = HashMap::new();
        for slide in presentation.slides.clone() {
            map.insert(slide.id, slide);
        }
        if order.len() != map.len() {
            // #628: typed refusal — a bare anyhow! here fell through to 500.
            // #652 F5: reclassified from `TargetNotFound` (422) to
            // `Conflict` (409) — this is a STALE-SET conflict (the body was
            // built against a slide count that has since changed via a
            // concurrent edit), not a body-referenced missing target; the
            // client should refresh and retry, mirroring
            // `PasteSlidesError::AnchorVanished`'s 409. The per-id lookup
            // guard below (a genuinely UNKNOWN slide id) stays
            // `TargetNotFound`/422.
            return Err(RepositoryError::Conflict("slide order length mismatch").into());
        }
        let mut slides = Vec::with_capacity(order.len());
        for id in order {
            let slide = map.remove(&id).ok_or(RepositoryError::TargetNotFound(
                "unknown slide in reorder request",
            ))?;
            slides.push(slide);
        }
        Self::reindex_slides(&mut slides);
        self.repository
            .replace_presentation_slides(presentation_id, &slides)
            .await?;
        self.reconcile_stage_after_edit_best_effort(presentation_id, &slides)
            .await;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_best_effort(presentation_id).await;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync().await;
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

        // Copy A and C, paste anchored on B (gap 1 = before B, i.e. after A —
        // #558 V8: the anchor names the slide the gap PRECEDES). New block =
        // clones of A,C in list order → [A, A', C', B, C].
        let result = state
            .paste_slides(presentation.id, vec![ids[0], ids[2]], Some(ids[1]))
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

        // Single-slide presentation: gap 1 == len == the end → no anchor.
        let result = state
            .paste_slides(presentation.id, vec![src_id], None)
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
    async fn paste_with_no_anchor_appends_at_the_end() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();
        // #558 V8: `None` = append at the end (replaces the old raw
        // "position past the end clamps" behavior — there is no longer a
        // raw index to clamp).
        let result = state
            .paste_slides(presentation.id, vec![ids[0]], None)
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
            .paste_slides(presentation.id, vec![ids[0], SlideId::new()], Some(ids[0]))
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
            .paste_slides(presentation.id, vec![ids[0]], Some(ids[0]))
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
    async fn paste_anchored_resolution_still_lands_in_the_right_gap_after_a_concurrent_delete() {
        // #558 V8: a raw clamped index would land in the WRONG gap once
        // another tab's concurrent delete shifts the list underneath it.
        // Anchoring on the SLIDE ID the gap precedes keeps the paste correct
        // regardless of what else changed the list order/length meanwhile.
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B", "C"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();

        // Intent: paste before C. Concurrently (simulated sequentially, as
        // the lock would force it): B is deleted first, shifting C from
        // index 2 to index 1.
        state.delete_slide(presentation.id, ids[1]).await.unwrap();

        let result = state
            .paste_slides(presentation.id, vec![ids[0]], Some(ids[2]))
            .await
            .unwrap();
        let mains: Vec<String> = result
            .iter()
            .map(|s| s.content.main.value().to_string())
            .collect();
        assert_eq!(
            mains,
            vec!["A", "A", "C"],
            "the clone lands right before C — the anchor, not a stale raw index"
        );
    }

    #[tokio::test]
    async fn paste_fails_loudly_when_the_anchor_slide_itself_vanished() {
        // #558 V8: when the anchor slide is the one that vanished (not just
        // shifted), there is no honest gap to resolve — fail loudly so the
        // client refreshes instead of silently guessing a position.
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B", "C"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();

        state.delete_slide(presentation.id, ids[1]).await.unwrap();

        let result = state
            .paste_slides(presentation.id, vec![ids[0]], Some(ids[1]))
            .await;
        assert!(
            matches!(result, Err(PasteSlidesError::AnchorVanished)),
            "a paste anchored on a slide that no longer exists must fail loudly"
        );
        // No half-paste: the presentation is unchanged (still 2 slides: A, C).
        let reloaded = state.presentation_detail(presentation.id).await.unwrap();
        let (_, _, reloaded) = reloaded.expect("still exists");
        assert_eq!(reloaded.slides.len(), 2);
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

    #[tokio::test]
    async fn a_snapshot_replace_edit_never_clobbers_a_sync_apply_that_landed_first() {
        // #558 V2/W7: every snapshot-replace edit op used to read the
        // presentation from AppState's in-memory CACHE, never a fresh DB
        // read. A sync apply writes DIRECTLY to the DB and is invisible to
        // that cache — an edit op landing after such an apply, but still
        // holding a snapshot cached from BEFORE it, silently overwrote the
        // synced content with its own stale-based result (and its own
        // `touch` would then LWW-propagate that loss back to the peer as
        // the "newer" copy). W7 closed this by having edit ops read the DB
        // DIRECTLY instead of through the cache at all — this reproduces
        // the exact scenario that used to clobber: warm the cache with a
        // stale (pre-sync) snapshot (mirrors a normal prior read, e.g.
        // opening the song in the editor), apply a sync update directly to
        // the DB (bypassing AppState, exactly like the real sync engine
        // does), then run an edit op and prove it builds on the FRESH
        // (synced) content from the DB, never the stale cached snapshot.
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["Original"]).await;

        // Warm the cache with the pre-sync snapshot.
        state.cache_presentation_ref(&presentation).await;

        // A peer sync apply lands directly in the DB, bypassing AppState
        // entirely — the cache warmed above is now stale relative to the DB.
        use presenter_persistence::entities::presentation as presentation_entity;
        use sea_orm::EntityTrait;
        let row = presentation_entity::Entity::find_by_id(presentation.id.to_string())
            .one(state.repository().connection())
            .await
            .unwrap()
            .expect("presentation row exists");
        let incoming = presenter_persistence::SyncPresentation {
            sync_id: row.sync_id,
            library_name: "Test Library".to_string(),
            name: "Test Presentation".to_string(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
            slides: vec![blank_slide(0, "PeerA"), blank_slide(1, "PeerB")],
        };
        state
            .repository()
            .apply_sync_presentation(&incoming, &std::collections::HashSet::new())
            .await
            .unwrap();

        // The edit op must build on the FRESH (synced) content, never the
        // stale cached "Original" snapshot.
        let result = state
            .insert_blank_slide(presentation.id, None)
            .await
            .unwrap();
        let mains: Vec<String> = result
            .iter()
            .map(|s| s.content.main.value().to_string())
            .collect();
        assert_eq!(
            mains,
            vec!["PeerA", "PeerB", ""],
            "the insert must be built on top of the synced content, not overwrite it with a \
             stale pre-sync snapshot"
        );

        // And persistence agrees — this isn't just the in-memory return value.
        let reloaded = state.presentation_detail(presentation.id).await.unwrap();
        let (_, _, reloaded) = reloaded.expect("still exists");
        let reloaded_mains: Vec<String> = reloaded
            .slides
            .iter()
            .map(|s| s.content.main.value().to_string())
            .collect();
        assert_eq!(reloaded_mains, vec!["PeerA", "PeerB", ""]);
    }

    #[tokio::test]
    async fn duplicate_slide_succeeds_even_when_post_commit_broadcast_fails() {
        // #558 V3: `reconcile_stage_state_after_edit` / `broadcast_stage_snapshots`
        // run AFTER `replace_presentation_slides` already committed — a
        // failure there must never turn a SUCCESSFUL mutation into a
        // client-visible error (the client would then retry the whole
        // request, duplicating an already-applied edit). Forces a REAL
        // post-commit broadcast failure: presentation `active` is the
        // on-stage song and gets a slide corrupted directly in the DB
        // (bypassing all validation) so re-reading it while building the
        // broadcast snapshot errors — then an edit on a DIFFERENT,
        // unrelated presentation must still succeed.
        let state = AppState::in_memory().await.unwrap();
        let library = state.create_library("V3 Library").await.unwrap();
        let (_, _, active, _) = state
            .create_presentation(
                library.id,
                "On Stage Song",
                Some(&[blank_slide(0, "On stage")]),
            )
            .await
            .unwrap();
        state
            .update_stage_state(active.id, active.slides[0].id, None, None, None)
            .await
            .unwrap();

        use presenter_persistence::entities::slide as slide_entity;
        use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter};
        slide_entity::Entity::update_many()
            .col_expr(
                slide_entity::Column::WorshipMain,
                Expr::value("x".repeat(5000)),
            )
            .filter(slide_entity::Column::Id.eq(active.slides[0].id.to_string()))
            .exec(state.repository().connection())
            .await
            .unwrap();

        // A completely different, unrelated presentation (SAME library) —
        // its own edit must succeed even though building the
        // (unconditional) stage broadcast now fails re-reading the
        // corrupted `active` song.
        let (_, _, other, _) = state
            .create_presentation(
                library.id,
                "Other Song",
                Some(&[blank_slide(0, "A"), blank_slide(1, "B")]),
            )
            .await
            .unwrap();
        let result = state.duplicate_slide(other.id, other.slides[0].id).await;
        assert!(
            result.is_ok(),
            "a post-commit broadcast failure must never surface as an error from a \
             successfully-committed edit: {result:?}"
        );

        let reloaded = state.presentation_detail(other.id).await.unwrap();
        let (_, _, reloaded) = reloaded.expect("still exists");
        assert_eq!(
            reloaded.slides.len(),
            3,
            "the duplicate DID commit despite the swallowed broadcast failure"
        );
    }
}
