use presenter_core::slide::{BibleSlideMetadata, BibleSlideVerseRef, SlideMetadata};
use presenter_core::{
    BiblePassage, BibleTranslation, PresentationId, Slide, SlideContent, SlideGroup, SlideId,
    SlideText,
};
use std::collections::{BTreeSet, HashMap};

use super::stage::blank_slide_content;
use super::AppState;
use crate::live::LiveEvent;

/// Extracts the short translation code from a full translation code.
/// E.g., "eng-kjv" → "KJV", "sk-roh" → "ROH"
fn translation_short_code(code: &str) -> String {
    code.rsplit('-').next().unwrap_or(code).to_uppercase()
}

/// Typed input for the AI-facing bible composer. A stream of these is
/// produced by the LLM after it edits DB verses against the sermon text,
/// and the server composes slides out of the stream respecting the
/// character limit — same splitting rules as live mode.
#[derive(Debug, Clone)]
pub(crate) enum BibleItem {
    Verse {
        number: u32,
        text: String,
        book: String,
        chapter: u32,
        translation: String,
    },
    Emphasis {
        text: String,
    },
}

/// A slide produced by `compose_bible_items_into_slides`. Plain data —
/// the tool handler wraps it into `BiblePresentationSlide` for persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComposedBibleSlide {
    pub main: String,
    pub main_reference: String,
}

