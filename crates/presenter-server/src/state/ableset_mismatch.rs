//! AbleSet<->Presenter song-number/title mismatch detection (#601) and its
//! per-number operator acknowledgement store.
//!
//! RED (this commit): only the test module exists, proving
//! `compute_ableset_mismatches` / `AckMap` / `AbleSetMismatchAck` do not
//! exist yet — the crate does not build. The GREEN commit adds the real
//! module (doc comment, types, `impl AppState` acknowledgement store, and
//! the pure comparison function) and restores it to a buildable, passing
//! state.

use std::collections::HashMap;

#[cfg(test)]
mod tests {
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
