//! The desktop Bible work area in the mockup v3 look (#875, section
//! "3 · Biblia v rovnakom rámci"): the title row's passage name, each card's
//! caption and which card is on the wall. Pure, host-tested; the views live in
//! `pages/bible_slides.rs`.

use presenter_core::bible::sermon::translation_short_code;
use presenter_core::BibleSlideOutput;

use crate::api::bible::{BibleSlideDto, BibleSlideMetaBible};

/// "Ján 3:16-18 (SEB)" → "Ján 3:16-18": the reference without its trailing
/// translation code in brackets (the title row names the translations once).
pub fn without_translation_code(reference: &str) -> &str {
    let trimmed = reference.trim_end();
    match (trimmed.ends_with(')'), trimmed.rfind(" (")) {
        (true, Some(at)) if at > 0 => trimmed[..at].trim_end(),
        _ => trimmed,
    }
}

/// The title of the loaded passage: "Ján 3:16-18 · SEB + KJV". Every live
/// slide carries the whole passage's reference, so the first one names it.
/// The secondary code is named only when the slides carry its text (an
/// unavailable NLT loads main-only, #826). `None` for an empty grid.
pub fn passage_title(
    slides: &[BibleSlideDto],
    main: Option<&str>,
    secondary: Option<&str>,
) -> Option<String> {
    let first = slides.first()?;
    let reference = without_translation_code(&first.bible_main_reference);
    let main = first
        .metadata
        .as_ref()
        .and_then(|m| m.bible.as_ref())
        .and_then(|b| b.translation_code.as_deref())
        .or(main)
        .filter(|code| !code.is_empty());
    let has_secondary_text = slides
        .iter()
        .any(|slide| !slide.bible_translation.trim().is_empty());
    let secondary = secondary
        .filter(|code| !code.is_empty() && has_secondary_text)
        .map(translation_short_code);
    let codes = match (main.map(translation_short_code), secondary) {
        (Some(main), Some(secondary)) => format!("{main} + {secondary}"),
        (Some(main), None) => main,
        (None, Some(secondary)) => secondary,
        (None, None) => String::new(),
    };
    Some(match (reference.is_empty(), codes.is_empty()) {
        (false, false) => format!("{reference} · {codes}"),
        (false, true) => reference.to_string(),
        (true, _) => codes,
    })
}

/// The verses one card holds: "Ján 3:16", "Ján 3:16-17", "Ján 3:16, 18".
/// From the slide's metadata; without it, its reference minus the code.
pub fn slide_verse_label(slide: &BibleSlideDto) -> String {
    let meta = slide.metadata.as_ref().and_then(|m| m.bible.as_ref());
    meta.and_then(verse_label_from_meta)
        .unwrap_or_else(|| without_translation_code(&slide.bible_main_reference).to_string())
}

