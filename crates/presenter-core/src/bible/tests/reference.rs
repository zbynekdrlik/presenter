use super::super::reference::{BibleReference, BibleReferenceError};

#[test]
fn rejects_invalid_reference_ranges() {
    assert_eq!(
        BibleReference::new("John", 1, 5, 3).unwrap_err(),
        BibleReferenceError::InvalidVerseRange
    );
}

#[test]
fn formats_reference() {
    let reference = BibleReference::new("John", 3, 16, 16).unwrap();
    assert_eq!(reference.to_human_readable(), "John 3:16");
    let range = BibleReference::new("Psalm", 23, 1, 3).unwrap();
    assert_eq!(range.to_human_readable(), "Psalm 23:1-3");
}

#[test]
fn rejects_zero_chapter() {
    assert_eq!(
        BibleReference::new("Genesis", 0, 1, 5).unwrap_err(),
        BibleReferenceError::InvalidChapter
    );
}

#[test]
fn rejects_zero_verse_start() {
    assert_eq!(
        BibleReference::new("John", 3, 0, 5).unwrap_err(),
        BibleReferenceError::InvalidVerseRange
    );
}

#[test]
fn rejects_zero_verse_end() {
    assert_eq!(
        BibleReference::new("John", 3, 1, 0).unwrap_err(),
        BibleReferenceError::InvalidVerseRange
    );
}

#[test]
fn accepts_single_verse() {
    let reference = BibleReference::new("John", 3, 16, 16).unwrap();
    assert_eq!(reference.chapter, 3);
    assert_eq!(reference.verse_start, 16);
    assert_eq!(reference.verse_end, 16);
}

#[test]
fn new_with_code_stores_code_and_number() {
    let reference =
        BibleReference::new_with_code("Genesis", "GEN", 1, 1, 1, 3).unwrap();
    assert_eq!(reference.book_code.as_deref(), Some("GEN"));
    assert_eq!(reference.book_number, Some(1));
}
