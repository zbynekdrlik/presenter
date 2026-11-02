//! The compare view's pure half (#851 spec §6.5): which translations it shows
//! and which verse numbers its rows have. Host-tested with `cargo test --lib`.

use crate::state::study::StudyVerse;

/// A translation lacks this verse (numbering differs, Mal 4:5 vs the SEB).
pub const MISSING_VERSE: &str = "— (v tomto preklade tento verš nie je; číslovanie sa môže líšiť)";

/// How the compare view lays out on this screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareLayout {
    /// One verse, one translation under the other (a phone).
    Stacked,
    /// The whole chapter, this many translations side by side.
    Columns(usize),
}

impl CompareLayout {
    pub fn for_screen(phone: bool, wide: bool) -> Self {
        match (phone, wide) {
            (true, _) => Self::Stacked,
            (false, true) => Self::Columns(3),
            (false, false) => Self::Columns(2),
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Stacked => "stacked",
            Self::Columns(_) => "columns",
        }
    }
}

/// The whole list of the view: the URL's installed ones, else the reader's
/// translation and the favourites. Writing a change back uses this list, so a
/// narrow screen never shortens what a wider one shows.
pub fn listed_translations(
    named: &[String],
    reader: Option<&str>,
    favourites: &[String],
    installed: &[String],
) -> Vec<String> {
    let usable = |codes: &[String]| {
        codes
            .iter()
            .filter(|code| installed.contains(code))
            .cloned()
            .collect::<Vec<_>>()
    };
    let listed = usable(named);
    if !listed.is_empty() {
        return listed;
    }
    let mut chosen: Vec<String> = reader.map(ToString::to_string).into_iter().collect();
    chosen.extend(
        favourites
            .iter()
            .filter(|code| Some(code.as_str()) != reader)
            .cloned(),
    );
    usable(&chosen)
}

/// What this layout has room for.
pub fn fitted(mut listed: Vec<String>, layout: CompareLayout) -> Vec<String> {
    if let CompareLayout::Columns(columns) = layout {
        listed.truncate(columns);
    }
    listed
}

/// `shown` with this column switched to `code`. A translation already in
/// another column trades places, so no column shows the same text twice.
pub fn switch_column(shown: &[String], column: usize, code: &str) -> Vec<String> {
    let mut next = shown.to_vec();
    let Some(held) = next.get(column).cloned() else {
        return next;
    };
    if let Some(other) = next.iter().position(|chosen| chosen == code) {
        next[other] = held;
    }
    next[column] = code.to_string();
    next
}

/// Every verse number any column has, in order: the rows of the grid.
pub fn verse_numbers<'a>(columns: impl Iterator<Item = &'a [StudyVerse]>) -> Vec<u16> {
    let mut numbers: Vec<u16> = columns.flatten().map(|verse| verse.verse).collect();
    numbers.sort_unstable();
    numbers.dedup();
    numbers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    fn verses(numbers: &[u16]) -> Vec<StudyVerse> {
        numbers
            .iter()
            .map(|verse| StudyVerse {
                verse: *verse,
                text: String::new(),
            })
            .collect()
    }

    #[test]
    fn a_phone_stacks_and_wider_screens_get_two_or_three_columns() {
        assert_eq!(
            CompareLayout::for_screen(true, false),
            CompareLayout::Stacked
        );
        assert_eq!(
            CompareLayout::for_screen(true, true),
            CompareLayout::Stacked
        );
        assert_eq!(
            CompareLayout::for_screen(false, false),
            CompareLayout::Columns(2)
        );
        assert_eq!(
            CompareLayout::for_screen(false, true),
            CompareLayout::Columns(3)
        );
    }

    #[test]
    fn no_url_list_is_the_reader_first_and_the_installed_favourites() {
        let listed = listed_translations(
            &[],
            Some("c"),
            &codes(&["a", "gone", "c", "b"]),
            &codes(&["a", "b", "c"]),
        );
        assert_eq!(listed, codes(&["c", "a", "b"]));
    }

    #[test]
    fn a_reader_translation_outside_the_favourites_still_shows() {
        let listed = listed_translations(&[], Some("mil"), &codes(&["a"]), &codes(&["a", "mil"]));
        assert_eq!(listed, codes(&["mil", "a"]));
    }

    #[test]
    fn the_url_list_wins_and_loses_what_is_not_installed() {
        let listed = listed_translations(
            &codes(&["c", "gone"]),
            Some("a"),
            &codes(&["a"]),
            &codes(&["a", "c"]),
        );
        assert_eq!(listed, codes(&["c"]));
    }

    #[test]
    fn a_url_naming_only_unknown_translations_shows_the_favourites() {
        let listed = listed_translations(&codes(&["gone"]), None, &codes(&["a"]), &codes(&["a"]));
        assert_eq!(listed, codes(&["a"]));
    }

    #[test]
    fn columns_cut_the_list_to_what_fits_and_a_phone_keeps_it() {
        let all = codes(&["a", "b", "c", "d"]);
        assert_eq!(
            fitted(all.clone(), CompareLayout::Columns(3)),
            codes(&["a", "b", "c"])
        );
        assert_eq!(fitted(all.clone(), CompareLayout::Stacked), all);
    }

    #[test]
    fn switching_a_column_replaces_it() {
        assert_eq!(
            switch_column(&codes(&["a", "b"]), 1, "c"),
            codes(&["a", "c"])
        );
    }

    #[test]
    fn switching_to_a_translation_in_another_column_trades_places() {
        assert_eq!(
            switch_column(&codes(&["a", "b", "c"]), 0, "c"),
            codes(&["c", "b", "a"])
        );
    }

    #[test]
    fn a_column_that_is_not_there_changes_nothing() {
        assert_eq!(switch_column(&codes(&["a"]), 3, "c"), codes(&["a"]));
    }

    #[test]
    fn rows_are_every_verse_of_any_column_in_order() {
        let one = verses(&[1, 2, 4]);
        let two = verses(&[1, 3]);
        let numbers = verse_numbers([one.as_slice(), two.as_slice()].into_iter());
        assert_eq!(numbers, [1, 2, 3, 4]);
    }
}
