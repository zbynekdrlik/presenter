//! Bible search, broadcast, and ingestion methods for [`AppState`].

use super::AppState;
use crate::live::LiveEvent;
use crate::resolume::BibleUpdate;
use chrono::Utc;
use presenter_bible::BibleImportSummary;
use presenter_core::{
    slide::BibleSlideMetadata, BibleBroadcast, BiblePreferences, BiblePresentation,
    BiblePresentationId, BiblePresentationSlide, BiblePresentationSummary, BibleReference,
    BibleSlideId, BibleSlideOutput, BibleTranslation, Slide, SlideText,
};
use presenter_importer::bible::BibleIngestionService;
use std::collections::HashMap;

/// Optional text overrides for triggered Bible slides (when the user edits text in the UI).
#[derive(Debug, Default)]
pub struct BibleTriggerOverrides {
    pub main_text: Option<String>,
    pub translation_text: Option<String>,
    #[allow(dead_code)] // Prepared for reference label editing in Bible trigger UI
    pub main_reference_label: Option<String>,
    #[allow(dead_code)] // Prepared for reference label editing in Bible trigger UI
    pub translation_reference_label: Option<String>,
}

/// Returns a safe placeholder BibleReference for legacy broadcast fallback.
/// The hardcoded values (empty book, chapter=1, verses 1-1) are guaranteed valid.
fn placeholder_bible_reference() -> BibleReference {
    // These values always satisfy validation: chapter > 0, verse_start <= verse_end
    BibleReference::new("", 1, 1, 1).unwrap_or_else(|e| {
        // This branch should never execute with valid hardcoded values,
        // but we log and create a minimal reference just in case.
        tracing::error!(
            ?e,
            "placeholder_bible_reference: unexpected validation failure"
        );
        // Last resort: create reference with same values - if this fails too,
        // something is fundamentally broken in the validation logic
        // Direct construction bypasses validation — safe because values are hardcoded valid.
        BibleReference {
            book: "Genesis".to_string(),
            book_code: None,
            book_number: None,
            chapter: 1,
            verse_start: 1,
            verse_end: 1,
        }
    })
}

/// Optional reference metadata for the legacy broadcast (backwards compatibility)
#[derive(Debug, Default)]
pub struct BibleSlideReferenceMetadata {
    pub translation_code: Option<String>,
    pub book: Option<String>,
    pub book_code: Option<String>,
    pub book_number: Option<u16>,
    pub chapter: Option<u16>,
    pub verse_start: Option<u16>,
    pub verse_end: Option<u16>,
}

impl AppState {
    // Bible translation methods
    pub async fn list_bible_translations(&self) -> anyhow::Result<Vec<BibleTranslation>> {
        self.repository.list_bible_translations().await
    }

    pub async fn update_bible_translation(
        &self,
        code: &str,
        name: Option<&str>,
        language: Option<&str>,
        show_in_dashboard: Option<bool>,
    ) -> anyhow::Result<Option<BibleTranslation>> {
        self.repository
            .update_bible_translation(code, name, language, show_in_dashboard)
            .await
    }