fn verse_label_from_meta(meta: &BibleSlideMetaBible) -> Option<String> {
    let book = meta
        .book
        .as_deref()
        .filter(|book| !book.trim().is_empty())?;
    let chapter = meta.chapter?;
    let ranges: Vec<(u16, u16)> = if meta.verses.is_empty() {
        let start = meta.verse_start?;
        vec![(start, meta.verse_end.unwrap_or(start))]
    } else {
        meta.verses.iter().map(|v| (v.start, v.end)).collect()
    };
    let verses = merge_runs(&ranges)
        .iter()
        .map(|&(start, end)| {
            if end > start {
                format!("{start}-{end}")
            } else {
                start.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("{book} {chapter}:{verses}"))
}

/// The composer lists one range per verse ((16,16), (17,17)): runs that
/// touch become one range, so the caption reads "16-17", never "16, 17".
fn merge_runs(ranges: &[(u16, u16)]) -> Vec<(u16, u16)> {
    let mut merged: Vec<(u16, u16)> = Vec::new();
    for &(start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1.saturating_add(1) && start >= last.0 => {
                last.1 = last.1.max(end);
            }
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// The card's caption: "1 · Ján 3:16" (the slide's number, from 1).
/// A slide with no reference at all (an empty prepared one) shows its number.
pub fn slide_caption(index: usize, slide: &BibleSlideDto) -> String {
    let label = slide_verse_label(slide);
    if label.is_empty() {
        (index + 1).to_string()
    } else {
        format!("{} · {label}", index + 1)
    }
}

/// Whether the wall shows this card: the same main text and reference as the
/// output the server holds (a card's ZOBRAZIŤ sends exactly these two).
pub fn is_on_wall(output: Option<&BibleSlideOutput>, main_text: &str, main_ref: &str) -> bool {
    output.is_some_and(|output| {
        !main_text.trim().is_empty()
            && output.main_text.trim() == main_text.trim()
            && output.main_reference.trim() == main_ref.trim()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::bible::{BibleSlideMetadataDto, BibleSlideVerseRefDto};

    fn slide(
        main_ref: &str,
        translation: &str,
        meta: Option<BibleSlideMetaBible>,
    ) -> BibleSlideDto {
        BibleSlideDto {
            id: "s".into(),
            order: 0,
            bible_main: "16. Veď Boh tak miloval svet".into(),
            bible_translation: translation.into(),
            metadata: meta.map(|bible| BibleSlideMetadataDto { bible: Some(bible) }),
            bible_main_reference: main_ref.into(),
            bible_translation_reference: String::new(),
        }
    }

    fn meta(code: &str, verses: &[(u16, u16)]) -> BibleSlideMetaBible {
        BibleSlideMetaBible {
            translation_code: Some(code.into()),
            book: Some("Ján".into()),
            book_code: Some("JHN".into()),
            book_number: Some(43),
            chapter: Some(3),
            verse_start: None,
            verse_end: None,
            verses: verses
                .iter()
                .map(|&(start, end)| BibleSlideVerseRefDto { start, end })
                .collect(),
            main_reference_label: None,
            translation_reference_label: None,
        }
    }

    #[test]
    fn strips_only_a_trailing_code() {
        assert_eq!(without_translation_code("Ján 3:16-18 (SEB)"), "Ján 3:16-18");
        assert_eq!(without_translation_code("Ján 3:16-18"), "Ján 3:16-18");
        assert_eq!(without_translation_code("(SEB)"), "(SEB)");
        assert_eq!(without_translation_code(""), "");
    }

    #[test]
    fn title_names_the_passage_and_both_translations() {
        let slides = [slide(
            "Ján 3:16-18 (SEB)",
            "16. For God",
            Some(meta("slk-seb", &[(16, 17)])),
        )];
        assert_eq!(
            passage_title(&slides, Some("slk-seb"), Some("eng-kjv")).as_deref(),
            Some("Ján 3:16-18 · SEB + KJV")
        );
    }

    #[test]
    fn title_leaves_out_a_secondary_without_text() {
        let slides = [slide(
            "Ján 3:16-18 (SEB)",
            "",
            Some(meta("slk-seb", &[(16, 18)])),
        )];
        assert_eq!(
            passage_title(&slides, Some("slk-seb"), Some("eng-nlt")).as_deref(),
            Some("Ján 3:16-18 · SEB")
        );
        assert_eq!(
            passage_title(&slides, Some("slk-seb"), None).as_deref(),
            Some("Ján 3:16-18 · SEB")
        );
    }

    #[test]
    fn title_prefers_the_slides_translation_and_is_none_when_empty() {
        let slides = [slide(
            "Ján 3:16 (ROH)",
            "",
            Some(meta("slk-roh", &[(16, 16)])),
        )];
        assert_eq!(
            passage_title(&slides, Some("slk-seb"), None).as_deref(),
            Some("Ján 3:16 · ROH")
        );
        assert_eq!(passage_title(&[], Some("slk-seb"), None), None);
        let bare = [slide("Ján 3:16", "", None)];
        assert_eq!(
            passage_title(&bare, None, None).as_deref(),
            Some("Ján 3:16")
        );
    }

    #[test]
    fn caption_numbers_the_card_and_names_its_verses() {
        let one = slide("Ján 3:16-18 (SEB)", "", Some(meta("slk-seb", &[(16, 16)])));
        assert_eq!(slide_caption(0, &one), "1 · Ján 3:16");
        let range = slide("Ján 3:16-18 (SEB)", "", Some(meta("slk-seb", &[(16, 17)])));
        assert_eq!(slide_caption(1, &range), "2 · Ján 3:16-17");
        let list = slide(
            "Ján 3:16-18 (SEB)",
            "",
            Some(meta("slk-seb", &[(16, 16), (18, 18)])),
        );
        assert_eq!(slide_verse_label(&list), "Ján 3:16, 18");
    }

    #[test]
    fn caption_joins_the_composers_one_verse_ranges() {
        // The live composer lists every verse on its own.
        let per_verse = slide(
            "Ján 3:16-18 (SEB)",
            "",
            Some(meta("slk-seb", &[(16, 16), (17, 17), (18, 18)])),
        );
        assert_eq!(slide_verse_label(&per_verse), "Ján 3:16-18");
        let gap = slide(
            "Ján 3:16-18 (SEB)",
            "",
            Some(meta("slk-seb", &[(16, 16), (17, 17), (19, 20)])),
        );
        assert_eq!(slide_verse_label(&gap), "Ján 3:16-17, 19-20");
    }

    #[test]
    fn caption_of_a_slide_without_any_reference_is_its_number() {
        assert_eq!(slide_caption(1, &slide("", "", None)), "2");
    }

    #[test]
    fn caption_falls_back_to_flat_verses_then_the_reference() {
        let mut flat = meta("slk-seb", &[]);
        flat.verse_start = Some(5);
        flat.verse_end = Some(7);
        assert_eq!(slide_verse_label(&slide("x", "", Some(flat))), "Ján 3:5-7");
        assert_eq!(
            slide_verse_label(&slide("Rim 8:28 (SEB)", "", None)),
            "Rim 8:28"
        );
        let mut bookless = meta("slk-seb", &[(1, 1)]);
        bookless.book = None;
        assert_eq!(
            slide_verse_label(&slide("Rim 8:28 (SEB)", "", Some(bookless))),
            "Rim 8:28"
        );
    }

    #[test]
    fn on_wall_needs_the_same_text_and_reference() {
        let output = BibleSlideOutput::new("16. Veď", "Ján 3:16 (SEB)", "", "", chrono::Utc::now());
        assert!(is_on_wall(Some(&output), "16. Veď", "Ján 3:16 (SEB)"));
        assert!(!is_on_wall(Some(&output), "16. Veď", "Ján 3:16 (ROH)"));
        assert!(!is_on_wall(Some(&output), "17. Lebo", "Ján 3:16 (SEB)"));
        assert!(!is_on_wall(None, "16. Veď", "Ján 3:16 (SEB)"));
        let blank = BibleSlideOutput::new("", "", "", "", chrono::Utc::now());
        assert!(!is_on_wall(Some(&blank), "", ""));
    }
}
