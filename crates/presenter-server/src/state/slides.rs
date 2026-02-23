use presenter_core::slide::{BibleSlideMetadata, BibleSlideVerseRef, SlideMetadata};
use presenter_core::{
    BiblePassage, BibleTranslation, PresentationId, Slide, SlideContent, SlideId, SlideText,
};
use std::collections::HashMap;

use super::stage::blank_slide_content;
use super::AppState;

pub(crate) fn compose_bible_slides(
    main_translation: &BibleTranslation,
    secondary_translation: Option<&BibleTranslation>,
    main_passages: &[BiblePassage],
    secondary_lookup: &HashMap<u16, BiblePassage>,
    character_limit: u32,
    full_verse_start: u16,
    full_verse_end: u16,
) -> anyhow::Result<Vec<Slide>> {
    let mut slides: Vec<Slide> = Vec::new();
    if main_passages.is_empty() {
        return Ok(slides);
    }

    let book = main_passages[0].reference.book.clone();
    let book_code = main_passages[0].reference.book_code.clone();
    let book_number = main_passages[0].reference.book_number;
    let chapter = main_passages[0].reference.chapter;

    // Build the full reference label that will appear on all slides
    let full_reference_label = if full_verse_start == full_verse_end {
        format!("{} {}:{}", book, chapter, full_verse_start)
    } else {
        format!(
            "{} {}:{}-{}",
            book, chapter, full_verse_start, full_verse_end
        )
    };

    let mut current_main = String::new();
    let mut current_tr = String::new();
    let mut verses_meta: Vec<(u16, u16)> = Vec::new();

    let push_slide = |slides: &mut Vec<Slide>,
                      main: String,
                      tr: String,
                      verses: &[(u16, u16)]|
     -> anyhow::Result<()> {
        if main.trim().is_empty() {
            return Ok(());
        }
        let content = SlideContent::new(
            SlideText::new(&main)?,
            SlideText::new(&tr)?,
            SlideText::new(&main)?,
            None,
        );
        let metadata = SlideMetadata::new().with_bible(BibleSlideMetadata {
            translation_code: main_translation.code.clone(),
            secondary_translation_code: secondary_translation.map(|t| t.code.clone()),
            book: book.clone(),
            book_code: book_code.clone(),
            book_number,
            chapter,
            verses: verses
                .iter()
                .map(|(s, e)| BibleSlideVerseRef::new(*s, *e))
                .collect(),
            main_reference_label: Some(full_reference_label.clone()),
            translation_reference_label: None,
        });
        slides.push(Slide::new(slides.len() as u32, content).with_metadata(Some(metadata)));
        Ok(())
    };

    for p in main_passages {
        let label = format!("{}. ", p.reference.verse_start);
        let line = format!("{}{}", label, p.text);
        let prospective_len =
            current_main.len() + if current_main.is_empty() { 0 } else { 1 } + line.len();
        if prospective_len > character_limit as usize && !current_main.is_empty() {
            // flush
            push_slide(
                &mut slides,
                current_main.clone(),
                current_tr.clone(),
                &verses_meta,
            )?;
            current_main.clear();
            current_tr.clear();
            verses_meta.clear();
        }
        if !current_main.is_empty() {
            current_main.push('\n');
        }
        current_main.push_str(&line);
        // translation (secondary)
        if let Some(sec) = secondary_lookup.get(&p.reference.verse_start) {
            let tr_label = format!("{}. ", sec.reference.verse_start);
            let tr_line = format!("{}{}", tr_label, sec.text);
            if !current_tr.is_empty() {
                current_tr.push('\n');
            }
            current_tr.push_str(&tr_line);
        }
        verses_meta.push((p.reference.verse_start, p.reference.verse_end));
    }

    // final flush
    if !current_main.is_empty() {
        push_slide(&mut slides, current_main, current_tr, &verses_meta)?;
    }

    Ok(slides)
}

impl AppState {
    pub async fn update_slide_content(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
        main: String,
        translation: String,
        stage: String,
        group: Option<String>,
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
        let updated_slide = Slide::new(existing_slide.order, content.clone()).with_id(slide_id);

        self.repository
            .update_slide_content(presentation_id, slide_id, &content)
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
        slides.insert(index + 1, Slide::new(0, source.content.clone()));
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
        self.reconcile_stage_state_after_edit(presentation_id, &slides)
            .await?;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
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
        Ok(slides)
    }
}
