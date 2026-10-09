//! #825: bound a typed chapter / verse by the selected book's counts and say
//! why when it was over — the decision behind the Bible page's range hint.

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