/// Format a sorted set of verse numbers as a reference suffix.
///
/// - Empty set → "" (caller skips the reference entirely).
/// - Single verse → "17".
/// - Contiguous range → "17-20".
/// - Non-contiguous → "1, 3, 5" (flat comma-list, no mixed range syntax).
fn format_verse_range(verses: &BTreeSet<u32>) -> String {
    let v: Vec<u32> = verses.iter().copied().collect();
    if v.is_empty() {
        return String::new();
    }
    let min = v[0];
    let max = v[v.len() - 1];
    let count = v.len() as u32;
    if min == max {
        format!("{}", min)
    } else if max - min + 1 == count {
        format!("{}-{}", min, max)
    } else {
        v.iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Compose a stream of `BibleItem` into slides. Same greedy-packing rule
/// as `compose_bible_slides`: accumulate verses into one slide until the
/// next verse would overflow the character limit, then flush. Emphasis
/// items and translation/book/chapter changes force a slide break.
///
/// If a single verse item is longer than `character_limit`, it is emitted
/// as its own oversized slide — the validator's `MainExceedsCharacterLimit`
/// rule catches this downstream and the LLM sees a rule-keyed error.
pub(crate) fn compose_bible_items_into_slides(
    items: &[BibleItem],
    character_limit: u32,
) -> Vec<ComposedBibleSlide> {
    let limit = character_limit as usize;
    let mut slides: Vec<ComposedBibleSlide> = Vec::new();

    // Pass 1: collect every verse number per (book, chapter, translation)
    // group across the whole items[] stream. Slides flushed in pass 2 use
    // this group's full verse list for the reference label, so all slides
    // of one passage display the same reference (issue #292).
    let mut group_verses: HashMap<(String, u32, String), BTreeSet<u32>> = HashMap::new();
    for item in items {
        if let BibleItem::Verse {
            number,
            book,
            chapter,
            translation,
            ..
        } = item
        {
            group_verses
                .entry((book.clone(), *chapter, translation.clone()))
                .or_default()
                .insert(*number);
        }
    }

    // Accumulator for the current verse slide.
    let mut cur_lines: Vec<String> = Vec::new();
    let mut cur_numbers: Vec<u32> = Vec::new();
    let mut cur_group: Option<(String, u32, String)> = None; // (book, chapter, translation)

    // Flush the current verse accumulator into a slide. The reference is
    // derived from the GROUP's full verse set (built in pass 1), not from
    // cur_numbers, so every slide of one passage displays the same label.
    //
    // Uses let-else pattern matching instead of expect()/unwrap() so there
    // is no panic path — if an invariant is ever broken by a future
    // refactor (e.g., group set without lines), the flush is a no-op
    // rather than a crash.
    let flush_verses =
        |slides: &mut Vec<ComposedBibleSlide>,
         lines: &mut Vec<String>,
         numbers: &mut Vec<u32>,
         group: &mut Option<(String, u32, String)>,
         group_verses: &HashMap<(String, u32, String), BTreeSet<u32>>| {
            if lines.is_empty() {
                *group = None;
                return;
            }
            let Some((book, chapter, translation)) = group.take() else {
                lines.clear();
                numbers.clear();
                return;
            };
            if numbers.is_empty() {
                lines.clear();
                return;
            }
            let main = lines.join("\n");
            let reference = match group_verses.get(&(book.clone(), chapter, translation.clone())) {
                Some(verses) => format!(
                    "{} {}:{} ({})",
                    book,
                    chapter,
                    format_verse_range(verses),
                    translation
                ),
                None => String::new(),
            };
            slides.push(ComposedBibleSlide {
                main,
                main_reference: reference,
            });
            lines.clear();
            numbers.clear();
        };

    for item in items {
        match item {
            BibleItem::Emphasis { text } => {
                flush_verses(
                    &mut slides,
                    &mut cur_lines,
                    &mut cur_numbers,
                    &mut cur_group,
                    &group_verses,
                );
                slides.push(ComposedBibleSlide {
                    main: text.clone(),
                    main_reference: String::new(),
                });
            }
            BibleItem::Verse {
                number,
                text,
                book,
                chapter,
                translation,
            } => {
                // Translation / book / chapter change forces a slide break.
                if let Some((cur_book, cur_chapter, cur_tr)) = &cur_group {
                    if cur_book != book || cur_chapter != chapter || cur_tr != translation {
                        flush_verses(
                            &mut slides,
                            &mut cur_lines,
                            &mut cur_numbers,
                            &mut cur_group,
                            &group_verses,
                        );
                    }
                }

                let line = format!("{}. {}", number, text);
                let existing_len: usize = cur_lines.iter().map(String::len).sum();
                let prospective = if cur_lines.is_empty() {
                    line.len()
                } else {
                    // existing lines joined by "\n" = existing_len + (cur_lines.len() - 1)
                    // plus "\n" + new line = + 1 + line.len()
                    // total = existing_len + cur_lines.len() + line.len()
                    existing_len + cur_lines.len() + line.len()
                };

                if prospective > limit && !cur_lines.is_empty() {
                    flush_verses(
                        &mut slides,
                        &mut cur_lines,
                        &mut cur_numbers,
                        &mut cur_group,
                        &group_verses,
                    );
                }

                cur_lines.push(line);
                cur_numbers.push(*number);
                cur_group = Some((book.clone(), *chapter, translation.clone()));
            }
        }
    }

    flush_verses(
        &mut slides,
        &mut cur_lines,
        &mut cur_numbers,
        &mut cur_group,
        &group_verses,
    );
    slides
}

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

    // Build the full reference label that will appear on all slides (includes translation code)
    let main_short_code = translation_short_code(&main_translation.code);
    let full_reference_label = if full_verse_start == full_verse_end {
        format!(
            "{} {}:{} ({})",
            book, chapter, full_verse_start, main_short_code
        )
    } else {
        format!(
            "{} {}:{}-{} ({})",
            book, chapter, full_verse_start, full_verse_end, main_short_code
        )
    };

    // Build translation reference label if secondary translation is present
    let translation_reference_label = secondary_translation.map(|t| {
        let secondary_short_code = translation_short_code(&t.code);
        if full_verse_start == full_verse_end {
            format!(
                "{} {}:{} ({})",
                book, chapter, full_verse_start, secondary_short_code
            )
        } else {
            format!(
                "{} {}:{}-{} ({})",
                book, chapter, full_verse_start, full_verse_end, secondary_short_code
            )
        }
    });

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
            SlideText::new(&full_reference_label)?,
            Some(SlideGroup::new(&full_reference_label)),
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
            translation_reference_label: translation_reference_label.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::BibleReference;
    use std::collections::HashMap;

    fn test_translation(code: &str) -> BibleTranslation {
        BibleTranslation {
            code: code.to_string(),
            name: code.to_uppercase(),
            language: "test".to_string(),
            show_in_dashboard: true,
            source: None,
        }
    }

    fn test_passage(book: &str, chapter: u16, verse: u16, text: &str) -> BiblePassage {
        BiblePassage::new(
            BibleReference::new(book, chapter, verse, verse).unwrap(),
            test_translation("slk-seb"),
            text.to_string(),
        )
    }

    #[test]
    fn compose_bible_slides_sets_reference_in_stage_and_group() {
        let translation = test_translation("slk-seb");
        let passages = vec![test_passage(
            "Jób",
            2,
            7,
            "Potom satan odišiel spred Hospodina.",
        )];

        let slides =
            compose_bible_slides(&translation, None, &passages, &HashMap::new(), 320, 7, 7)
                .unwrap();

        assert_eq!(slides.len(), 1);
        let slide = &slides[0];

        // main = verse text (with verse number prefix)
        assert!(
            slide.content.main.value().contains("Potom satan"),
            "main should contain verse text, got: {}",
            slide.content.main.value()
        );

        // stage = reference with translation code
        assert_eq!(
            slide.content.stage.value(),
            "Jób 2:7 (SEB)",
            "stage should be the reference label"
        );

        // group = reference with translation code
        let group = slide.content.group.as_ref().expect("group should be set");
        assert_eq!(
            group.name(),
            "Jób 2:7 (SEB)",
            "group should be the reference label"
        );

        // main must NOT contain the reference
        assert!(
            !slide.content.main.value().contains("Jób 2:7"),
            "main must not contain the reference"
        );
    }

    #[test]
    fn compose_bible_slides_multi_verse_range_reference() {
        let translation = test_translation("slk-seb");
        let passages = vec![
            test_passage("Marek", 3, 14, "Vtedy ustanovil Dvanástich."),
            test_passage("Marek", 3, 15, "A aby mali moc vyháňať zlých duchov."),
        ];

        let slides =
            compose_bible_slides(&translation, None, &passages, &HashMap::new(), 320, 14, 15)
                .unwrap();

        assert!(!slides.is_empty());
        let slide = &slides[0];

        // Reference should show verse range
        assert_eq!(
            slide.content.stage.value(),
            "Marek 3:14-15 (SEB)",
            "stage should show verse range reference"
        );

        let group = slide.content.group.as_ref().expect("group should be set");
        assert_eq!(group.name(), "Marek 3:14-15 (SEB)");
    }

    #[test]
    fn compose_bible_slides_with_secondary_translation() {
        let main_translation = test_translation("slk-seb");
        let secondary_translation = test_translation("eng-kjv");
        let passages = vec![test_passage("John", 3, 16, "Lebo Boh tak miloval svet.")];
        let mut secondary = HashMap::new();
        secondary.insert(
            16,
            BiblePassage::new(
                BibleReference::new("John", 3, 16, 16).unwrap(),
                secondary_translation.clone(),
                "For God so loved the world.".to_string(),
            ),
        );

        let slides = compose_bible_slides(
            &main_translation,
            Some(&secondary_translation),
            &passages,
            &secondary,
            320,
            16,
            16,
        )
        .unwrap();

        assert_eq!(slides.len(), 1);
        let slide = &slides[0];

        // main = verse text
        assert!(slide.content.main.value().contains("Boh tak miloval"));

        // translation = secondary verse text
        assert!(
            slide.content.translation.value().contains("God so loved"),
            "translation should contain secondary text, got: {}",
            slide.content.translation.value()
        );

        // stage = main reference (NOT secondary)
        assert_eq!(slide.content.stage.value(), "John 3:16 (SEB)");
    }

    // --- compose_bible_items_into_slides tests ---

    fn verse(number: u32, text: &str) -> BibleItem {
        BibleItem::Verse {
            number,
            text: text.to_string(),
            book: "Ján".to_string(),
            chapter: 1,
            translation: "SEB".to_string(),
        }
    }

    fn emphasis(text: &str) -> BibleItem {
        BibleItem::Emphasis {
            text: text.to_string(),
        }
    }

    #[test]
    fn compose_items_single_short_verse_emits_one_slide() {
        let items = vec![verse(1, "Na počiatku bolo Slovo.")];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].main, "1. Na počiatku bolo Slovo.");
        assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
    }

    #[test]
    fn compose_items_two_verses_that_fit_emit_one_slide_with_range() {
        let items = vec![
            verse(1, "Na počiatku bolo Slovo."),
            verse(2, "Ono bolo na počiatku u Boha."),
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 1);
        assert!(slides[0].main.contains("1. Na počiatku"));
        assert!(slides[0].main.contains("2. Ono bolo"));
        assert_eq!(slides[0].main_reference, "Ján 1:1-2 (SEB)");
    }

    #[test]
    fn compose_items_two_verses_that_overflow_emit_two_slides() {
        // limit = 30; each verse line exceeds ~24 chars; together they exceed 30.
        let items = vec![
            verse(1, "Na počiatku bolo Slovo."), // "1. Na počiatku bolo Slovo."
            verse(2, "Ono bolo na počiatku."),   // "2. Ono bolo na počiatku."
        ];
        let slides = compose_bible_items_into_slides(&items, 30);
        assert_eq!(slides.len(), 2);
        // Both slides show the FULL group reference (verses 1–2) even though
        // they are split across two slides — that is the point of pass 1.
        assert_eq!(slides[0].main_reference, "Ján 1:1-2 (SEB)");
        assert_eq!(slides[1].main_reference, "Ján 1:1-2 (SEB)");
    }

    #[test]
    fn compose_items_emphasis_between_verses_breaks_slide() {
        let items = vec![
            verse(1, "Na počiatku."),
            emphasis("NOVÁ ZMLUVA"),
            verse(2, "Ono bolo."),
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 3);
        assert_eq!(slides[0].main, "1. Na počiatku.");
        // Both verse slides share (Ján, 1, SEB) group → full-range reference on each.
        assert_eq!(slides[0].main_reference, "Ján 1:1-2 (SEB)");
        assert_eq!(slides[1].main, "NOVÁ ZMLUVA");
        assert_eq!(slides[1].main_reference, "");
        assert_eq!(slides[2].main, "2. Ono bolo.");
        assert_eq!(slides[2].main_reference, "Ján 1:1-2 (SEB)");
    }

    #[test]
    fn compose_items_translation_change_forces_break() {
        let items = vec![
            BibleItem::Verse {
                number: 1,
                text: "Na počiatku bolo Slovo.".to_string(),
                book: "Ján".to_string(),
                chapter: 1,
                translation: "SEB".to_string(),
            },
            BibleItem::Verse {
                number: 2,
                text: "Ono bolo na počiatku.".to_string(),
                book: "Ján".to_string(),
                chapter: 1,
                translation: "MIL".to_string(),
            },
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
        assert_eq!(slides[1].main_reference, "Ján 1:2 (MIL)");
    }

    #[test]
    fn compose_items_chapter_change_forces_break() {
        let items = vec![
            BibleItem::Verse {
                number: 14,
                text: "last verse ch1".to_string(),
                book: "Ján".to_string(),
                chapter: 1,
                translation: "SEB".to_string(),
            },
            BibleItem::Verse {
                number: 1,
                text: "first verse ch2".to_string(),
                book: "Ján".to_string(),
                chapter: 2,
                translation: "SEB".to_string(),
            },
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].main_reference, "Ján 1:14 (SEB)");
        assert_eq!(slides[1].main_reference, "Ján 2:1 (SEB)");
    }

    #[test]
    fn compose_items_book_change_forces_break() {
        let items = vec![
            BibleItem::Verse {
                number: 1,
                text: "first".to_string(),
                book: "Ján".to_string(),
                chapter: 1,
                translation: "SEB".to_string(),
            },
            BibleItem::Verse {
                number: 1,
                text: "second".to_string(),
                book: "Marek".to_string(),
                chapter: 1,
                translation: "SEB".to_string(),
            },
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
        assert_eq!(slides[1].main_reference, "Marek 1:1 (SEB)");
    }

    #[test]
    fn compose_items_empty_returns_empty() {
        let slides = compose_bible_items_into_slides(&[], 320);
        assert!(slides.is_empty());
    }

    #[test]
    fn compose_items_single_verse_longer_than_limit_emits_oversized_slide() {
        // Limit 20; verse line is much longer. Composer still emits it —
        // the validator catches oversize downstream.
        let items = vec![verse(1, "Na počiatku bolo Slovo a Slovo bolo u Boha.")];
        let slides = compose_bible_items_into_slides(&items, 20);
        assert_eq!(slides.len(), 1);
        assert!(slides[0].main.len() > 20);
        assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
    }

    #[test]
    fn compose_items_adjacent_emphasis_emit_separate_slides() {
        let items = vec![emphasis("FIRST"), emphasis("SECOND"), verse(1, "verse")];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 3);
        assert_eq!(slides[0].main, "FIRST");
        assert_eq!(slides[1].main, "SECOND");
        assert_eq!(slides[2].main_reference, "Ján 1:1 (SEB)");
    }
}
