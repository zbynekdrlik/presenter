//! The chapter/verse grammar (#850 spec §5.4).

use crate::bible::resolver::grammar::split_input;
use crate::bible::resolver::RawSpec;

fn verses(chapter: u16, ranges: &[(u16, u16)]) -> RawSpec {
    RawSpec::Verses {
        chapter,
        ranges: ranges.to_vec(),
    }
}

#[test]
fn every_separator_and_range_form() {
    let cross = RawSpec::CrossChapter {
        chapter: 1,
        verse_start: 1,
        end_chapter: 2,
        verse_end: 3,
    };
    let cases = [
        ("heb 11:1", verses(11, &[(1, 1)])),
        ("heb 11,1", verses(11, &[(1, 1)])),
        ("heb 11.1", verses(11, &[(1, 1)])),
        ("heb 11 1", verses(11, &[(1, 1)])),
        ("heb11:1", verses(11, &[(1, 1)])),
        ("heb 11 : 1", verses(11, &[(1, 1)])),
        ("heb 11:1-3", verses(11, &[(1, 3)])),
        ("heb 11:1–3", verses(11, &[(1, 3)])),
        ("heb 11:1—3", verses(11, &[(1, 3)])),
        ("heb 11,1 až 3", verses(11, &[(1, 3)])),
        ("heb 11,1 az 3", verses(11, &[(1, 3)])),
        ("mt 5:3.5.7", verses(5, &[(3, 3), (5, 5), (7, 7)])),
        ("jn 3:16,18", verses(3, &[(16, 16), (18, 18)])),
        ("mt 5,3.5", verses(5, &[(3, 3), (5, 5)])),
        ("ef 2 8-9", verses(2, &[(8, 9)])),
        ("1 kor 13 4-7", verses(13, &[(4, 7)])),
        ("heb 11:5-3", verses(11, &[(5, 5)])),
        ("ž 23-24", RawSpec::ChapterRange { start: 23, end: 24 }),
        ("ž 24-23", RawSpec::Chapter(24)),
        ("heb 11", RawSpec::Chapter(11)),
        ("heb 0", RawSpec::Chapter(1)),
        ("gn 1:1-2:3", cross.clone()),
        ("gn 1,1-2,3", cross),
    ];
    for (input, want) in cases {
        assert_eq!(split_input(input).spec, Some(want), "{input}");
    }
}

#[test]
fn the_book_text_ends_at_the_first_digit_after_an_ordinal() {
    for (input, book) in [
        ("1kor13:4", "1kor"),
        ("1. Korinťanom 13,4", "1. Korinťanom"),
        ("I. Jánov 4,8", "I. Jánov"),
        ("Rim.8,25", "Rim."),
        ("4M 6,24", "4M"),
        ("viera je zárukou", "viera je zárukou"),
    ] {
        assert_eq!(split_input(input).book_text, book, "{input}");
    }
    assert_eq!(split_input("1").bare_ordinal, Some(1));
    assert_eq!(split_input("2.").bare_ordinal, Some(2));
    assert_eq!(split_input("4.").bare_ordinal, Some(4));
    assert_eq!(split_input("5").bare_ordinal, Some(5));
    assert_eq!(split_input("6").bare_ordinal, None);
    assert_eq!(split_input("6").spec, Some(RawSpec::Chapter(6)));
    assert_eq!(split_input("11").bare_ordinal, None);
    assert_eq!(split_input("11").spec, Some(RawSpec::Chapter(11)));
}

#[test]
fn trailing_separators_are_ignored_and_unused_text_is_leftover() {
    for (input, want, leftover) in [
        ("zid 11:", RawSpec::Chapter(11), ""),
        ("zid 11:1-", verses(11, &[(1, 1)]), ""),
        ("zid 11:1 až", verses(11, &[(1, 1)]), ""),
        ("zid 11 až", RawSpec::Chapter(11), ""),
        ("zid 11:1 viera", verses(11, &[(1, 1)]), "viera"),
        ("zid 11:1 až viera", verses(11, &[(1, 1)]), "viera"),
        ("zid 11,1:3", verses(11, &[(1, 1)]), "3"),
        ("zid 99999999", RawSpec::Chapter(u16::MAX), ""),
    ] {
        let split = split_input(input);
        assert_eq!(split.spec, Some(want), "{input}");
        assert_eq!(split.leftover, leftover, "{input}");
    }
}

#[test]
fn an_nfd_az_is_a_range_like_the_nfc_one() {
    let nfd = split_input("zid 11,1 az\u{30C} 3");
    let nfc = split_input("zid 11,1 až 3");
    assert_eq!(nfd.spec, Some(verses(11, &[(1, 3)])));
    assert_eq!(nfd.leftover, "");
    assert_eq!(
        (nfd.spec, nfd.leftover, nfd.book_text),
        (nfc.spec, nfc.leftover, nfc.book_text)
    );
}

#[test]
fn a_long_paste_is_bounded() {
    let split = split_input(&"1,".repeat(5000));
    assert_eq!(split.spec, Some(verses(1, &[(1, 1)])));
    assert!(!split.leftover.is_empty());
}

#[test]
fn unbound_segments_keep_what_was_typed() {
    use crate::bible::resolver::Segment;
    assert_eq!(
        verses(11, &[(1, 1)]).unbound_segments(),
        [Segment::verses(11, 1, 1)]
    );
    let cross = RawSpec::CrossChapter {
        chapter: 1,
        verse_start: 5,
        end_chapter: 2,
        verse_end: 3,
    };
    assert_eq!(
        cross.unbound_segments(),
        [
            Segment {
                chapter: 1,
                verse_start: Some(5),
                verse_end: None
            },
            Segment::verses(2, 1, 3)
        ]
    );
}
