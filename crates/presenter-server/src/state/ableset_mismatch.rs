//! Pure AbleSet<->Presenter song-number/title mismatch COMPARISON logic
//! (#601). The acknowledgement store (persistence) lives in the sibling
//! `ableset_ack.rs` (#655 — split out to keep both files under the repo's
//! per-file size cap, same precedent as `state/background_tasks.rs`; F4's
//! domain refusal enum and F9's mutex/prune/cap/validation land there in a
//! follow-up commit).
//!
//! The song NUMBER is the only link between AbleSet and Presenter —
//! `resolve_ableset_presentation` matches purely on the numeric prefix, so a
//! silent numbering disagreement puts the WRONG lyrics on the wall with no
//! warning (the exact failure that hit prod SNV four times on 2026-07-27,
//! see the issue's settled design comments). This module is the pre-service
//! checklist that catches it: it NEVER blocks projection (per the issue's
//! explicit decision — this is a report to read before the service, not a
//! runtime gate). It is recomputed on every AbleSet cache rebuild
//! (`ableset.rs::refresh_ableset_cache`), atomically with the #639
//! generation guard, and surfaced read-only via `GET
//! /integrations/ableset/status`.

use std::collections::{BTreeSet, HashMap};

use presenter_core::{normalize_title_for_mismatch, strip_song_prefix, AbleSetTitleMismatch};

use super::ableset_ack::AckMap;

/// Compare the live AbleSet setlist against the resolved Presenter library
/// and report per-number title disagreements the operator has not
/// acknowledged (#601). Three outcomes per number:
///
/// - both sides present and agree once normalised (diacritics/whitespace/
///   punctuation/case folded — see `normalize_title_for_mismatch`) → silent.
/// - both sides present, titles genuinely differ, NOT acknowledged for this
///   exact pair → reported.
/// - present on only one side → ALWAYS reported; acknowledgement does not
///   apply — a missing number is a structural gap, not a naming choice, and
///   cannot be silenced away.
pub(crate) fn compute_ableset_mismatches(
    presenter_titles: &HashMap<String, String>,
    ableset_songs: &[(String, String)],
    prefix_length: u8,
    acks: &AckMap,
) -> Vec<AbleSetTitleMismatch> {
    let ableset_by_number: HashMap<&str, &str> = ableset_songs
        .iter()
        .map(|(number, title)| (number.as_str(), title.as_str()))
        .collect();

    let mut numbers: BTreeSet<&str> = presenter_titles.keys().map(String::as_str).collect();
    numbers.extend(ableset_by_number.keys().copied());

    numbers
        .into_iter()
        .filter_map(|number| {
            let presenter_title = presenter_titles.get(number).map(String::as_str);
            let ableset_title = ableset_by_number.get(number).copied();
            mismatch_for_number(number, ableset_title, presenter_title, prefix_length, acks)
        })
        .collect()
}