    // Bible passage search methods
    pub async fn search_bible_passages_cross(
        &self,
        translation_code: Option<&str>,
        query: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<presenter_core::BiblePassage>> {
        self.repository
            .search_bible_passages_cross(translation_code, query, limit)
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

    /// Load a contiguous verse range from the DB in a single query. Used
    /// by the AI `load_bible_verses` tool to return raw per-verse objects
    /// to the LLM. Thin wrapper over the repository method — no splitting,
    /// no grouping, no composition; the caller decides what to do with
    /// the verses.
    pub async fn bible_passage_range(
        &self,
        translation_code: &str,
        book: &str,
        book_code: Option<&str>,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
    ) -> anyhow::Result<Vec<presenter_core::BiblePassage>> {
        self.repository
            .bible_passage_range(
                translation_code,
                book,
                book_code,
                chapter,
                verse_start,
                verse_end,
            )
            .await
    }

    pub async fn list_bible_books(
        &self,
        translation_code: &str,
    ) -> anyhow::Result<Vec<presenter_core::bible::BibleBookChapterSummary>> {
        self.repository
            .bible_book_chapter_summaries(translation_code)
            .await
    }

    pub async fn bible_book_chapter_summaries(
        &self,
        translation_code: &str,
    ) -> anyhow::Result<Vec<presenter_core::bible::BibleBookChapterSummary>> {
        self.repository
            .bible_book_chapter_summaries(translation_code)
            .await
    }

    pub async fn generate_bible_slides(
        &self,
        main_translation_code: &str,
        secondary_translation_code: Option<&str>,
        book: &str,
        book_code: Option<&str>,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
        character_limit: u32,
    ) -> anyhow::Result<(BibleTranslation, Option<BibleTranslation>, Vec<Slide>)> {
        let translations = self.list_bible_translations().await?;
        let main_translation = translations
            .iter()
            .find(|t| t.code.eq_ignore_ascii_case(main_translation_code))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown main translation"))?;

        let secondary_translation = if let Some(code) = secondary_translation_code {
            translations
                .into_iter()
                .find(|t| t.code.eq_ignore_ascii_case(code))
        } else {
            None
        };

        let main_passages = self
            .repository
            .bible_passage_range(
                &main_translation.code,
                book,
                book_code,
                chapter,
                verse_start,
                verse_end,
            )
            .await?;

        let canonical_book_code = main_passages
            .first()
            .and_then(|p| p.reference.book_code.clone());

        let secondary_lookup: HashMap<u16, presenter_core::BiblePassage> =
            if let Some(ref tr) = secondary_translation {
                let passages = self
                    .repository
                    .bible_passage_range(
                        &tr.code,
                        book,
                        canonical_book_code.as_deref(),
                        chapter,
                        verse_start,
                        verse_end,
                    )
                    .await?;
                passages
                    .into_iter()
                    .map(|p| (p.reference.verse_start, p))
                    .collect()
            } else {
                HashMap::new()
            };

        let slides = super::slides::compose_bible_slides(
            &main_translation,
            secondary_translation.as_ref(),
            &main_passages,
            &secondary_lookup,
            character_limit,
            verse_start,
            verse_end,
        )?;

        Ok((main_translation, secondary_translation, slides))
    }

    // Bible presentation methods
    pub async fn list_bible_presentations(&self) -> anyhow::Result<Vec<BiblePresentationSummary>> {
        self.repository.list_bible_presentation_summaries().await
    }

    pub async fn bible_presentation_detail(
        &self,
        id: BiblePresentationId,
    ) -> anyhow::Result<Option<BiblePresentation>> {
        self.repository.fetch_bible_presentation(id).await
    }

    pub async fn create_bible_presentation(&self, name: &str) -> anyhow::Result<BiblePresentation> {
        let presentation = self.repository.create_bible_presentation(name).await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation.id.to_string(),
        });
        Ok(presentation)
    }

