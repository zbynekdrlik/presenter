//! Sermon slides (#851 spec §7.4). Each item is rendered (`render_item`) and
//! composed by the live composer (`compose_bible_slides_labelled`, so the #828
//! rules hold unchanged: the LONGER of main/secondary decides, in `chars()`, a
//! lone oversized verse stays whole) with the item's computed label. The
//! secondary line carries the FULL verses of the kept verse numbers in the
//! sermon's secondary translation, labelled with the secondary book and plain
//! numbers; an unavailable NLT gives slides without the line plus a warning,
//! never a failed write, and a line stored earlier stays. NLT text stored this way is bounded to the sermon's
//! verses, exactly like the operator's append path.

use std::collections::HashMap;

use futures_util::future::join_all;
use presenter_core::bible::sermon::{
    item_label, render_item, verse_suffix, RenderedItem, VerseMark,
};
use presenter_core::{
    BiblePassage, BiblePresentationId, BiblePresentationSlide, BibleReference, BibleSlideId,
    BibleTranslation, Slide,
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

/// The secondary rows of one item's whole range, and the warning when the
/// translation was unavailable. The full range is asked even when an edge verse
/// is struck: the remote cache keys on the range, so an edit stays a cache hit.
#[derive(Debug, Default)]
struct ItemSecondary {
    verses: HashMap<u16, BiblePassage>,
    warning: Option<String>,
}

type StoredVerse = (String, u16, u16);

impl AppState {
    /// The slides of `items` under the current character limit. Lookups run at
    /// once: a sermon of ten items must not ask a slow remote API ten times in
    /// a row while its revision stays open.
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
        let lookups = join_all(
            items
                .iter()
                .map(|item| self.item_secondary(item, secondary.as_ref())),
        )
        .await
        .into_iter()
        .collect::<anyhow::Result<Vec<_>>>()?;
        let stored = match &secondary {
            Some(translation) if lookups.iter().any(|lookup| lookup.warning.is_some()) => {
                self.stored_secondary(sermon.id, translation).await?
            }
            _ => HashMap::new(),
        };
        let mut generated = GeneratedSermon {
            slides: Vec::new(),
            slide_counts: Vec::with_capacity(items.len()),
            warnings: Vec::new(),
        };
        for (item, lookup) in items.iter().zip(lookups) {
            let main = translation_or_code(&translations, &item.translation_code);
            let slides = item_slides(
                item,
                &main,
                secondary.as_ref(),
                character_limit,
                lookup,
                &stored,
                &mut generated.warnings,
            )?;
            generated
                .slide_counts
                .push(u32::try_from(slides.len()).unwrap_or(u32::MAX));
            generated.slides.extend(slides);
        }
        Ok(generated)
    }

    /// The secondary verses of an item; nothing for a wholly struck item or
    /// when the secondary is the item's own translation: its full verse under
    /// the cut one would put the struck words back on the wall.
    async fn item_secondary(
        &self,
        item: &SermonItemRecord,
        secondary: Option<&BibleTranslation>,
    ) -> anyhow::Result<ItemSecondary> {
        let Some(translation) = secondary else {
            return Ok(ItemSecondary::default());
        };
        if translation
            .code
            .eq_ignore_ascii_case(&item.translation_code)
            || render_item(&item.source, &item.edits).is_empty()
        {
            return Ok(ItemSecondary::default());
        }
        let (verses, warning) = self
            .secondary_verse_lookup(
                translation,
                &item.book_name,
                Some(item.book_code.as_str()),
                item.chapter,
                item.verse_start,
                item.verse_end,
            )
            .await?;
        Ok(ItemSecondary { verses, warning })
    }

    /// The secondary lines the sermon already stores, read back from its
    /// slides: an outage must not take away an English line prepared earlier.
    async fn stored_secondary(
        &self,
        id: BiblePresentationId,
        translation: &BibleTranslation,
    ) -> anyhow::Result<HashMap<StoredVerse, BiblePassage>> {
        let slides = self
            .repository
            .fetch_bible_presentation(id)
            .await?
            .map(|presentation| presentation.slides)
            .unwrap_or_default();
        let mut stored = HashMap::new();
        for slide in &slides {
            stored_lines(slide, translation, &mut stored)?;
        }
        Ok(stored)
    }
}