/// Single-number decision extracted from `compute_ableset_mismatches` to
/// keep that function under the repo's per-function line cap.
fn mismatch_for_number(
    number: &str,
    ableset_title: Option<&str>,
    presenter_title: Option<&str>,
    prefix_length: u8,
    acks: &AckMap,
) -> Option<AbleSetTitleMismatch> {
    match (ableset_title, presenter_title) {
        (Some(a), Some(p)) => {
            let differs = normalize_title_for_mismatch(strip_song_prefix(a, prefix_length))
                != normalize_title_for_mismatch(strip_song_prefix(p, prefix_length));
            let acknowledged = acks
                .get(number)
                .is_some_and(|ack| ack.ableset_title == a && ack.presenter_title == p);
            if differs && !acknowledged {
                Some(AbleSetTitleMismatch {
                    number: number.to_string(),
                    ableset_title: a.to_string(),
                    presenter_title: p.to_string(),
                })
            } else {
                None
            }
        }
        (Some(a), None) => Some(AbleSetTitleMismatch {
            number: number.to_string(),
            ableset_title: a.to_string(),
            presenter_title: String::new(),
        }),
        (None, Some(p)) => Some(AbleSetTitleMismatch {
            number: number.to_string(),
            ableset_title: String::new(),
            presenter_title: p.to_string(),
        }),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::ableset_ack::AbleSetMismatchAck;
    use super::*;

    fn ack(ableset_title: &str, presenter_title: &str) -> AbleSetMismatchAck {
        AbleSetMismatchAck {
            ableset_title: ableset_title.to_string(),
            presenter_title: presenter_title.to_string(),
        }
    }

    #[test]
    fn diacritic_only_difference_is_silent() {
        // #601 acceptance: 130 of 164 SNV songs differ only by diacritics —
        // that must NEVER be reported, or the warning is useless noise.
        let presenter = HashMap::from([("102".to_string(), "102 10 000 armád".to_string())]);
        let ableset = vec![("102".to_string(), "102 10000 armad".to_string())];
        let mismatches = compute_ableset_mismatches(&presenter, &ableset, 3, &AckMap::new());
        assert!(
            mismatches.is_empty(),
            "a diacritic-only difference must never be reported: {mismatches:?}"
        );
    }

    #[test]
    fn genuinely_different_title_is_reported() {
        // #601 live evidence: prod SNV's real 017 disagreement before the
        // user's correction — Ableton played one song, Presenter would have
        // shown a completely different song's lyrics.
        let presenter =
            HashMap::from([("017".to_string(), "017 Tvoja blízkosť je nebo".to_string())]);
        let ableset = vec![("017".to_string(), "017 Viem, ze Ty Pan".to_string())];
        let mismatches = compute_ableset_mismatches(&presenter, &ableset, 3, &AckMap::new());
        assert_eq!(
            mismatches.len(),
            1,
            "a genuinely different title must be reported"
        );
        assert_eq!(mismatches[0].number, "017");
    }

    #[test]
    fn acknowledged_pair_is_silenced() {
        let presenter = HashMap::from([("088".to_string(), "088 Alive with you".to_string())]);
        let ableset = vec![("088".to_string(), "088 Alive with you KIDS".to_string())];
        let acks = AckMap::from([(
            "088".to_string(),
            ack("088 Alive with you KIDS", "088 Alive with you"),
        )]);
        let mismatches = compute_ableset_mismatches(&presenter, &ableset, 3, &acks);
        assert!(
            mismatches.is_empty(),
            "an acknowledged exact title pair must stay silent"
        );
    }

    #[test]
    fn ack_bound_to_titles_does_not_survive_a_title_change() {
        // Settled design: the acknowledgement is bound to the two titles it
        // was granted for, not to the number alone — changing either title
        // re-raises the warning.
        let presenter = HashMap::from([(
            "088".to_string(),
            "088 Alive with you (new title)".to_string(),
        )]);
        let ableset = vec![("088".to_string(), "088 Alive with you KIDS".to_string())];
        let acks = AckMap::from([(
            "088".to_string(),
            ack("088 Alive with you KIDS", "088 Alive with you"),
        )]);
        let mismatches = compute_ableset_mismatches(&presenter, &ableset, 3, &acks);
        assert_eq!(
            mismatches.len(),
            1,
            "a title change after acknowledgement must re-raise the warning, not stay silent"
        );
    }

    // #655 F9e — RED (this commit): the ack comparison is currently raw
    // `==`, so a purely cosmetic edit (whitespace/case here) on either title
    // WRONGLY re-raises a warning the operator already dismissed. GREEN
    // switches the comparison to `normalize_title_for_mismatch` on both
    // sides.
    #[test]
    fn ack_survives_a_purely_cosmetic_title_change() {
        let presenter = HashMap::from([("088".to_string(), "088 Alive With  You".to_string())]);
        let ableset = vec![("088".to_string(), "088 alive with you kids".to_string())];
        let acks = AckMap::from([(
            "088".to_string(),
            ack("088 Alive with you KIDS", "088 Alive with you"),
        )]);
        let mismatches = compute_ableset_mismatches(&presenter, &ableset, 3, &acks);
        assert!(
            mismatches.is_empty(),
            "a purely cosmetic title edit must not re-raise an acknowledged mismatch: {mismatches:?}"
        );
    }

    #[test]
    fn number_missing_from_presenter_is_always_reported_even_if_acked() {
        let presenter = HashMap::new();
        let ableset = vec![("099".to_string(), "099 Only In AbleSet".to_string())];
        // A structural gap must be reported regardless of any ack on file —
        // acknowledgement only silences a NAMING disagreement.
        let acks = AckMap::from([("099".to_string(), ack("099 Only In AbleSet", ""))]);
        let mismatches = compute_ableset_mismatches(&presenter, &ableset, 3, &acks);
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].presenter_title, "");
    }

    #[test]
    fn number_missing_from_ableset_is_always_reported() {
        let presenter = HashMap::from([("055".to_string(), "055 Only In Presenter".to_string())]);
        let mismatches = compute_ableset_mismatches(&presenter, &[], 3, &AckMap::new());
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].ableset_title, "");
    }

    #[test]
    fn matching_titles_produce_no_mismatch() {
        let presenter = HashMap::from([("001".to_string(), "001 Amazing Grace".to_string())]);
        let ableset = vec![("001".to_string(), "001 Amazing Grace".to_string())];
        let mismatches = compute_ableset_mismatches(&presenter, &ableset, 3, &AckMap::new());
        assert!(mismatches.is_empty());
    }
}
