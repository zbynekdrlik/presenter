use super::{compose_bible_items_into_slides, compose_bible_slides, BibleItem, ComposedBibleSlide};
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};
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
        compose_bible_slides(&translation, None, &passages, &HashMap::new(), 320, 7, 7).unwrap();

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
        compose_bible_slides(&translation, None, &passages, &HashMap::new(), 320, 14, 15).unwrap();

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
fn compose_items_second_verse_exactly_at_limit_stays_on_one_slide() {
    // Pins the overflow boundary at `prospective > limit` (ASCII = 1 byte/char).
    // verse 1 = "1. " + 6 a = 9 bytes; verse 2 = "2. " + 7 b = 10 bytes.
    // prospective = 9 (existing) + 1 (lines.len()) + 10 (new line) = 20 == limit
    // → does NOT overflow → both verses share ONE slide. This kills the
    // `> -> >=` and the `+ -> -`/`+ -> *` mutants on would_overflow's arithmetic.
    let items = vec![verse(1, "aaaaaa"), verse(2, "bbbbbbb")];
    let slides = compose_bible_items_into_slides(&items, 20);
    assert_eq!(slides.len(), 1, "prospective == limit must NOT overflow");
    assert_eq!(slides[0].main, "1. aaaaaa\n2. bbbbbbb");
}

