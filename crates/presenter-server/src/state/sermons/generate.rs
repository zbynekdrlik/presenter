//! Sermon slides (#851 spec §7.4). Each item is rendered (`render_item`) and
//! composed by the live composer (`compose_bible_slides_labelled`, so the #828
//! rules hold unchanged: the LONGER of main/secondary decides, in `chars()`, a
//! lone oversized verse stays whole) with the item's computed label. The
//! secondary line carries the FULL verses of the kept verse numbers in the
//! sermon's secondary translation, labelled with the secondary book and plain
//! numbers; an unavailable NLT gives slides without the line plus a warning,
//! never a failed write. NLT text stored this way is bounded to the sermon's
//! verses, exactly like the operator's append path.

use std::collections::HashMap;

use presenter_core::bible::sermon::{
    item_label, render_item, verse_suffix, RenderedItem, VerseMark,
};
use presenter_core::{
    BiblePassage, BiblePresentationSlide, BibleReference, BibleSlideId, BibleTranslation, Slide,
};
use presenter_persistence::{SermonItemRecord, SermonRecord};
use uuid::Uuid;

use super::super::slides::{compose_bible_slides_labelled, SlideLabels};
use super::super::AppState;

/// The slides of a whole sermon, in item order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GeneratedSermon {
    pub(crate) slides: Vec<BiblePresentationSlide>,
    /// Slides per item, in item order (`bible_sermon_items.slide_count`).
    pub(crate) slide_counts: Vec<u32>,
    /// Operator-facing warnings (an unavailable secondary translation).
    pub(crate) warnings: Vec<String>,
}

/// The id of an item's `n`-th slide. Stable while the item exists, so a slide
/// selected on the tablet or in the console survives an edit elsewhere.
pub(crate) fn stable_slide_id(item_id: Uuid, n: usize) -> BibleSlideId {
    BibleSlideId::from_uuid(Uuid::new_v5(&item_id, format!("slide/{n}").as_bytes()))
}

impl AppState {
    /// The slides of `items` under the current character limit.
    pub(crate) async fn generate_sermon_slides(
        &self,
        sermon: &SermonRecord,
        items: &[SermonItemRecord],
    ) -> anyhow::Result<GeneratedSermon> {
        let character_limit = self.get_bible_preferences().await?.character_limit;
        let translations = self.list_bible_translations().await?;
        let secondary = sermon
            .secondary_translation_code
            .as_deref()
            .map(|code| translation_or_code(&translations, code));
        let mut generated = GeneratedSermon {
            slides: Vec::new(),
            slide_counts: Vec::with_capacity(items.len()),
            warnings: Vec::new(),
        };
        for item in items {
            let main = translation_or_code(&translations, &item.translation_code);
            let slides = self
                .item_slides(
                    item,
                    &main,
                    secondary.as_ref(),
                    character_limit,
                    &mut generated.warnings,
                )
                .await?;
            generated
                .slide_counts
                .push(u32::try_from(slides.len()).unwrap_or(u32::MAX));
            generated.slides.extend(slides);
        }
        Ok(generated)
    }

    async fn item_slides(
        &self,
        item: &SermonItemRecord,
        main: &BibleTranslation,
        secondary: Option<&BibleTranslation>,
        character_limit: u32,
        warnings: &mut Vec<String>,
    ) -> anyhow::Result<Vec<BiblePresentationSlide>> {
        let rendered = render_item(&item.source, &item.edits);
        if rendered.is_empty() {
            return Ok(Vec::new());
        }
        let passages = rendered_passages(item, main, &rendered)?;
        let (lookup, translation_label) = match secondary {
            Some(translation) => {
                self.sermon_secondary(item, translation, &rendered, warnings)
                    .await?
            }
            None => (HashMap::new(), None),
        };
        let secondary = secondary.filter(|_| translation_label.is_some());
        let labels = SlideLabels {
            main: item_label(
                &item.book_name,
                item.chapter,
                &rendered.suffix,
                &item.translation_code,
            ),
            translation: translation_label,
        };
        let slides = compose_bible_slides_labelled(
            main,
            secondary,
            &passages,
            &lookup,
            character_limit,
            labels,
        )?;
        Ok(slides
            .into_iter()
            .enumerate()
            .map(|(n, slide)| presentation_slide(item.id, n, slide))
            .collect())
    }

