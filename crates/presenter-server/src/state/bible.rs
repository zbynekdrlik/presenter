//! Bible search, broadcast, and ingestion methods for [`AppState`].

use super::AppState;
use crate::bible_remote::RemoteBibleError;
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
use presenter_persistence::RepositoryError;
use std::collections::HashMap;

/// Optional text overrides for triggered Bible slides (when the user edits text in the UI).
#[derive(Debug, Default)]
pub struct BibleTriggerOverrides {
    pub main_text: Option<String>,
    pub translation_text: Option<String>,
}

/// The secondary side of a legacy trigger (#824): its text, its translation
/// code and the book name that translation uses (for the translate reference).
#[derive(Debug, Default)]
struct TriggerSecondary {
    text: Option<String>,
    translation_code: Option<String>,
    book: Option<String>,
}

/// What [`AppState::generate_bible_slides`] produced. `secondary_warning`
/// is set when the secondary translation could not be read (the NLT API is
/// unavailable, #826): the slides then carry the main text only, and the
/// caller shows the warning to the operator.
#[derive(Debug)]
pub struct GeneratedBibleSlides {
    pub main_translation: BibleTranslation,
    pub secondary_translation: Option<BibleTranslation>,
    pub slides: Vec<Slide>,
    pub secondary_warning: Option<String>,
}

