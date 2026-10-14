//! Candidate ranking (#850 spec §5.5).

use crate::bible::resolver::rank::{no_fold_candidate, osa_distance, rank};
use crate::bible::resolver::{fold_book_text, BookCandidate, MatchKind};

fn candidates(input: &str) -> Vec<BookCandidate> {
    no_fold_candidate(input)
        .map(|candidate| vec![candidate])
        .unwrap_or_else(|| rank(&fold_book_text(input), &[]))
}

fn codes(input: &str) -> Vec<String> {
    candidates(input).into_iter().map(|c| c.code).collect()
}

fn first(input: &str) -> String {
    codes(input).into_iter().next().unwrap_or_default()
}

#[test]
fn fold_collisions_and_the_no_fold_list() {
    assert_eq!(first("ab"), "HAB");
    assert_eq!(first("1k"), "1CO");
    assert_eq!(first("2k"), "2CO");
    assert_eq!(first("ps"), "PSA");
    assert_eq!(first("PŠ"), "SNG");
    assert_eq!(first("P.Š."), "SNG");
    assert_eq!(codes("1k")[..3], ["1CO", "1KI", "1CH"]);
}

#[test]
fn short_keys_follow_the_curated_defaults() {
    assert_eq!(first("z"), "PSA");
    assert_eq!(first("ž"), "PSA");
    assert_eq!(first("j"), "JHN");
    assert_eq!(first("jo"), "JOB");
    assert_eq!(first("kr"), "1KI");
    assert_eq!(first("jd"), "JUD");
    assert_eq!(codes("g")[..2], ["GEN", "GAL"]);
    assert_eq!(codes("z")[..6], ["PSA", "REV", "HEB", "ZEC", "LAM", "ZEP"]);
}

#[test]
fn one_letter_aliases_carry_no_extra_weight() {
    let jhn = candidates("j").remove(0);
    assert_eq!((jhn.code.as_str(), jhn.kind), ("JHN", MatchKind::Prefix));
}

#[test]
fn a_numbered_family_defaults_to_its_first_book() {
    assert_eq!(codes("kor")[..2], ["1CO", "2CO"]);
    assert_eq!(codes("tim")[..2], ["1TI", "2TI"]);
    let candidate = candidates("kor").remove(0);
    assert_eq!(
        (candidate.kind, candidate.penalty),
        (MatchKind::OrdinalStripped, 1)
    );
}

#[test]
fn exact_aliases_carry_their_source_form() {
    let heb = candidates("zid").remove(0);
    assert_eq!((heb.code.as_str(), heb.kind), ("HEB", MatchKind::Exact));
    assert_eq!(heb.matched_form, "Žid");
    assert_eq!(heb.number, 58);
}

#[test]
fn fuzzy_only_from_four_letters_within_the_length_limits() {
    let fuzzy = |input: &str| {
        candidates(input)
            .first()
            .filter(|c| c.kind == MatchKind::Fuzzy)
            .map(|c| (c.code.clone(), c.penalty))
    };
    assert!(candidates("hbr").is_empty(), "shorter than 4: never fuzzy");
    assert_eq!(
        fuzzy("hebx"),
        Some(("HEB".to_string(), 1)),
        "4–6 letters: distance 1"
    );
    assert_eq!(fuzzy("hxbx"), None, "4–6 letters: distance 2 is too far");
    assert_eq!(fuzzy("hebrjm"), None, "6 letters: distance 2 is too far");
    assert_eq!(
        fuzzy("hebrjmo"),
        Some(("HEB".to_string(), 2)),
        "7+ letters: distance 2"
    );
    assert_eq!(fuzzy("hebrjom"), Some(("HEB".to_string(), 1)));
}

#[test]
fn osa_distance_counts_a_transposition_once_and_caps_at_three() {
    assert_eq!(osa_distance(b"hebrejom", b"hebrjom"), 1);
    assert_eq!(osa_distance(b"hebrejom", b"hebrejom"), 0);
    assert_eq!(osa_distance(b"zidom", b"zdiom"), 1);
    assert_eq!(osa_distance(b"ab", b"abcdef"), 3);
}
