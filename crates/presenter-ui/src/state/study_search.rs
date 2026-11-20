//! The study search's pure half (#983): which translations the results view
//! offers, which ones a hit offers, and the text of a hit's verses in another
//! translation.

use crate::state::bible_translations::favourite_row;
use crate::state::study::StudyVerse;

/// The translation chips above the hits: the reader's favourites that can be
/// searched (a remote translation has no text on this box), the open one in
/// the last place when it is none of them (`favourite_row`).
pub fn search_translation_row(
    favourites: Vec<(usize, String)>,
    open: Option<&str>,
    cap: usize,
    searchable: impl Fn(&str) -> bool,
) -> Vec<(usize, String)> {
    let searchable_favourites = favourites
        .into_iter()
        .filter(|(_, code)| searchable(code))
        .collect();
    favourite_row(searchable_favourites, open, cap)
}

/// A hit's own translation chips: the favourites, its own translation first
/// when it is none of them, so a tap on it always goes back.
pub fn hit_translations(favourites: &[(usize, String)], original: &str) -> Vec<String> {
    let mut codes: Vec<String> = favourites.iter().map(|(_, code)| code.clone()).collect();
    if !codes.iter().any(|code| code == original) {
        codes.insert(0, original.to_string());
    }
    codes
}

/// The text of `first..=last` in a chapter, one verse after the other. `None`
/// when the chapter has none of them (that translation lacks the verse).
pub fn passage_text(verses: &[StudyVerse], first: u16, last: u16) -> Option<String> {
    let last = last.max(first);
    let texts: Vec<&str> = verses
        .iter()
        .filter(|verse| (first..=last).contains(&verse.verse))
        .map(|verse| verse.text.trim())
        .filter(|text| !text.is_empty())
        .collect();
    (!texts.is_empty()).then(|| texts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(codes: &[&str]) -> Vec<(usize, String)> {
        codes
            .iter()
            .enumerate()
            .map(|(slot, code)| (slot, code.to_string()))
            .collect()
    }

    fn verse(number: u16, text: &str) -> StudyVerse {
        StudyVerse {
            verse: number,
            text: text.to_string(),
            markup: None,
        }
    }

    #[test]
    fn the_search_row_leaves_out_what_cannot_be_searched() {
        let row = search_translation_row(
            slots(&["slk-seb", "slk-roh", "eng-kjv", "eng-nlt"]),
            Some("slk-seb"),
            5,
            |code| code != "eng-nlt",
        );
        assert_eq!(row, slots(&["slk-seb", "slk-roh", "eng-kjv"]));
    }

    #[test]
    fn the_open_translation_takes_the_last_chip_of_the_search_row() {
        let row = search_translation_row(
            slots(&["slk-seb", "slk-roh", "eng-kjv"]),
            Some("ces-b21"),
            2,
            |_| true,
        );
        assert_eq!(
            row,
            vec![(0, "slk-seb".to_string()), (1, "ces-b21".to_string())]
        );
    }

    #[test]
    fn a_hit_always_offers_its_own_translation() {
        let favourites = slots(&["slk-seb", "eng-kjv"]);
        assert_eq!(
            hit_translations(&favourites, "eng-kjv"),
            vec!["slk-seb".to_string(), "eng-kjv".to_string()]
        );
        assert_eq!(
            hit_translations(&favourites, "slk-roh"),
            vec![
                "slk-roh".to_string(),
                "slk-seb".to_string(),
                "eng-kjv".to_string()
            ]
        );
    }

    #[test]
    fn a_passage_reads_its_verses_in_order() {
        let verses = vec![
            verse(1, "Na počiatku"),
            verse(2, " Zem bola "),
            verse(3, "Boh"),
        ];
        assert_eq!(
            passage_text(&verses, 1, 2).as_deref(),
            Some("Na počiatku Zem bola")
        );
        assert_eq!(passage_text(&verses, 3, 1).as_deref(), Some("Boh"));
        assert_eq!(passage_text(&verses, 7, 7), None);
        assert_eq!(passage_text(&[], 1, 1), None);
    }
}