#[test]
fn compose_items_second_verse_one_over_limit_splits_to_two_slides() {
    // One byte more than the at-limit case above: verse 2 = "2. " + 8 b = 11
    // bytes → prospective = 9 + 1 + 11 = 21 > 20 → overflow → two slides.
    // Together with the at-limit test this pins the exact boundary.
    let items = vec![verse(1, "aaaaaa"), verse(2, "bbbbbbbb")];
    let slides = compose_bible_items_into_slides(&items, 20);
    assert_eq!(slides.len(), 2, "prospective == limit+1 must overflow");
    assert_eq!(slides[0].main, "1. aaaaaa");
    assert_eq!(slides[1].main, "2. bbbbbbbb");
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
fn compose_items_single_verse_longer_than_limit_kept_whole_on_own_slide() {
    // Limit 20; verse line is much longer. A lone oversized verse is kept
    // WHOLE on its own slide — it is never split mid-verse, and the
    // validator now ACCEPTS it (display shrink / autofit handles oversize),
    // so no oversized-slide error is emitted downstream (issue #394).
    let items = vec![verse(1, "Na počiatku bolo Slovo a Slovo bolo u Boha.")];
    let slides = compose_bible_items_into_slides(&items, 20);
    assert_eq!(slides.len(), 1);
    // The full verse text is intact on the single slide — nothing dropped.
    assert_eq!(
        slides[0].main,
        "1. Na počiatku bolo Slovo a Slovo bolo u Boha."
    );
    assert!(slides[0].main.len() > 20);
    assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
}

#[test]
fn compose_items_same_verse_number_split_is_merged_whole_not_split_mid_verse() {
    // Issue #394: the LLM's oversized-single-verse recovery path may emit
    // ONE logical verse as several consecutive `Verse` items that share the
    // SAME verse number (it broke the verse text apart). The composer must
    // merge those back into one slide rather than flushing mid-verse into
    // two slides. Low limit (30) would, with the buggy packer, force a
    // mid-verse flush after the first fragment — that is the bug.
    let items = vec![
        verse(1, "Na počiatku bolo Slovo"),
        verse(1, "a Slovo bolo u Boha a Boh bol to Slovo."),
    ];
    let slides = compose_bible_items_into_slides(&items, 30);
    // Whole verse 1 lands on ONE slide — never split across two.
    assert_eq!(
        slides.len(),
        1,
        "a single verse must never be split mid-verse across slides"
    );
    assert_eq!(
        slides[0].main,
        "1. Na počiatku bolo Slovo a Slovo bolo u Boha a Boh bol to Slovo."
    );
    assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
}

#[test]
fn compose_items_same_number_merge_then_next_verse_overflows_to_own_slide() {
    // Two fragments of verse 1 merge into one (oversized) slide; verse 2 is a
    // separate verse and overflows to its own slide. The whole-verse rule for
    // verse 1 must not bleed verse 2's text onto the same line.
    let items = vec![
        verse(1, "Na počiatku bolo Slovo"),
        verse(1, "a Slovo bolo u Boha a Boh bol to Slovo."),
        verse(2, "Ono bolo na počiatku u Boha."),
    ];
    let slides = compose_bible_items_into_slides(&items, 30);
    assert_eq!(slides.len(), 2);
    assert_eq!(
        slides[0].main,
        "1. Na počiatku bolo Slovo a Slovo bolo u Boha a Boh bol to Slovo."
    );
    assert_eq!(slides[1].main, "2. Ono bolo na počiatku u Boha.");
    // Both slides share the full group range (verses 1-2).
    assert_eq!(slides[0].main_reference, "Ján 1:1-2 (SEB)");
    assert_eq!(slides[1].main_reference, "Ján 1:1-2 (SEB)");
}

#[test]
fn compose_items_same_number_fragment_after_earlier_verse_moves_growing_verse_to_own_slide() {
    // Issue #394 edge (from code review): verse 1 and verse 2 first pack onto
    // one slide; then a verse-2 continuation fragment arrives that would push
    // the slide over the limit. The growing verse 2 must move to its OWN slide
    // kept WHOLE — never an oversized two-verse slide (which the validator would
    // reject). So: slide 0 = verse 1 alone, slide 1 = whole verse 2.
    let items = vec![
        verse(1, "short one"),
        verse(2, "bbb"),
        verse(2, "this is a long continuation fragment that makes it big"),
    ];
    let slides = compose_bible_items_into_slides(&items, 30);
    assert_eq!(slides.len(), 2);
    assert_eq!(slides[0].main, "1. short one");
    assert_eq!(
        slides[1].main,
        "2. bbb this is a long continuation fragment that makes it big"
    );
    // Each slide is a single verse-prefixed line — the validator accepts both
    // (slide 1 is a lone oversized whole verse).
    assert_eq!(slides[0].main.lines().count(), 1);
    assert_eq!(slides[1].main.lines().count(), 1);
    assert_eq!(slides[0].main_reference, "Ján 1:1-2 (SEB)");
    assert_eq!(slides[1].main_reference, "Ján 1:1-2 (SEB)");
}

#[test]
fn compose_items_same_number_fragment_that_still_fits_stays_with_earlier_verse() {
    // When the merged verse still fits under the limit, the same-number fragment
    // merges in place and stays on the shared slide (no premature split).
    let items = vec![verse(1, "alpha"), verse(2, "beta"), verse(2, "gamma")];
    let slides = compose_bible_items_into_slides(&items, 320);
    assert_eq!(slides.len(), 1);
    assert_eq!(slides[0].main, "1. alpha\n2. beta gamma");
    assert_eq!(slides[0].main_reference, "Ján 1:1-2 (SEB)");
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

fn verse_full(number: u32, text: &str, book: &str, chapter: u32, translation: &str) -> BibleItem {
    BibleItem::Verse {
        number,
        text: text.to_string(),
        book: book.to_string(),
        chapter,
        translation: translation.to_string(),
    }
}

#[test]
fn compose_uses_full_passage_range_across_split_slides() {
    // Numeri 13:17-20 forced to split into 2 slides via low char limit.
    // Both slides must show the FULL range "Numeri 13:17-20 (SEB)".
    let items = vec![
        verse_full(
            17,
            "Verse seventeen text long enough to fill",
            "Numeri",
            13,
            "SEB",
        ),
        verse_full(18, "Verse eighteen text", "Numeri", 13, "SEB"),
        verse_full(
            19,
            "Verse nineteen text long enough to fill",
            "Numeri",
            13,
            "SEB",
        ),
        verse_full(20, "Verse twenty text", "Numeri", 13, "SEB"),
    ];
    // Char limit chosen so that 2 verses pack into one slide and the
    // next two pack into the second slide.
    let slides = compose_bible_items_into_slides(&items, 80);
    assert!(
        slides.len() >= 2,
        "expected at least 2 slides, got {}",
        slides.len()
    );
    for (i, slide) in slides.iter().enumerate() {
        assert_eq!(
            slide.main_reference, "Numeri 13:17-20 (SEB)",
            "slide {} must show full range",
            i
        );
    }
}

#[test]
fn compose_handles_emphasis_between_verses_with_full_range() {
    let items = vec![
        verse_full(17, "Verse seventeen", "Numeri", 13, "SEB"),
        emphasis("DÔLEŽITÉ SLOVO"),
        verse_full(18, "Verse eighteen", "Numeri", 13, "SEB"),
        verse_full(19, "Verse nineteen", "Numeri", 13, "SEB"),
        verse_full(20, "Verse twenty", "Numeri", 13, "SEB"),
    ];
    let slides = compose_bible_items_into_slides(&items, 320);
    // Find the emphasis slide (empty reference, main = "DÔLEŽITÉ SLOVO")
    let emphasis_slide = slides
        .iter()
        .find(|s| s.main == "DÔLEŽITÉ SLOVO")
        .expect("emphasis slide present");
    assert_eq!(
        emphasis_slide.main_reference, "",
        "emphasis slide has empty reference"
    );
    // Every verse slide (the ones whose main contains "Verse ") must
    // show the full passage range.
    let verse_slides: Vec<&ComposedBibleSlide> = slides
        .iter()
        .filter(|s| s.main.contains("Verse "))
        .collect();
    assert!(verse_slides.len() >= 2, "expected at least 2 verse slides");
    for (i, slide) in verse_slides.iter().enumerate() {
        assert_eq!(
            slide.main_reference, "Numeri 13:17-20 (SEB)",
            "verse slide {} must show full range across emphasis interruption",
            i
        );
    }
}

#[test]
fn compose_two_distinct_passages_get_independent_ranges() {
    let items = vec![
        verse_full(1, "In the beginning was the Word.", "Ján", 1, "SEB"),
        verse_full(2, "He was in the beginning with God.", "Ján", 1, "SEB"),
        verse_full(3, "Blessed are the poor in spirit.", "Mat", 5, "SEB"),
    ];
    let slides = compose_bible_items_into_slides(&items, 320);
    let jan_slides: Vec<&ComposedBibleSlide> = slides
        .iter()
        .filter(|s| s.main_reference.starts_with("Ján"))
        .collect();
    let mat_slides: Vec<&ComposedBibleSlide> = slides
        .iter()
        .filter(|s| s.main_reference.starts_with("Mat"))
        .collect();
    assert!(!jan_slides.is_empty(), "Ján slides present");
    assert!(!mat_slides.is_empty(), "Mat slides present");
    for slide in &jan_slides {
        assert_eq!(slide.main_reference, "Ján 1:1-2 (SEB)");
    }
    for slide in &mat_slides {
        assert_eq!(slide.main_reference, "Mat 5:3 (SEB)");
    }
}

#[test]
fn compose_non_contiguous_verses_render_as_comma_list() {
    // Pastor cited only verses 1, 3, 5 — non-contiguous. Reference
    // must show the explicit list, not a misleading 1-5 range.
    let items = vec![
        verse_full(1, "First cited verse content here", "Numeri", 13, "SEB"),
        verse_full(3, "Third cited verse content here", "Numeri", 13, "SEB"),
        verse_full(5, "Fifth cited verse content here", "Numeri", 13, "SEB"),
    ];
    let slides = compose_bible_items_into_slides(&items, 60);
    assert!(!slides.is_empty(), "at least one slide");
    for (i, slide) in slides.iter().enumerate() {
        assert_eq!(
            slide.main_reference, "Numeri 13:1, 3, 5 (SEB)",
            "slide {} must show comma-list of cited verses",
            i
        );
    }
}

#[test]
fn compose_mixed_gap_renders_as_comma_list() {
    // Mixed gap (skip verse 3): 1, 2, 4, 5. Not perfectly contiguous,
    // so the reference is a flat comma-list — no mixed "1-2, 4-5" syntax.
    let items = vec![
        verse_full(1, "Verse one", "Numeri", 13, "SEB"),
        verse_full(2, "Verse two", "Numeri", 13, "SEB"),
        verse_full(4, "Verse four", "Numeri", 13, "SEB"),
        verse_full(5, "Verse five", "Numeri", 13, "SEB"),
    ];
    let slides = compose_bible_items_into_slides(&items, 320);
    assert!(!slides.is_empty(), "at least one slide");
    for (i, slide) in slides.iter().enumerate() {
        assert_eq!(
            slide.main_reference, "Numeri 13:1, 2, 4, 5 (SEB)",
            "slide {} must show flat comma-list",
            i
        );
    }
}
