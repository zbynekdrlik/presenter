//! #851 D1: the import defects that reached the walls (spec §5.1, T1). The SEB
//! and ROH modules are private after #859, so their markup shapes are rebuilt
//! here as synthetic MySword archives with made-up sentences; the KJV is public
//! domain, so its own lines and the bundled `data/bibles/kjv.usfm.zip` are used.

use super::*;
use presenter_core::BiblePassage;
use regex::Regex;
use rusqlite::Connection;
use std::io::{Cursor, Write};
use tempfile::NamedTempFile;
use zip::write::SimpleFileOptions;

const BUNDLED_KJV: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/bibles/kjv.usfm.zip"
);

fn zip_with(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buffer));
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("zip entry");
        writer.write_all(bytes).expect("zip write");
        writer.finish().expect("zip finish");
    }
    buffer
}

/// A MySword module zip whose `Bible` table holds `rows`
/// (book, chapter, verse, scripture).
fn mysword_archive(rows: &[(i64, i64, i64, &str)]) -> Vec<u8> {
    let temp = NamedTempFile::new().expect("temp file");
    let conn = Connection::open(temp.path()).expect("sqlite");
    conn.execute(
        "CREATE TABLE Bible (Book INTEGER, Chapter INTEGER, Verse INTEGER, Scripture TEXT)",
        [],
    )
    .expect("create table");
    for (book, chapter, verse, scripture) in rows {
        conn.execute(
            "INSERT INTO Bible (Book, Chapter, Verse, Scripture) VALUES (?1, ?2, ?3, ?4)",
            (book, chapter, verse, scripture),
        )
        .expect("insert row");
    }
    drop(conn);
    let bytes = std::fs::read(temp.path()).expect("read sqlite");
    zip_with("MODULE.bbl.mybible", &bytes)
}

/// The verse texts the SEB spec imports from `rows`.
fn mysword_texts(rows: &[(i64, i64, i64, &str)]) -> Vec<String> {
    let (batch, _) =
        parsers::build_ingestion_batch(&mysword_archive(rows), &slovak_ekumenicky_spec())
            .expect("parse MySword archive");
    batch.passages().iter().map(|p| p.text.clone()).collect()
}

/// The passages the KJV spec imports from one USFM document.
fn usfm_passages(usfm: &str) -> Vec<BiblePassage> {
    let archive = zip_with("BOOK.usfm", usfm.as_bytes());
    let (batch, _) =
        parsers::build_ingestion_batch(&archive, &king_james_spec()).expect("parse USFM");
    batch.passages().to_vec()
}

fn text_of(passages: &[BiblePassage], book: &str, chapter: u16, verse: u16) -> String {
    passages
        .iter()
        .find(|p| {
            p.reference.book == book
                && p.reference.chapter == chapter
                && p.reference.verse_start == verse
        })
        .map(|p| p.text.clone())
        .unwrap_or_default()
}

#[test]
fn a_seb_note_goes_with_the_space_before_it() {
    let texts = mysword_texts(&[
        (
            1,
            2,
            14,
            "Tretia rieka je Tigris <FI>[ Pozn.: iný názov.]<Fi>. Tečie na východ <FI>[ Hebr. slovo.]<Fi>.",
        ),
        (1, 2, 15, "Pôjdeš <FI>[ Pozn.: iný tvar.]<Fi>ďalej."),
    ]);
    assert_eq!(
        texts,
        ["Tretia rieka je Tigris. Tečie na východ.", "Pôjdeš ďalej."]
    );
}

#[test]
fn bare_brackets_belong_to_the_translation_and_stay() {
    let texts = mysword_texts(&[
        (
            41,
            16,
            20,
            "Oni odišli <FI>[ V niektorých rkp. chýba.]<Fi>. [Kratší záver textu. Amen <FI>[ Pozn. k záveru.]<Fi>.]",
        ),
        (52, 3, 13, "Nech vás posilní vo svätosti. [Amen.]"),
    ]);
    assert_eq!(
        texts,
        [
            "Oni odišli. [Kratší záver textu. Amen.]",
            "Nech vás posilní vo svätosti. [Amen.]",
        ]
    );
}

#[test]
fn a_roh_heading_is_dropped_from_its_verse() {
    let texts = mysword_texts(&[
        (
            1,
            1,
            1,
            "<TS>Nadpis kapitoly<Ts>  Na začiatku bol text verša. ",
        ),
        (1, 1, 2, "<ts>Iný nadpis<ts> Druhý verš."),
    ]);
    assert_eq!(texts, ["Na začiatku bol text verša.", "Druhý verš."]);
}

#[test]
fn roh_bracketed_words_stay() {
    let texts = mysword_texts(&[(
        40,
        17,
        21,
        " Taký druh nevyženiete inak ako modlitbou [a pôstom]. ",
    )]);
    assert_eq!(
        texts,
        ["Taký druh nevyženiete inak ako modlitbou [a pôstom]."]
    );
}