    /// The secondary verses of the KEPT verse numbers and their plain label;
    /// nothing when the translation has none of them (plus a warning when it
    /// was unavailable).
    async fn sermon_secondary(
        &self,
        item: &SermonItemRecord,
        translation: &BibleTranslation,
        rendered: &RenderedItem,
        warnings: &mut Vec<String>,
    ) -> anyhow::Result<(HashMap<u16, BiblePassage>, Option<String>)> {
        let kept: Vec<u16> = rendered.verses.iter().map(|verse| verse.verse).collect();
        let (Some(&first), Some(&last)) = (kept.first(), kept.last()) else {
            return Ok((HashMap::new(), None));
        };
        let (mut lookup, warning) = self
            .secondary_verse_lookup(
                translation,
                &item.book_name,
                Some(item.book_code.as_str()),
                item.chapter,
                first,
                last,
            )
            .await?;
        if let Some(warning) = warning {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
        lookup.retain(|verse, _| kept.contains(verse));
        let marks: Vec<VerseMark> = kept
            .iter()
            .copied()
            .filter(|verse| lookup.contains_key(verse))
            .map(VerseMark::plain)
            .collect();
        let Some(book) = lookup.values().next().map(|p| p.reference.book.clone()) else {
            return Ok((HashMap::new(), None));
        };
        let label = item_label(
            &book,
            item.chapter,
            &verse_suffix(&marks),
            &translation.code,
        );
        Ok((lookup, Some(label)))
    }
}

/// The installed translation, or a bare stand-in when it was removed after the
/// item was added: the snapshot still renders (spec §5.7).
fn translation_or_code(translations: &[BibleTranslation], code: &str) -> BibleTranslation {
    translations
        .iter()
        .find(|translation| translation.code.eq_ignore_ascii_case(code))
        .cloned()
        .unwrap_or_else(|| BibleTranslation::new(code, code, ""))
}

/// The rendered verses as passages for the composer: the stored book name and
/// code, one passage per kept verse.
fn rendered_passages(
    item: &SermonItemRecord,
    main: &BibleTranslation,
    rendered: &RenderedItem,
) -> anyhow::Result<Vec<BiblePassage>> {
    let mut passages = Vec::with_capacity(rendered.verses.len());
    for verse in &rendered.verses {
        let reference = BibleReference::new_with_code(
            item.book_name.as_str(),
            item.book_code.as_str(),
            item.book_number,
            item.chapter,
            verse.verse,
            verse.verse,
        )?;
        passages.push(BiblePassage::new(
            reference,
            main.clone(),
            verse.text.clone(),
        ));
    }
    Ok(passages)
}

/// A composed slide as a stored presentation slide with its stable id; the
/// labels come from the composer's metadata, like the `/bible/resolve` DTO.
fn presentation_slide(item_id: Uuid, n: usize, slide: Slide) -> BiblePresentationSlide {
    let bible = slide.metadata.and_then(|metadata| metadata.bible);
    let (main_reference, secondary_reference) = bible
        .as_ref()
        .map(|meta| {
            (
                meta.main_reference_label.clone().unwrap_or_default(),
                meta.translation_reference_label.clone().unwrap_or_default(),
            )
        })
        .unwrap_or_default();
    BiblePresentationSlide {
        id: stable_slide_id(item_id, n),
        order: 0,
        main: slide.content.main,
        main_reference,
        secondary: slide.content.translation,
        secondary_reference,
        metadata: bible,
    }
}
