//! Bible search, broadcast, and ingestion methods for [`AppState`].

use super::AppState;
use crate::live::LiveEvent;
use crate::resolume::BibleUpdate;
use chrono::Utc;
use presenter_bible::BibleImportSummary;
use presenter_core::{
    BibleBroadcast, BiblePreferences, BibleReference, BibleTranslation, Presentation,
    PresentationId, Slide,
};
use presenter_importer::bible::BibleIngestionService;
use std::collections::HashMap;

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
    pub async fn search_bible_passages(
        &self,
        translation_code: &str,
        query: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<presenter_core::BiblePassage>> {
        self.repository
            .search_bible_passages(translation_code, query, limit)
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
        )?;

        Ok((main_translation, secondary_translation, slides))
    }

    // Bible presentation methods
    pub async fn list_bible_presentations(
        &self,
    ) -> anyhow::Result<Vec<presenter_core::PresentationSummary>> {
        let libraries = self.repository.fetch_libraries().await?;
        if let Some(bible_lib) = libraries
            .into_iter()
            .find(|l| l.name.eq_ignore_ascii_case("Bible"))
        {
            let summaries = bible_lib
                .presentations
                .into_iter()
                .map(|p| presenter_core::PresentationSummary::new(p.id, p.name))
                .collect();
            Ok(summaries)
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn bible_presentation_detail(
        &self,
        id: PresentationId,
    ) -> anyhow::Result<Option<Presentation>> {
        let result = self.repository.fetch_presentation_detail(id).await?;
        Ok(result.map(|(_, _, presentation)| presentation))
    }

    pub async fn create_bible_presentation(&self, name: &str) -> anyhow::Result<Presentation> {
        // Ensure a Bible library exists
        let libraries = self.repository.fetch_libraries().await?;
        let library = if let Some(existing) = libraries
            .into_iter()
            .find(|l| l.name.eq_ignore_ascii_case("Bible"))
        {
            existing
        } else {
            self.repository.create_library("Bible").await?
        };
        let (_lib_id, _lib_name, presentation) = self
            .repository
            .create_presentation(library.id, name, None)
            .await?;
        Ok(presentation)
    }

    pub async fn rename_bible_presentation(
        &self,
        id: PresentationId,
        name: &str,
    ) -> anyhow::Result<()> {
        self.repository.rename_presentation(id, name).await
    }

    pub async fn append_bible_presentation_slides(
        &self,
        id: PresentationId,
        new_slides: Vec<Slide>,
    ) -> anyhow::Result<Presentation> {
        let (_, _, mut presentation) = self
            .repository
            .fetch_presentation_detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("presentation not found"))?;
        // Filter out empty placeholder slides created by default
        // (empty main, translation, and stage text)
        presentation.slides.retain(|s| {
            !s.content.main.value().is_empty()
                || !s.content.translation.value().is_empty()
                || !s.content.stage.value().is_empty()
        });
        let start_order = presentation.slides.len() as u32;
        for (i, mut slide) in new_slides.into_iter().enumerate() {
            slide.order = start_order + i as u32;
            presentation.slides.push(slide);
        }
        self.repository
            .replace_presentation_slides(id, &presentation.slides)
            .await?;
        Ok(presentation)
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

        let passage = if range.is_empty() {
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
                combined_text.push_str(entry.text.as_str());
            }
            let translation = range[0].translation.clone();
            // Use the original reference directly - it already has the correct range
            presenter_core::BiblePassage::new(reference.clone(), translation, combined_text)
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
            })
            .await;
        Ok(broadcast)
    }

    pub async fn clear_bible_broadcast(&self) {
        {
            let mut guard = self.bible_broadcast.write().await;
            *guard = None;
        }
        self.live_hub.publish(LiveEvent::BibleCleared);
        self.resolume_registry
            .bible_update(BibleUpdate { passage: None })
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
