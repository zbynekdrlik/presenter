//! #851 D2a over the real KJV (public domain, spec §14): the markup spells
//! `content` for every verse, and `content` itself is pinned by a digest.

use super::*;
use presenter_core::bible::{SegmentKind, VerseMarkup};
use presenter_core::BiblePassage;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const BUNDLED_KJV: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/bibles/kjv.usfm.zip"
);
const KJV_CONTENT_SHA256: &str = include_str!("testdata/kjv-content.sha256");

fn kjv() -> (Vec<BiblePassage>, Vec<Option<VerseMarkup>>) {
    let bytes = std::fs::read(BUNDLED_KJV).expect("bundled KJV archive");
    let (batch, _) =
        parsers::build_ingestion_batch(&bytes, &king_james_spec()).expect("parse the KJV");
    let (_, passages, markup) = batch.into_parts_with_markup();
    (passages, markup)
}

fn markup_of<'a>(
    verses: &'a (Vec<BiblePassage>, Vec<Option<VerseMarkup>>),
    book: &str,
    chapter: u16,
    verse: u16,
) -> &'a VerseMarkup {
    let at = verses
        .0
        .iter()
        .position(|p| {
            p.reference.book == book
                && p.reference.chapter == chapter
                && p.reference.verse_start == verse
        })
        .expect("verse");
    verses.1[at].as_ref().expect("markup")
}

#[test]
fn the_kjv_content_is_pinned_by_its_digest() {
    let (passages, _) = kjv();
    let mut hash = Sha256::new();
    for passage in &passages {
        hash.update(passage.text.as_bytes());
        hash.update(b"\n");
    }
    let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(digest, KJV_CONTENT_SHA256.trim());
}

#[test]
fn kjv_markup_spells_its_verses() {
    let (passages, markup) = kjv();
    assert_eq!(passages.len(), 31_102);
    let mut tags = 0_u32;
    let mut pairs = HashSet::new();
    let mut bare = Vec::new();
    for (index, entry) in markup.iter().enumerate() {
        let Some(entry) = entry else {
            let reference = &passages[index].reference;
            bare.push((
                reference.book.as_str(),
                reference.chapter,
                reference.verse_start,
            ));
            continue;
        };
        assert_eq!(entry.plain_text(), passages[index].text);
        for (strongs, count) in entry.strongs_counts() {
            tags += u32::from(count);
            pairs.insert((index, strongs));
        }
    }
    assert_eq!((tags, pairs.len()), (348_884, 310_076));
    // The module tags no word of these two verses: nothing to tell about them.
    assert_eq!(bare, [("Mark", 9, 43), ("Luke", 6, 41)]);
}

#[test]
fn headings_open_the_next_verse_and_a_subscription_ends_the_last() {
    let verses = kjv();
    let kinds = |m: &VerseMarkup| m.0.iter().map(|s| s.kind).collect::<Vec<_>>();
    let psalm = markup_of(&verses, "Psalms", 3, 1);
    assert_eq!(psalm.0[0].kind, SegmentKind::Heading);
    assert!(psalm.0[0].text.starts_with("A Psalm of David"));
    assert_eq!(markup_of(&verses, "Psalms", 119, 9).0[0].text, "ב BETH.");
    assert!(!kinds(markup_of(&verses, "Psalms", 119, 8)).contains(&SegmentKind::Heading));

    let last = markup_of(&verses, "Hebrews", 13, 25);
    let tail = last.0.last().expect("segment");
    assert_eq!(tail.kind, SegmentKind::Heading);
    assert_eq!(tail.text, "Written to the Hebrews from Italy by Timothy.");
}

#[test]
fn kjv_details_land_in_their_segments() {
    let verses = kjv();
    let faith = markup_of(&verses, "Hebrews", 11, 1);
    assert_eq!(faith.0[0].text, "Now");
    assert_eq!(faith.0[0].strongs, ["G1161"]);
    let river = markup_of(&verses, "Genesis", 2, 14);
    assert!(river.0.iter().any(|s| s.italic && s.text == "is"));
    assert!(river.0.iter().any(|s| s.kind == SegmentKind::Note));
}