#[test]
fn whitespace_before_punctuation_goes_in_mysword_modules() {
    let texts = mysword_texts(&[
        (
            40,
            21,
            42,
            "Povedal: <FR> Kameň sa stal základom<Fr> <FR> .<Fr> <FR> Pán to urobil,<Fr>",
        ),
        (
            58,
            4,
            7,
            "Určuje iný <FR> dnes <Fr> , keď hovorí ! Áno ; teda : tak ?",
        ),
    ]);
    assert_eq!(
        texts,
        [
            "Povedal: Kameň sa stal základom. Pán to urobil,",
            "Určuje iný dnes, keď hovorí! Áno; teda: tak?",
        ]
    );
}

#[test]
fn a_kjv_footnote_goes_with_its_text() {
    let passages = usfm_passages(
        r#"\id GEN Genesis
\c 2
\p
\v 14 And the \w name|strong="H8034"\w* of the \w third|strong="H7992"\w* \w river|strong="H5104"\w* \add is\add* \w Hiddekel|strong="H2313"\w*: that \add is\add* it which goeth \w toward|strong="H1980"\w* the \w east|strong="H6926"\w* of \w Assyria|strong="H0804"\w*. And the \w fourth|strong="H7243"\w* \w river|strong="H5104"\w* \add is\add* \w Euphrates|strong="H6578"\w*.\f + \fr 2.14 \ft toward…: or, eastward to Assyria\f*
"#,
    );
    assert_eq!(
        text_of(&passages, "Genesis", 2, 14),
        "And the name of the third river is Hiddekel: that is it which goeth toward the east of Assyria. And the fourth river is Euphrates."
    );
}

#[test]
fn a_leading_pilcrow_goes_even_behind_a_marker() {
    let passages = usfm_passages(
        r#"\id JHN John
\c 3
\v 16 \wj ¶ \+w For|strong="G1063"\+w* \+w God|strong="G2316"\+w* \+w so|strong="G3779"\+w* \+w loved|strong="G0025"\+w* \+w the world|strong="G2889"\+w*\wj*
\v 17 ¶ For God sent not his Son into the world.
"#,
    );
    assert_eq!(
        text_of(&passages, "John", 3, 16),
        "For God so loved the world"
    );
    assert_eq!(
        text_of(&passages, "John", 3, 17),
        "For God sent not his Son into the world."
    );
}

#[test]
fn heading_lines_never_join_a_verse() {
    let passages = usfm_passages(
        r#"\id PSA Psalms
\c 119
\s1 \tl  א ALEPH.\tl*
\q1
\v 8 I will keep thy statutes: O forsake me not utterly.
\s1 \tl  ב BETH.\tl*
\q1
\v 9 Wherewithal shall a young man cleanse his way?
\id HEB Hebrews
\c 13
\v 25 Grace be with you all. Amen.
\s1 Written to the Hebrews from Italy by Timothy.
"#,
    );
    assert_eq!(
        text_of(&passages, "Psalms", 119, 8),
        "I will keep thy statutes: O forsake me not utterly."
    );
    assert_eq!(
        text_of(&passages, "Psalms", 119, 9),
        "Wherewithal shall a young man cleanse his way?"
    );
    assert_eq!(
        text_of(&passages, "Hebrews", 13, 25),
        "Grace be with you all. Amen."
    );
}

#[test]
fn the_bundled_kjv_has_no_import_defects() {
    let bytes = std::fs::read(BUNDLED_KJV).expect("bundled KJV archive");
    let (batch, _) =
        parsers::build_ingestion_batch(&bytes, &king_james_spec()).expect("parse the KJV");
    let passages = batch.passages();
    let footnote = Regex::new(r"\+ \d+\.\d+").expect("footnote pattern");

    assert_eq!(passages.len(), 31_102);
    let pilcrows = passages.iter().filter(|p| p.text.contains('¶')).count();
    let footnotes = passages
        .iter()
        .filter(|p| footnote.is_match(&p.text))
        .count();
    assert_eq!((pilcrows, footnotes), (0, 0));
    // Survival: no rule may wipe verse text. The pre-D1 parser kept 4,377,552
    // characters here (footnotes, `¶` and glued headings included); the cleaned
    // corpus keeps 4,107,378 (94 %), so a floor just under it fails the moment a
    // rule over-reaches.
    let empty = passages.iter().filter(|p| p.text.trim().is_empty()).count();
    let total: usize = passages.iter().map(|p| p.text.chars().count()).sum();
    assert_eq!(empty, 0);
    assert!(
        total >= 4_100_000,
        "the cleaned KJV kept only {total} chars"
    );
    assert!(text_of(passages, "Genesis", 2, 14).ends_with("the fourth river is Euphrates."));
    assert!(text_of(passages, "John", 3, 16).starts_with("For God so loved the world"));
    assert_eq!(
        text_of(passages, "Hebrews", 13, 25),
        "Grace be with you all. Amen."
    );
    assert_eq!(
        text_of(passages, "Psalms", 119, 8),
        "I will keep thy statutes: O forsake me not utterly."
    );
}
