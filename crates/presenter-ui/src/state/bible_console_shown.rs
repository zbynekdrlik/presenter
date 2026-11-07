//! "Táto bohoslužba" (#850 B2d): a verse shown today as the console's preview
//! target. Pure — host-tested.

use presenter_core::bible::resolver::{segment_label, BookRef, Segment};

use super::bible_console::PreviewTarget;
use crate::api::bible::ShownVerse;

pub fn shown_target(verse: &ShownVerse) -> PreviewTarget {
    let segment = Segment::verses(verse.chapter, verse.verse_start, verse.verse_end);
    PreviewTarget {
        book: BookRef {
            code: verse.book_code.clone(),
            number: verse.book_number,
            name: verse.book.clone(),
        },
        label: segment_label(&verse.book, &segment),
        segment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shown_verse_previews_its_own_book_and_verses() {
        let target = shown_target(&ShownVerse {
            label: "Hebrejom 11:1-3".to_string(),
            book: "Hebrejom".to_string(),
            book_code: "HEB".to_string(),
            book_number: 58,
            chapter: 11,
            verse_start: 1,
            verse_end: 3,
            translation_code: "slk-seb".to_string(),
        });
        assert_eq!(target.book.code, "HEB");
        assert_eq!(target.book.number, 58);
        assert_eq!(target.segment, Segment::verses(11, 1, 3));
        assert_eq!(target.label, "Hebrejom 11:1-3");
    }
}
