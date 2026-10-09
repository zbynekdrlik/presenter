//! #825: bound a typed chapter / verse by the selected book's counts and say
//! why when it was over — the decision behind the Bible page's range hint.

use super::bible::clamp_selection;

/// A typed chapter or verse bounded to the selected book, plus the note to
/// show when it was past the range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedInput {
    pub value: u16,
    pub hint: Option<String>,
}

/// The typed chapter bounded to the book's chapters (`clamp_selection`), with
/// "Kniha má len N kapitol" when it was past the last one. Without a selected
/// book (`chapter_count == 0`) only the lower bound 1 applies.
pub fn bound_chapter(typed: u16, chapter_count: u16, verse_counts: &[u16]) -> BoundedInput {
    if chapter_count == 0 || verse_counts.is_empty() {
        return BoundedInput {
            value: typed.max(1),
            hint: None,
        };
    }
    let value = clamp_selection(chapter_count, verse_counts, typed, 1, None).chapter;
    let hint = (typed > chapter_count).then(|| {
        format!(
            "Kniha má len {}",
            counted(chapter_count, "kapitolu", "kapitoly", "kapitol")
        )
    });
    BoundedInput { value, hint }
}

/// A typed verse (start OR end) bounded to `chapter`'s verses
/// (`clamp_selection`), with "Kapitola má len M veršov" when it was past the
/// last one. The END is bounded like the start and never collapsed to "all"
/// (`clamp_selection` turns `end <= start` into `None`, but the #702 mirror
/// sets `end = start` for a single verse). Without a selected book only the
/// lower bound 1 applies.
pub fn bound_verse(
    typed: u16,
    chapter: u16,
    chapter_count: u16,
    verse_counts: &[u16],
) -> BoundedInput {
    if chapter_count == 0 || verse_counts.is_empty() {
        return BoundedInput {
            value: typed.max(1),
            hint: None,
        };
    }
    let clamped = clamp_selection(chapter_count, verse_counts, chapter, typed, None);
    let max_verse = verse_counts
        .get(usize::from(clamped.chapter.max(1)) - 1)
        .copied()
        .unwrap_or(1)
        .max(1);
    let hint = (typed > max_verse).then(|| {
        format!(
            "Kapitola má len {}",
            counted(max_verse, "verš", "verše", "veršov")
        )
    });
    BoundedInput {
        value: clamped.verse_start,
        hint,
    }
}

/// `n` with the Slovak noun form it takes: 1 → `one`, 2–4 → `few`, else `many`.
fn counted(n: u16, one: &str, few: &str, many: &str) -> String {
    let noun = match n {
        1 => one,
        2..=4 => few,
        _ => many,
    };
    format!("{n} {noun}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chapter_within_the_book_is_kept_without_a_hint() {
        assert_eq!(
            bound_chapter(3, 5, &[10; 5]),
            BoundedInput {
                value: 3,
                hint: None
            }
        );
    }

    #[test]
    fn a_chapter_past_the_last_one_is_clamped_with_a_hint() {
        assert_eq!(
            bound_chapter(60, 50, &[20; 50]),
            BoundedInput {
                value: 50,
                hint: Some("Kniha má len 50 kapitol".to_string())
            }
        );
    }

    #[test]
    fn chapter_zero_becomes_the_first_chapter_without_a_hint() {
        assert_eq!(
            bound_chapter(0, 5, &[10; 5]),
            BoundedInput {
                value: 1,
                hint: None
            }
        );
    }

    #[test]
    fn without_a_selected_book_the_typed_chapter_stays() {
        assert_eq!(
            bound_chapter(60, 0, &[]),
            BoundedInput {
                value: 60,
                hint: None
            }
        );
    }

    #[test]
    fn a_verse_past_the_end_of_the_chapter_is_clamped_with_a_hint() {
        assert_eq!(
            bound_verse(99, 2, 3, &[10, 31, 5]),
            BoundedInput {
                value: 31,
                hint: Some("Kapitola má len 31 veršov".to_string())
            }
        );
    }

    #[test]
    fn a_verse_within_the_chapter_is_kept_without_a_hint() {
        assert_eq!(
            bound_verse(7, 1, 3, &[10, 31, 5]),
            BoundedInput {
                value: 7,
                hint: None
            }
        );
        assert_eq!(bound_verse(0, 1, 3, &[10, 31, 5]).value, 1);
    }

    #[test]
    fn without_a_selected_book_the_typed_verse_stays() {
        assert_eq!(
            bound_verse(99, 1, 0, &[]),
            BoundedInput {
                value: 99,
                hint: None
            }
        );
    }

    fn hint(field: RangeField, text: &str) -> RangeHint {
        RangeHint {
            field,
            text: text.to_string(),
        }
    }

    #[test]
    fn a_clamp_shows_its_note_for_that_box() {
        let bounded = bound_chapter(60, 5, &[10; 5]);
        assert_eq!(
            next_hint(None, RangeField::Chapter, 1, &bounded),
            Some(hint(RangeField::Chapter, "Kniha má len 5 kapitol"))
        );
    }

    #[test]
    fn the_same_box_recommitting_its_clamped_value_keeps_the_note() {
        // Enter writes "5" into the box; moving the focus then fires the
        // browser's `change` with that "5" — the note must survive it.
        let shown = hint(RangeField::Chapter, "Kniha má len 5 kapitol");
        let recommit = bound_chapter(5, 5, &[10; 5]);
        assert_eq!(
            next_hint(Some(&shown), RangeField::Chapter, 5, &recommit),
            Some(shown.clone())
        );
    }

    #[test]
    fn a_new_valid_value_or_another_box_clears_the_note() {
        let shown = hint(RangeField::VerseStart, "Kapitola má len 21 veršov");
        let other_value = bound_verse(2, 5, 5, &[10, 10, 10, 10, 21]);
        assert_eq!(
            next_hint(Some(&shown), RangeField::VerseStart, 21, &other_value),
            None
        );
        let other_box = bound_chapter(5, 5, &[10; 5]);
        assert_eq!(
            next_hint(Some(&shown), RangeField::Chapter, 5, &other_box),
            None
        );
        assert_eq!(next_hint(None, RangeField::VerseEnd, 3, &other_value), None);
    }

    #[test]
    fn the_hints_use_slovak_plural_forms() {
        assert_eq!(
            bound_chapter(9, 1, &[25]).hint.as_deref(),
            Some("Kniha má len 1 kapitolu")
        );
        assert_eq!(
            bound_chapter(9, 3, &[25; 3]).hint.as_deref(),
            Some("Kniha má len 3 kapitoly")
        );
        assert_eq!(
            bound_verse(9, 1, 1, &[1]).hint.as_deref(),
            Some("Kapitola má len 1 verš")
        );
        assert_eq!(
            bound_verse(9, 1, 1, &[4]).hint.as_deref(),
            Some("Kapitola má len 4 verše")
        );
        assert_eq!(
            bound_verse(30, 1, 1, &[21]).hint.as_deref(),
            Some("Kapitola má len 21 veršov")
        );
    }
}
