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
