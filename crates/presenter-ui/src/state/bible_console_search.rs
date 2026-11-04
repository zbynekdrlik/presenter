//! The console's text search (#850 spec §5.11): words the resolver cannot
//! read as a reference are looked up in the main translation's text. Asked
//! only when no reference is being typed, so `zid 11 1` never searches.
//! Pure — host-tested.

use presenter_core::bible::resolver::{
    parse, segment_label, BookRef, MatchKind, Resolution, Segment,
};

use super::bible_console::PreviewTarget;
use crate::api::bible::BibleSearchHit;

/// Fewer letters than this never search (`11:1`, `ž`).
const MIN_LETTERS: usize = 3;

/// Whether `input` should search the text: words alone (no book, no numbers),
/// or at least two words or no digit at all, with no book name that matches
/// exactly or as a prefix (`jan krstitel`, `mária`, which only LOOKS like Marek;
/// never `1 kor 13` or a mistyped `zidd 11 1`).
pub fn wants_text_search(input: &str, resolution: &Resolution) -> bool {
    if input.chars().filter(|c| c.is_alphabetic()).count() < MIN_LETTERS {
        return false;
    }
    if let Resolution::NoBook { segments, .. } = resolution {
        return segments.is_empty();
    }
    let words = input
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .count();
    (words >= 2 || !input.chars().any(char::is_numeric))
        && !parse(input)
            .candidates
            .first()
            .is_some_and(|best| best.kind <= MatchKind::Prefix)
}

/// The passage a search hit names; `None` for a hit without a book code.
pub fn hit_target(hit: &BibleSearchHit) -> Option<PreviewTarget> {
    let reference = &hit.reference;
    let segment = Segment::verses(
        reference.chapter,
        reference.verse_start,
        reference.verse_end,
    );
    Some(PreviewTarget {
        book: BookRef {
            code: reference.book_code.clone()?,
            number: reference.book_number.unwrap_or(0),
            name: reference.book.clone(),
        },
        label: segment_label(&reference.book, &segment),
        segment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::bible::BibleBookDto;
    use crate::state::bible_console::versification;
    use presenter_core::bible::resolver::lookup;

    fn wants(input: &str) -> bool {
        let books: Vec<BibleBookDto> = serde_json::from_str(include_str!(
            "../../../presenter-core/src/bible/resolver/testdata/structure-slk-seb.json"
        ))
        .expect("the SEB structure fixture");
        wants_text_search(input, &lookup(input, &versification(&books)))
    }

    #[test]
    fn words_search_and_references_never_do() {
        assert!(wants("viera je zárukou"));
        assert!(wants("jan krstitel"));
        assert!(wants("mária"));
        assert!(wants("judáš"));
        assert!(!wants("zid 11 1"));
        assert!(!wants("zidd 11 1"));
        assert!(!wants("1 kor 13"));
        assert!(!wants("jan"));
        assert!(!wants("11:1"));
        assert!(!wants("ž"));
    }

    #[test]
    fn a_hit_previews_its_own_passage() {
        let hit: BibleSearchHit = serde_json::from_value(serde_json::json!({
            "reference": { "book": "Hebrejom", "bookCode": "HEB", "bookNumber": 58,
                           "chapter": 11, "verseStart": 1, "verseEnd": 1 },
            "translation": { "code": "slk-seb", "name": "SEB", "language": "sk" },
            "text": "Viera je zárukou toho, v čo dúfame"
        }))
        .expect("a search hit");
        let target = hit_target(&hit).expect("a target");
        assert_eq!(target.book.code, "HEB");
        assert_eq!(target.segment, Segment::verses(11, 1, 1));
        assert_eq!(target.label, "Hebrejom 11:1");
    }
}