    pub async fn rename_bible_presentation(
        &self,
        id: BiblePresentationId,
        name: &str,
    ) -> anyhow::Result<()> {
        self.repository.rename_bible_presentation(id, name).await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: id.to_string(),
        });
        Ok(())
    }

    pub async fn delete_bible_presentation(&self, id: BiblePresentationId) -> anyhow::Result<()> {
        self.repository.delete_bible_presentation(id).await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: id.to_string(),
        });
        Ok(())
    }

    pub async fn append_bible_presentation_slides(
        &self,
        id: BiblePresentationId,
        new_slides: Vec<BiblePresentationSlide>,
    ) -> anyhow::Result<BiblePresentation> {
        // Note: empty slides are permitted — the operator UI's "add empty
        // slide" button intentionally creates placeholder slides that the
        // operator fills in by editing text in place.
        let presentation = self
            .repository
            .append_bible_presentation_slides(id, &new_slides)
            .await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation.id.to_string(),
        });
        Ok(presentation)
    }

    /// Delete a single slide from a bible presentation. Implemented via
    /// fetch + modify + replace_all — bible presentation slide counts are
    /// small (typically a few to a few dozen), so read-modify-write is fine.
    pub async fn delete_bible_slide(
        &self,
        presentation_id: BiblePresentationId,
        slide_id: BibleSlideId,
    ) -> anyhow::Result<BiblePresentation> {
        let mut presentation = self
            .repository
            .fetch_bible_presentation(presentation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("bible presentation not found"))?;

        let before = presentation.slides.len();
        presentation.slides.retain(|s| s.id != slide_id);
        if presentation.slides.len() == before {
            return Err(anyhow::anyhow!("slide not found in presentation"));
        }

        self.repository
            .replace_bible_presentation_slides(presentation_id, &presentation.slides)
            .await?;

        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });

        // Re-fetch to get the normalized orders assigned by replace_all.
        self.repository
            .fetch_bible_presentation(presentation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("bible presentation disappeared after delete"))
    }

    /// Reorder slides in a bible presentation by providing the desired slide
    /// ID sequence. Missing IDs are dropped; unknown IDs are ignored.
    pub async fn reorder_bible_slides(
        &self,
        presentation_id: BiblePresentationId,
        slide_ids: Vec<BibleSlideId>,
    ) -> anyhow::Result<BiblePresentation> {
        let presentation = self
            .repository
            .fetch_bible_presentation(presentation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("bible presentation not found"))?;

        // Build a lookup of existing slides by ID.
        let mut by_id: HashMap<BibleSlideId, BiblePresentationSlide> = presentation
            .slides
            .iter()
            .cloned()
            .map(|s| (s.id, s))
            .collect();

        let mut reordered: Vec<BiblePresentationSlide> = Vec::with_capacity(by_id.len());
        for id in slide_ids {
            if let Some(slide) = by_id.remove(&id) {
                reordered.push(slide);
            }
        }
        // Append any slides not mentioned in the reorder list to preserve them.
        for slide in presentation.slides {
            if by_id.contains_key(&slide.id) {
                reordered.push(slide.clone());
                by_id.remove(&slide.id);
            }
        }

        self.repository
            .replace_bible_presentation_slides(presentation_id, &reordered)
            .await?;

        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });

        self.repository
            .fetch_bible_presentation(presentation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("bible presentation disappeared after reorder"))
    }

    /// Replace a single slide within a bible presentation. Implemented via
    /// fetch + modify + replace_all — bible presentation slide counts are
    /// small (typically a few to a few dozen), so read-modify-write is fine.
    pub async fn update_bible_slide(
        &self,
        presentation_id: BiblePresentationId,
        slide_id: BibleSlideId,
        main_text: String,
        main_reference: String,
        secondary_text: String,
        secondary_reference: String,
        metadata: Option<BibleSlideMetadata>,
    ) -> anyhow::Result<BiblePresentationSlide> {
        let mut presentation = self
            .repository
            .fetch_bible_presentation(presentation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("bible presentation not found"))?;

        let main =
            SlideText::new(main_text).map_err(|err| anyhow::anyhow!("invalid main text: {err}"))?;
        let secondary = SlideText::new(secondary_text)
            .map_err(|err| anyhow::anyhow!("invalid secondary text: {err}"))?;

        let mut updated_slide: Option<BiblePresentationSlide> = None;
        for slide in &mut presentation.slides {
            if slide.id == slide_id {
                slide.main = main.clone();
                slide.main_reference = main_reference.clone();
                slide.secondary = secondary.clone();
                slide.secondary_reference = secondary_reference.clone();
                slide.metadata = metadata.clone();
                updated_slide = Some(slide.clone());
                break;
            }
        }
        let updated_slide =
            updated_slide.ok_or_else(|| anyhow::anyhow!("slide not found in presentation"))?;

        self.repository
            .replace_bible_presentation_slides(presentation_id, &presentation.slides)
            .await?;

        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });

        Ok(updated_slide)
    }

    // Bible preferences (persisted via app_settings)
    pub async fn get_bible_preferences(&self) -> anyhow::Result<BiblePreferences> {
        let key = "bible-preferences";
        match self.repository.get_app_setting(key).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(BiblePreferences::default()),
        }
    }

    pub async fn set_bible_preferences(&self, prefs: BiblePreferences) -> anyhow::Result<()> {
        let key = "bible-preferences";
        let json = serde_json::to_string(&prefs)?;
        self.repository.set_app_setting(key, &json).await?;
        self.live_hub.publish(LiveEvent::BiblePreferencesChanged {
            character_limit: prefs.character_limit,
        });
        Ok(())
    }

    // Bible broadcast methods
    pub async fn active_bible_broadcast(&self) -> Option<BibleBroadcast> {
        self.bible_broadcast.read().await.clone()
    }

    pub async fn trigger_bible_passage(
        &self,
        translation_code: &str,
        reference: &BibleReference,
        overrides: BibleTriggerOverrides,
    ) -> anyhow::Result<BibleBroadcast> {
        // Try to find a range of verses first (for multi-verse slides)
        let range = self
            .repository
            .bible_passage_range(
                translation_code,
                reference.book.as_str(),
                reference.book_code.as_deref(),
                reference.chapter,
                reference.verse_start,
                reference.verse_end,
            )
            .await?;

        let passage = if let Some(main_text) = overrides.main_text {
            // Use the text override from the client (edited slide)
            let translation = if range.is_empty() {
                self.repository
                    .find_bible_passage(translation_code, reference)
                    .await?
                    .map(|p| p.translation)
                    .ok_or_else(|| anyhow::anyhow!("passage not found"))?
            } else {
                range[0].translation.clone()
            };
            presenter_core::BiblePassage::new(reference.clone(), translation, main_text)
        } else if range.is_empty() {
            // Fall back to exact match for single-verse passages
            self.repository
                .find_bible_passage(translation_code, reference)
                .await?
                .ok_or_else(|| anyhow::anyhow!("passage not found"))?
        } else {
            // Combine multiple verses into a single passage
            let mut combined_text = String::new();
            for entry in &range {
                if !combined_text.is_empty() {
                    combined_text.push_str("\n\n");
                }
                let label = format!("{}. ", entry.reference.verse_start);
                combined_text.push_str(&label);
                combined_text.push_str(entry.text.as_str());
            }
            let translation = range[0].translation.clone();
            // Use the original reference directly - it already has the correct range
            presenter_core::BiblePassage::new(reference.clone(), translation, combined_text)
        };

        // Fetch secondary translation text if configured
        let (secondary_text, secondary_translation_code) =
            if let Some(translation_text) = overrides.translation_text {
                // Use the translation text override from the client
                let prefs = self.get_bible_preferences().await?;
                let sec_code = prefs.secondary_translation.clone();
                if translation_text.is_empty() {
                    (None, sec_code)
                } else {
                    (Some(translation_text), sec_code)
                }
            } else {
                let prefs = self.get_bible_preferences().await?;
                if let Some(ref sec_code) = prefs.secondary_translation {
                    let sec_range = self
                        .repository
                        .bible_passage_range(
                            sec_code,
                            reference.book.as_str(),
                            reference.book_code.as_deref(),
                            reference.chapter,
                            reference.verse_start,
                            reference.verse_end,
                        )
                        .await
                        .unwrap_or_default();
                    if sec_range.is_empty() {
                        (None, None)
                    } else {
                        let mut sec_text = String::new();
                        for entry in &sec_range {
                            if !sec_text.is_empty() {
                                sec_text.push_str("\n\n");
                            }
                            let label = format!("{}. ", entry.reference.verse_start);
                            sec_text.push_str(&label);
                            sec_text.push_str(entry.text.as_str());
                        }
                        (Some(sec_text), Some(sec_code.clone()))
                    }
                } else {
                    (None, None)
                }
            };

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
                secondary_text,
                secondary_translation_code,
                slide_output: None, // Legacy path - no slide output
            })
            .await;
        Ok(broadcast)
    }

    /// Trigger a Bible slide using the single-source-of-truth output.
    /// This method does NOT fetch from the database - it uses the provided content directly.
    pub async fn trigger_bible_slide_output(
        &self,
        output: BibleSlideOutput,
        reference_metadata: BibleSlideReferenceMetadata,
    ) {
        // Store as the new active output
        {
            let mut guard = self.bible_slide_output.write().await;
            *guard = Some(output.clone());
        }
        // Also update legacy bible_broadcast for backwards compatibility with /bible/active endpoint
        // Use reference metadata if available, otherwise use placeholder
        let reference = if let (Some(book), Some(chapter), Some(verse_start)) = (
            reference_metadata.book.as_deref(),
            reference_metadata.chapter,
            reference_metadata.verse_start,
        ) {
            let verse_end = reference_metadata.verse_end.unwrap_or(verse_start);
            if let (Some(book_code), Some(book_number)) = (
                reference_metadata.book_code.as_deref(),
                reference_metadata.book_number,
            ) {
                // Try with book code first, fall back to without, then to placeholder
                BibleReference::new_with_code(
                    book,
                    book_code,
                    book_number,
                    chapter,
                    verse_start,
                    verse_end,
                )
                .or_else(|_| BibleReference::new(book, chapter, verse_start, verse_end))
                .unwrap_or_else(|_| placeholder_bible_reference())
            } else {
                BibleReference::new(book, chapter, verse_start, verse_end)
                    .unwrap_or_else(|_| placeholder_bible_reference())
            }
        } else {
            placeholder_bible_reference()
        };

        let translation = if let Some(code) = reference_metadata.translation_code {
            presenter_core::BibleTranslation::new(code, "", "")
        } else {
            presenter_core::BibleTranslation::new("", "", "")
        };

        let legacy_broadcast = BibleBroadcast::new(
            presenter_core::BiblePassage::new(reference, translation, output.main_text.clone()),
            output.triggered_at,
        )
        .with_reference_label(output.main_reference.clone());
        {
            let mut guard = self.bible_broadcast.write().await;
            *guard = Some(legacy_broadcast.clone());
        }
        // Publish to WebSocket subscribers (both old and new formats)
        self.live_hub.publish(LiveEvent::Bible {
            broadcast: legacy_broadcast,
        });
        self.live_hub.publish(LiveEvent::BibleSlide {
            output: output.clone(),
        });
        // Send to Resolume
        self.resolume_registry
            .bible_update(BibleUpdate::from_slide_output(Some(output)))
            .await;
    }

    /// Get the current active Bible slide output.
    /// Used by `/bible/active-slide` endpoint (stage page initial load).
    pub async fn active_bible_slide_output(&self) -> Option<BibleSlideOutput> {
        self.bible_slide_output.read().await.clone()
    }

    pub async fn clear_bible_broadcast(&self) {
        {
            let mut guard = self.bible_broadcast.write().await;
            *guard = None;
        }
        {
            let mut guard = self.bible_slide_output.write().await;
            *guard = None;
        }
        self.live_hub.publish(LiveEvent::BibleCleared);
        self.resolume_registry
            .bible_update(BibleUpdate::from_slide_output(None))
            .await;
    }

    // Bible ingestion
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

    #[cfg(test)]
    pub fn set_test_bible_ingestion(
        &mut self,
        ingestion: std::sync::Arc<dyn super::seed::TestBibleIngestion + Send + Sync>,
    ) {
        self.bible_ingestion_override = Some(ingestion);
    }
}
