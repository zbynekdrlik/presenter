//! The fold and the generated name table (#850 spec §5.2, §5.3, §10).

use serde::Deserialize;

use crate::bible::resolver::names_table::{
    BOOK_CODES, BOOK_KEYS, FREQUENCY_RANK, NO_FOLD, SHORT_DEFAULTS,
};
use crate::bible::resolver::{fold_book_text, FoldedBook};

#[derive(Deserialize)]
struct FoldCases {
    cases: Vec<FoldCase>,
}

#[derive(Deserialize)]
struct FoldCase {
    input: String,
    ordinal: u8,
    key: String,
}

fn row_codes(key: &str, ordinal: u8) -> Vec<&'static str> {
    BOOK_KEYS
        .iter()
        .filter(|row| row.0 == key && row.3 & 0x0F == ordinal)
        .map(|row| BOOK_CODES[usize::from(row.2)])
        .collect()
}

#[test]
fn fold_matches_the_cases_shared_with_the_generator() {
    let cases: FoldCases = serde_json::from_str(include_str!("../testdata/fold-cases.json"))
        .expect("fold cases parse");
    assert!(cases.cases.len() >= 40);
    for case in cases.cases {
        let expected = FoldedBook {
            ordinal: (case.ordinal != 0).then_some(case.ordinal),
            key: case.key.clone(),
        };
        assert_eq!(
            fold_book_text(&case.input),
            expected,
            "fold({:?})",
            case.input
        );
    }
}

#[test]
fn every_generated_row_folds_back_to_its_key() {
    for &(key, form, index, packed) in BOOK_KEYS {
        let folded = fold_book_text(form);
        assert_eq!(folded.key, key, "{form:?} (book {index})");
        assert_eq!(
            folded.ordinal.unwrap_or(0),
            packed & 0x0F,
            "{form:?} ordinal"
        );
    }
}

#[test]
fn table_covers_66_distinct_codes_with_valid_rows() {
    let mut codes = BOOK_CODES.to_vec();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), 66);
    let mut seen = [false; 66];
    for &(_, _, index, packed) in BOOK_KEYS {
        assert!(usize::from(index) < 66);
        assert!((1..=3).contains(&(packed >> 4)), "tier of {packed:#x}");
        assert!(packed & 0x0F <= 5, "ordinal of {packed:#x}");
        seen[usize::from(index)] = true;
    }
    assert!(seen.iter().all(|book| *book), "every book has a key");
}

#[test]
fn table_rows_are_sorted_without_duplicate_key_ordinal_code() {
    for pair in BOOK_KEYS.windows(2) {
        let a = (pair[0].0, pair[0].3 & 0x0F, pair[0].2);
        let b = (pair[1].0, pair[1].3 & 0x0F, pair[1].2);
        assert!(a < b, "{a:?} then {b:?}");
    }
}

#[test]
fn short_defaults_and_frequency_rank_are_valid() {
    for pair in SHORT_DEFAULTS.windows(2) {
        assert!(pair[0].0 < pair[1].0, "SHORT_DEFAULTS sorted");
    }
    for &(key, books) in SHORT_DEFAULTS {
        let letters = key.trim_start_matches(|c: char| c.is_ascii_digit());
        assert!(!letters.is_empty() && letters.len() <= 2, "{key}");
        assert!(
            !books.is_empty() && books.iter().all(|b| usize::from(*b) < 66),
            "{key}"
        );
    }
    let mut ranks = FREQUENCY_RANK.to_vec();
    ranks.sort_unstable();
    assert_eq!(ranks, (1..=66).collect::<Vec<u8>>());
}

#[test]
fn table_carries_the_data_fixes() {
    assert_eq!(row_codes("ab", 0), ["HAB"], "Ab is Habakkuk, never Obadiah");
    assert_eq!(
        row_codes("ps", 0),
        ["PSA"],
        "P.Š. must not fold into Psalms"
    );
    assert!(row_codes("mr", 0).contains(&"MRK"));
    assert!(row_codes("filimonovi", 0).contains(&"PHM"));
    assert!(row_codes("zid", 0).contains(&"HEB"));
    assert_eq!(NO_FOLD, [("pš", 21_u8)].as_slice());
}

#[test]
fn slovak_1k_and_2k_beat_english_kings() {
    let tier_of = |ordinal: u8, index: u8| {
        BOOK_KEYS
            .iter()
            .find(|row| row.0 == "k" && row.3 & 0x0F == ordinal && row.2 == index)
            .map(|row| row.3 >> 4)
    };
    assert_eq!(tier_of(1, 45), Some(1), "1K = 1 Korinťanom (SEP), tier 1");
    assert_eq!(
        tier_of(1, 10),
        Some(3),
        "1 K = 1 Kings only in BPRP English, tier 3"
    );
    assert_eq!(tier_of(2, 46), Some(1), "2K = 2 Korinťanom (SEP), tier 1");
    assert_eq!(
        tier_of(2, 11),
        Some(3),
        "2 K = 2 Kings only in BPRP English, tier 3"
    );
}