/// Adds the `"{verse}. {text}"` lines of `slide`'s secondary text to `stored`,
/// when the slide was made with `translation`.
fn stored_lines(
    slide: &BiblePresentationSlide,
    translation: &BibleTranslation,
    stored: &mut HashMap<StoredVerse, BiblePassage>,
) -> anyhow::Result<()> {
    let Some(meta) = slide.metadata.as_ref() else {
        return Ok(());
    };
    let (Some(code), Some(number)) = (meta.book_code.as_deref(), meta.book_number) else {
        return Ok(());
    };
    let same_translation = meta
        .secondary_translation_code
        .as_deref()
        .is_some_and(|stored_code| stored_code.eq_ignore_ascii_case(&translation.code));
    let chapter = meta.chapter;
    let book = slide
        .secondary_reference
        .rsplit_once(&format!(" {chapter}:"))
        .map(|(book, _)| book);
    let (true, Some(book)) = (same_translation, book) else {
        return Ok(());
    };
    for line in slide.secondary.value().lines() {
        let Some((verse, text)) = line.split_once(". ") else {
            continue;
        };
        let Ok(verse) = verse.parse::<u16>() else {
            continue;
        };
        let reference = BibleReference::new_with_code(book, code, number, chapter, verse, verse)?;
        let passage = BiblePassage::new(reference, translation.clone(), text.to_string());
        stored.insert((code.to_string(), chapter, verse), passage);
    }
    Ok(())
}

fn item_slides(
    item: &SermonItemRecord,
    main: &BibleTranslation,
    secondary: Option<&BibleTranslation>,
    character_limit: u32,
    lookup: ItemSecondary,
    stored: &HashMap<StoredVerse, BiblePassage>,
    warnings: &mut Vec<String>,
) -> anyhow::Result<Vec<BiblePresentationSlide>> {
    let rendered = render_item(&item.source, &item.edits);
    if rendered.is_empty() {
        return Ok(Vec::new());
    }
    let passages = rendered_passages(item, main, &rendered)?;
    let (verses, translation_label) = match secondary {
        Some(translation) => kept_secondary(item, translation, &rendered, lookup, stored, warnings),
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
        &verses,
        character_limit,
        labels,
    )?;
    Ok(slides
        .into_iter()
        .enumerate()
        .map(|(n, slide)| presentation_slide(item.id, n, slide))
        .collect())
}

/// The secondary verses of the KEPT verse numbers and their plain label. A
/// verse an outage hides comes from `stored`; the warning stays only when one
/// is still missing.
fn kept_secondary(
    item: &SermonItemRecord,
    translation: &BibleTranslation,
    rendered: &RenderedItem,
    lookup: ItemSecondary,
    stored: &HashMap<StoredVerse, BiblePassage>,
    warnings: &mut Vec<String>,
) -> (HashMap<u16, BiblePassage>, Option<String>) {
    let kept: Vec<u16> = rendered.verses.iter().map(|verse| verse.verse).collect();
    let ItemSecondary {
        mut verses,
        warning,
    } = lookup;
    verses.retain(|verse, _| kept.contains(verse));
    if let Some(warning) = warning {
        for verse in &kept {
            let key = (item.book_code.clone(), item.chapter, *verse);
            if let Some(passage) = stored.get(&key) {
                verses.entry(*verse).or_insert_with(|| passage.clone());
            }
        }
        let missing = kept.iter().any(|verse| !verses.contains_key(verse));
        if missing && !warnings.contains(&warning) {
            warnings.push(warning);
        }
    }
    let marks: Vec<VerseMark> = kept
        .iter()
        .copied()
        .filter(|verse| verses.contains_key(verse))
        .map(VerseMark::plain)
        .collect();
    let Some(book) = verses.values().next().map(|p| p.reference.book.clone()) else {
        return (HashMap::new(), None);
    };
    let label = item_label(
        &book,
        item.chapter,
        &verse_suffix(&marks),
        &translation.code,
    );
    (verses, Some(label))
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
