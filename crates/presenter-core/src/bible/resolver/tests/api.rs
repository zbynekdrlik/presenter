//! The public API end to end (#850 spec §5.1, §5.7): `lookup`, the name and
//! translation lookups, stored names at runtime, partial main translations.

use super::{seb, sevp};
use crate::bible::resolver::{
    lookup, parse, resolve_book_name, resolve_translation_code, segment_label, Passage, Resolution,
    Segment, Versification, MISSING_BOOK_HINT,
};

fn codes(passages: &[Passage]) -> Vec<&str> {
    passages.iter().map(|p| p.book.code.as_str()).collect()
}

#[test]
fn main_translation_decides_the_versification_and_the_names() {
    let Resolution::OutOfRange { hint, .. } = lookup("mal 4:5", &seb()) else {
        panic!("Malachi has 3 chapters in SEB");
    };
    assert_eq!(hint, "Kniha má len 3 kapitoly");
    let Resolution::Ambiguous {
        default,
        alternatives,
    } = lookup("J 3,16", &sevp())
    else {
        panic!("J 3,16 is ambiguous (Jer/Job 3:16 exist too)");
    };
    assert_eq!(default.book.name, "Evanjelium podľa Jána");
    assert_eq!(codes(&alternatives)[..4], ["JER", "JOB", "JAS", "JOS"]);
    let Resolution::Passage {
        passage,
        alternatives,
    } = lookup("1K 13,4", &sevp())
    else {
        panic!("1K 13,4 is 1 Korinťanom");
    };
    assert_eq!(passage.book.name, "Prvýlist apoštola Pavla Korintským");
    assert_eq!(codes(&alternatives), ["1KI", "1CH"]);
}

#[test]
fn the_structure_filter_picks_the_book_that_has_the_verse() {
    for (input, code) in [
        ("pt 5:7", "1PE"),
        ("ab 2:4", "HAB"),
        ("fil 4:13", "PHP"),
        ("jud 2:5", "JDG"),
    ] {
        let Resolution::Passage { passage, .. } = lookup(input, &seb()) else {
            panic!("{input} must be one passage");
        };
        assert_eq!(passage.book.code, code, "{input}");
    }
}

#[test]
fn out_of_range_fix_labels() {
    for (input, label) in [
        ("zid 14", "Hebrejom 13"),
        ("zid 11:41", "Hebrejom 11:40"),
        ("jud 30", "Júda 1:25"),
    ] {
        let Resolution::OutOfRange {
            book,
            fix: Some(fix),
            ..
        } = lookup(input, &seb())
        else {
            panic!("{input} must be out of range with a fix");
        };
        assert_eq!(
            segment_label(&book.name, &fix.segments[0]),
            label,
            "{input}"
        );
    }
}

#[test]
fn an_ordinal_alone_lists_the_books_named_with_it() {
    let structure = seb();
    let first = [
        "GEN", "1SA", "1KI", "1CH", "1CO", "1TH", "1TI", "1PE", "1JN",
    ];
    let second = [
        "EXO", "2SA", "2KI", "2CH", "2CO", "2TH", "2TI", "2PE", "2JN",
    ];
    let cases: [(&str, u8, &[&str]); 13] = [
        ("1", 1, &first),
        ("1.", 1, &first),
        ("I.", 1, &first),
        ("prvý", 1, &first),
        ("prvá", 1, &first),
        ("2", 2, &second),
        ("III.", 3, &["LEV", "1KI", "3JN"]),
        ("IV.", 4, &["NUM", "2KI"]),
        ("4.", 4, &["NUM", "2KI"]),
        ("štvrtá", 4, &["NUM", "2KI"]),
        ("V.", 5, &["DEU"]),
        ("piata", 5, &["DEU"]),
        ("5", 5, &["DEU"]),
    ];
    for (input, want_ordinal, want_books) in cases {
        let Resolution::Incomplete { ordinal, books } = lookup(input, &structure) else {
            panic!("{input} must be incomplete");
        };
        let codes: Vec<&str> = books.iter().map(|b| b.code.as_str()).collect();
        assert_eq!(
            (ordinal, codes.as_slice()),
            (want_ordinal, want_books),
            "{input}"
        );
    }
}

#[test]
fn no_book_inputs() {
    let structure = seb();
    let no_book = |segments: Vec<Segment>, hint: Option<&str>| Resolution::NoBook {
        segments,
        hint: hint.map(str::to_string),
    };
    assert_eq!(
        lookup("11:1", &structure),
        no_book(vec![Segment::verses(11, 1, 1)], Some("Napíš aj knihu"))
    );
    assert_eq!(
        lookup("viera je zárukou", &structure),
        no_book(Vec::new(), None)
    );
    assert_eq!(lookup("", &structure), no_book(Vec::new(), None));
}

#[test]
fn a_book_missing_from_a_partial_main_translation_says_so() {
    let gospels = Versification::new(
        seb()
            .books()
            .iter()
            .filter(|b| ["MAT", "MRK", "LUK", "JHN"].contains(&b.code.as_str()))
            .cloned()
            .collect(),
    );
    assert_eq!(
        lookup("zid 11 1", &gospels),
        Resolution::NoBook {
            segments: vec![Segment::verses(11, 1, 1)],
            hint: Some(MISSING_BOOK_HINT.to_string()),
        }
    );
    assert!(matches!(
        lookup("jan 3 16", &gospels),
        Resolution::Passage { .. }
    ));
}

#[test]
fn stored_names_of_the_main_translation_match_at_runtime() {
    let mut books = seb().books().to_vec();
    if let Some(heb) = books.iter_mut().find(|b| b.code == "HEB") {
        heb.name = "Hebräer".to_string();
    }
    let Resolution::Passage { passage, .. } = lookup("hebräer 11 1", &Versification::new(books))
    else {
        panic!("a stored name must resolve without the table");
    };
    assert_eq!(passage.book.code, "HEB");
    assert_eq!(passage.matched_form, "Hebräer");
}

#[test]
fn parse_keeps_the_typed_parts() {
    let parsed = parse("zid 11:1 viera");
    assert_eq!(
        (parsed.book_text.as_str(), parsed.key.as_str()),
        ("zid", "zid")
    );
    assert_eq!(parsed.leftover, "viera");
    assert_eq!(parsed.candidates[0].code, "HEB");
    assert_eq!(parse("PŠ 2,1").candidates[0].code, "SNG");
}

#[test]
fn book_name_and_translation_code_lookups() {
    let code = |name: &str| resolve_book_name(name).map(|c| c.code);
    assert_eq!(code("Hebr").as_deref(), Some("HEB"));
    assert_eq!(code("Židom").as_deref(), Some("HEB"));
    assert_eq!(code("John").as_deref(), Some("JHN"));
    assert_eq!(code(""), None);
    let installed = ["eng-kjv", "slk-seb", "slk-roh", "slk-sevp", "eng-nlt"];
    let translation = |input: &str| resolve_translation_code(input, &installed);
    assert_eq!(translation("KJV").as_deref(), Some("eng-kjv"));
    assert_eq!(translation("seb").as_deref(), Some("slk-seb"));
    assert_eq!(translation("SLK-SEVP").as_deref(), Some("slk-sevp"));
    assert_eq!(translation(" nlt ").as_deref(), Some("eng-nlt"));
    assert_eq!(translation("xyz"), None);
    assert_eq!(translation(""), None);
    assert_eq!(
        resolve_translation_code("seb", &["slk-seb", "ces-seb"]),
        None
    );
}
