//! The main translation's structure (#850 spec §5.6): binding, out-of-range
//! hints and fixes, stepping, building from chapter summaries.

use super::{kjv, seb};
use crate::bible::resolver::versification::{bind, out_of_range};
use crate::bible::resolver::{BookStructure, RawSpec, Segment, Step, Versification};
use crate::bible::BibleBookChapterSummary;

fn book(structure: &Versification, code: &str) -> BookStructure {
    structure.book(code).cloned().expect("book in the fixture")
}

fn verses(chapter: u16, ranges: &[(u16, u16)]) -> RawSpec {
    RawSpec::Verses {
        chapter,
        ranges: ranges.to_vec(),
    }
}

#[test]
fn versification_differs_per_translation() {
    let spec = verses(4, &[(5, 5)]);
    assert_eq!(
        bind(&spec, &book(&kjv(), "MAL")),
        Some(vec![Segment::verses(4, 5, 5)])
    );
    assert_eq!(
        bind(&spec, &book(&seb(), "MAL")),
        None,
        "SEB Malachi has 3 chapters"
    );
    assert_eq!(
        out_of_range(&spec, &book(&seb(), "MAL")).hint,
        "Kniha má len 3 kapitoly"
    );
}

#[test]
fn single_chapter_books_read_a_lone_number_as_a_verse() {
    let jude = book(&seb(), "JUD");
    assert_eq!(
        bind(&RawSpec::Chapter(5), &jude),
        Some(vec![Segment::verses(1, 5, 5)])
    );
    assert_eq!(
        bind(&RawSpec::Chapter(1), &jude),
        Some(vec![Segment::whole(1)])
    );
    let range = RawSpec::ChapterRange { start: 1, end: 5 };
    assert_eq!(bind(&range, &jude), Some(vec![Segment::verses(1, 1, 5)]));
    assert_eq!(
        bind(&verses(2, &[(5, 5)]), &jude),
        None,
        "Jude has no chapter 2"
    );
}

#[test]
fn lists_chapter_ranges_and_cross_chapter_ranges_become_segments() {
    let s = seb();
    assert_eq!(
        bind(&verses(5, &[(3, 3), (5, 5), (7, 7)]), &book(&s, "MAT")),
        Some(vec![
            Segment::verses(5, 3, 3),
            Segment::verses(5, 5, 5),
            Segment::verses(5, 7, 7)
        ])
    );
    let psalms = RawSpec::ChapterRange { start: 23, end: 24 };
    assert_eq!(
        bind(&psalms, &book(&s, "PSA")),
        Some(vec![Segment::whole(23), Segment::whole(24)])
    );
    let cross = RawSpec::CrossChapter {
        chapter: 1,
        verse_start: 1,
        end_chapter: 3,
        verse_end: 2,
    };
    assert_eq!(
        bind(&cross, &book(&s, "GEN")),
        Some(vec![
            Segment::verses(1, 1, 31),
            Segment::whole(2),
            Segment::verses(3, 1, 2)
        ])
    );
}

#[test]
fn every_segment_is_checked_not_only_the_last() {
    let s = seb();
    let list = verses(5, &[(3, 3), (99, 99), (7, 7)]);
    assert_eq!(bind(&list, &book(&s, "MAT")), None);
    assert_eq!(
        out_of_range(&list, &book(&s, "MAT")).hint,
        "Kapitola má len 48 veršov"
    );
    let beyond = RawSpec::CrossChapter {
        chapter: 50,
        verse_start: 1,
        end_chapter: 51,
        verse_end: 2,
    };
    assert_eq!(bind(&beyond, &book(&s, "GEN")), None);
    assert_eq!(
        out_of_range(&beyond, &book(&s, "GEN")).hint,
        "Kniha má len 50 kapitol"
    );
    let psalms = RawSpec::ChapterRange {
        start: 150,
        end: 151,
    };
    assert_eq!(bind(&psalms, &book(&s, "PSA")), None);
    assert_eq!(
        out_of_range(&psalms, &book(&s, "PSA")).requested,
        Segment::whole(151)
    );
}

#[test]
fn out_of_range_offers_the_last_chapter_or_verse_as_fix() {
    let s = seb();
    let heb = book(&s, "HEB");
    let failure = out_of_range(&RawSpec::Chapter(14), &heb);
    assert_eq!(failure.requested, Segment::whole(14));
    assert_eq!(failure.hint, "Kniha má len 13 kapitol");
    assert_eq!(failure.fix, [Segment::whole(13)]);
    let failure = out_of_range(&verses(11, &[(41, 41)]), &heb);
    assert_eq!(failure.requested, Segment::verses(11, 41, 41));
    assert_eq!(failure.hint, "Kapitola má len 40 veršov");
    assert_eq!(failure.fix, [Segment::verses(11, 40, 40)]);
    let failure = out_of_range(&verses(11, &[(39, 45)]), &heb);
    assert_eq!(failure.fix, [Segment::verses(11, 39, 40)]);
    let failure = out_of_range(&RawSpec::Chapter(30), &book(&s, "JUD"));
    assert_eq!(failure.hint, "Kapitola má len 25 veršov");
    assert_eq!(failure.fix, [Segment::verses(1, 25, 25)]);
}

#[test]
fn step_crosses_chapter_and_book_ends() {
    let s = seb();
    let at = |code: &str, chapter, verse| Some((code.to_string(), chapter, verse));
    assert_eq!(s.step("HEB", 11, 1, Step::Next), at("HEB", 11, 2));
    assert_eq!(s.step("HEB", 11, 40, Step::Next), at("HEB", 12, 1));
    assert_eq!(s.step("HEB", 13, 25, Step::Next), at("JAS", 1, 1));
    assert_eq!(s.step("JAS", 1, 1, Step::Prev), at("HEB", 13, 25));
    assert_eq!(s.step("HEB", 12, 1, Step::Prev), at("HEB", 11, 40));
    assert_eq!(s.step("GEN", 1, 1, Step::Prev), None);
    assert_eq!(s.step("REV", 22, 21, Step::Next), None);
    assert_eq!(s.step("XXX", 1, 1, Step::Next), None);
}

#[test]
fn chapter_summaries_build_a_canonical_structure() {
    let row = |book: &str, code: Option<&str>, chapter, verse_count| BibleBookChapterSummary {
        book: book.to_string(),
        book_code: code.map(str::to_string),
        book_number: None,
        chapter,
        verse_count,
    };
    let rows = [
        row("Židom", Some("HEB"), 2, 18),
        row("Genezis", Some("GEN"), 1, 31),
        row("Židom", Some("HEB"), 1, 14),
        row("Bez kódu", None, 1, 5),
        row("Židom", Some("HEB"), 0, 9),
    ];
    let s = Versification::from_chapter_summaries(&rows);
    let codes: Vec<&str> = s.books().iter().map(|b| b.code.as_str()).collect();
    assert_eq!(
        codes,
        ["GEN", "HEB"],
        "canonical order; no code / chapter 0 skipped"
    );
    let heb = book(&s, "HEB");
    assert_eq!((heb.number, heb.verse_counts.clone()), (58, vec![14, 18]));
    assert_eq!(heb.verse_count(3), None);
}