/// Secondary verses keyed by verse number, plus the operator warning when the
/// secondary translation was unavailable.
type SecondaryVerses = (HashMap<u16, presenter_core::BiblePassage>, Option<String>);

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
    // Translation listing and every verse/structure read live in
    // `bible_source.rs` (#826: the repository-or-remote dispatch).

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
    ) -> anyhow::Result<GeneratedBibleSlides> {
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

        let (secondary_lookup, secondary_warning) = match secondary_translation {
            Some(ref tr) => {
                self.secondary_verse_lookup(
                    tr,
                    book,
                    canonical_book_code.as_deref(),
                    chapter,
                    verse_start,
                    verse_end,
                )
                .await?
            }
            None => (HashMap::new(), None),
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

        Ok(GeneratedBibleSlides {
            main_translation,
            secondary_translation,
            slides,
            secondary_warning,
        })
    }

    /// The secondary translation's verses by verse number. A remote
    /// translation that is unavailable right now (the NLT API unreachable or
    /// answering unusably, #826) gives no verses plus a warning for the
    /// operator, so the main text still loads during a service; any other
    /// error still fails the load.
    async fn secondary_verse_lookup(
        &self,
        translation: &BibleTranslation,
        book: &str,
        book_code: Option<&str>,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
    ) -> anyhow::Result<SecondaryVerses> {
        let fetched = self
            .bible_passage_range(
                &translation.code,
                book,
                book_code,
                chapter,
                verse_start,
                verse_end,
            )
            .await;
        match fetched {
            Ok(passages) => Ok((
                passages
                    .into_iter()
                    .map(|p| (p.reference.verse_start, p))
                    .collect(),
                None,
            )),
            Err(err) => match err.downcast_ref::<RemoteBibleError>() {
                Some(remote) => {
                    tracing::warn!(
                        translation = %translation.code,
                        error = %remote,
                        "secondary Bible translation unavailable — loading the main text only"
                    );
                    Ok((
                        HashMap::new(),
                        Some(format!("{remote} — sekundárny preklad vynechaný")),
                    ))
                }
                None => Err(err),
            },
        }
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
            // #587: typed refusal (#584 pattern) — the router downcasts to
            // `RepositoryError` and maps `NotFound` to 404 instead of a bare 500.
            .ok_or(RepositoryError::NotFound("bible presentation not found"))?;

        let before = presentation.slides.len();
        presentation.slides.retain(|s| s.id != slide_id);
        if presentation.slides.len() == before {
            return Err(RepositoryError::NotFound("slide not found in presentation").into());
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
            // #587: typed refusal (#584 pattern), see delete_bible_slide above.
            .ok_or(RepositoryError::NotFound("bible presentation not found"))?;

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
            // #608: typed refusal (#584/#586 pattern), for consistency with the other
            // `update_bible_slide` refusal below and with `state/bible.rs`'s other converted
            // sites — the router downcasts to `RepositoryError` and maps `NotFound` to 404
            // instead of a bare 500 (TOCTOU-only in practice: masked by the router's own
            // pre-checks under normal traffic, but a delete racing the pre-check would hit this).
            .ok_or(RepositoryError::NotFound("bible presentation not found"))?;

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
            updated_slide.ok_or(RepositoryError::NotFound("slide not found in presentation"))?;

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
        self.bible.broadcast.read().await.clone()
    }

    pub async fn trigger_bible_passage(
        &self,
        translation_code: &str,
        reference: &BibleReference,
        overrides: BibleTriggerOverrides,
    ) -> anyhow::Result<BibleBroadcast> {
        let passage = self
            .trigger_main_passage(translation_code, reference, overrides.main_text)
            .await?;
        let secondary = self
            .trigger_secondary_text(reference, overrides.translation_text)
            .await?;

        let broadcast = BibleBroadcast::new(passage, Utc::now());
        {
            let mut guard = self.bible.broadcast.write().await;
            *guard = Some(broadcast.clone());
        }
        self.live_hub.publish(LiveEvent::Bible {
            broadcast: broadcast.clone(),
        });
        self.resolume_registry
            .bible_update(BibleUpdate {
                passage: Some(broadcast.clone()),
                secondary_text: secondary.text,
                secondary_translation_code: secondary.translation_code,
                secondary_book: secondary.book,
                slide_output: None, // Legacy path - no slide output
            })
            .await;
        Ok(broadcast)
    }

    /// The main passage of a legacy trigger: the client's edited text, else
    /// the verse range as numbered paragraphs, else the exact passage row.
    /// Read through the #826 dispatch, so a remote translation works too.
    async fn trigger_main_passage(
        &self,
        translation_code: &str,
        reference: &BibleReference,
        main_text: Option<String>,
    ) -> anyhow::Result<presenter_core::BiblePassage> {
        // Try to find a range of verses first (for multi-verse slides)
        let range = self
            .bible_passage_range(
                translation_code,
                reference.book.as_str(),
                reference.book_code.as_deref(),
                reference.chapter,
                reference.verse_start,
                reference.verse_end,
            )
            .await?;
        let translation = match range.first() {
            Some(first) => first.translation.clone(),
            // Fall back to exact match for single-verse passages
            None => {
                let exact = self
                    .find_bible_passage(translation_code, reference)
                    .await?
                    .ok_or(RepositoryError::NotFound("passage not found"))?;
                if main_text.is_none() {
                    return Ok(exact);
                }
                exact.translation
            }
        };
        // The edited text from the client wins; otherwise combine the verses.
        // The original reference already carries the correct range.
        let text = main_text.unwrap_or_else(|| numbered_verse_text(&range));
        Ok(presenter_core::BiblePassage::new(
            reference.clone(),
            translation,
            text,
        ))
    }

    /// The secondary side of a legacy trigger: the client's edited text (under
    /// the saved secondary translation), else the saved secondary
    /// translation's verses — plus the book name that translation uses (#824).
    /// A secondary translation that cannot be read right now (e.g. the NLT API
    /// is down, #826) is logged and left out — the main passage still goes on
    /// air.
    async fn trigger_secondary_text(
        &self,
        reference: &BibleReference,
        translation_text: Option<String>,
    ) -> anyhow::Result<TriggerSecondary> {
        let prefs = self.get_bible_preferences().await?;
        let Some(code) = prefs.secondary_translation else {
            return Ok(TriggerSecondary {
                text: translation_text.filter(|text| !text.is_empty()),
                ..TriggerSecondary::default()
            });
        };
        // The main reference may carry only the MAIN book name ("1 Ján" from
        // the AI tool); the secondary rows are found by the canonical code.
        let book_code = secondary_book_code(reference);
        if let Some(text) = translation_text {
            let book = if text.is_empty() {
                None
            } else {
                self.first_verse_book(&code, reference, book_code.as_deref())
                    .await
            };
            return Ok(TriggerSecondary {
                text: (!text.is_empty()).then_some(text),
                translation_code: Some(code),
                book,
            });
        }
        let range = self
            .bible_passage_range(
                &code,
                reference.book.as_str(),
                book_code.as_deref(),
                reference.chapter,
                reference.verse_start,
                reference.verse_end,
            )
            .await
            .unwrap_or_else(|err| {
                tracing::warn!(
                    translation = %code,
                    reference = %reference.to_human_readable(),
                    error = %err,
                    "secondary Bible translation unavailable — triggering without it"
                );
                Vec::new()
            });
        let Some(first) = range.first() else {
            return Ok(TriggerSecondary::default());
        };
        Ok(TriggerSecondary {
            book: Some(first.reference.book.clone()),
            text: Some(numbered_verse_text(&range)),
            translation_code: Some(code),
        })
    }

    /// The book name `translation_code` uses for `reference`'s book, read from
    /// the passage's first verse (one cheap lookup); `None` when unreadable.
    async fn first_verse_book(
        &self,
        translation_code: &str,
        reference: &BibleReference,
        book_code: Option<&str>,
    ) -> Option<String> {
        let first = self
            .bible_passage_range(
                translation_code,
                reference.book.as_str(),
                book_code,
                reference.chapter,
                reference.verse_start,
                reference.verse_start,
            )
            .await
            .ok()?;
        first
            .into_iter()
            .next()
            .map(|passage| passage.reference.book)
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
            let mut guard = self.bible.slide_output.write().await;
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
            let mut guard = self.bible.broadcast.write().await;
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
        self.bible.slide_output.read().await.clone()
    }

    pub async fn clear_bible_broadcast(&self) {
        {
            let mut guard = self.bible.broadcast.write().await;
            *guard = None;
        }
        {
            let mut guard = self.bible.slide_output.write().await;
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
        if let Some(ingestion) = &self.bible.ingestion_override {
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
        self.bible.ingestion_override = Some(ingestion);
    }
}

/// The canonical book code to look a SECONDARY translation up by: the
/// reference's own code, else the code of its (main-language) book name.
fn secondary_book_code(reference: &BibleReference) -> Option<String> {
    reference.book_code.clone().or_else(|| {
        presenter_core::bible::canonical_book_by_name(&reference.book)
            .map(|book| book.code.to_string())
    })
}

/// Verses as `"N. text"` paragraphs separated by a blank line — the legacy
/// trigger's multi-verse text.
fn numbered_verse_text(range: &[presenter_core::BiblePassage]) -> String {
    range
        .iter()
        .map(|entry| format!("{}. {}", entry.reference.verse_start, entry.text))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod trigger_tests;
