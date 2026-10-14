//! Per-source silent-start streak (#858).
//!
//! RED stage: the tests below pin the behavior; the implementation lands in the
//! following commit.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::PipelineStartError;

    /// The attempt number a `SourceSilent` carries. A total match (never `if let`)
    /// so a new variant is a compile error here, not a silently skipped arm.
    fn attempt_of(err: &PipelineStartError) -> u32 {
        match err {
            PipelineStartError::SourceSilent { attempt, .. } => *attempt,
            PipelineStartError::Failed(_) | PipelineStartError::Superseded => {
                panic!("expected SourceSilent, got {err:?}")
            }
        }
    }

    // The log gate is the resolume `should_log_error` idiom (#484): the first
    // attempt and then only the power-of-two milestones, so a night of silence
    // (~2880 attempts) logs ~12 INFO lines instead of one per 30 s tick.
    #[test]
    fn gate_logs_the_first_attempt_and_the_power_of_two_milestones() {
        for attempt in [1, 2, 4, 8, 16, 32, 1024, 1 << 31] {
            assert!(
                should_log_silent_attempt(attempt),
                "attempt #{attempt} is a milestone and must log at INFO"
            );
        }
        for attempt in [0, 3, 5, 6, 7, 9, 100, 1000, u32::MAX] {
            assert!(
                !should_log_silent_attempt(attempt),
                "attempt #{attempt} must stay at DEBUG"
            );
        }
    }

    #[test]
    fn gate_logs_exactly_log2_plus_one_lines_over_a_long_streak() {
        let logged = (1..=2880u32)
            .filter(|attempt| should_log_silent_attempt(*attempt))
            .count();
        assert_eq!(
            logged, 12,
            "a 24 h silent night (2880 x 30 s) logs attempts 1,2,4,...,2048"
        );
    }

    #[test]
    fn consecutive_silent_attempts_are_numbered_from_one() {
        let streaks = SilentStreaks::default();
        assert_eq!(streaks.bump("a"), 1);
        assert_eq!(streaks.bump("a"), 2);
        assert_eq!(streaks.bump("a"), 3);
        assert_eq!(streaks.get("a"), 3);
    }

    #[test]
    fn streaks_are_counted_per_source() {
        let streaks = SilentStreaks::default();
        assert_eq!(streaks.bump("a"), 1);
        assert_eq!(streaks.bump("a"), 2);
        assert_eq!(streaks.bump("b"), 1, "b has its own streak");
        assert_eq!(streaks.bump("a"), 3, "b's attempt did not touch a");
        assert_eq!(streaks.get("b"), 1);
        assert_eq!(streaks.get("unknown"), 0);
    }

    #[test]
    fn reset_reports_the_streak_it_ended_and_restarts_the_numbering() {
        let streaks = SilentStreaks::default();
        for _ in 0..3 {
            streaks.bump("a");
        }
        assert_eq!(streaks.reset("a"), 3, "the streak that just ended");
        assert_eq!(streaks.get("a"), 0);
        assert_eq!(streaks.bump("a"), 1, "numbering restarts after a reset");
        assert_eq!(streaks.reset("a"), 1);
        assert_eq!(streaks.reset("a"), 0, "no streak to end");
        assert_eq!(streaks.reset("never-silent"), 0);
    }

    #[test]
    fn reset_touches_only_the_given_source() {
        let streaks = SilentStreaks::default();
        streaks.bump("a");
        streaks.bump("b");
        streaks.bump("b");
        assert_eq!(streaks.reset("a"), 1);
        assert_eq!(streaks.get("b"), 2, "b's streak is untouched");
    }

    #[test]
    fn clear_forgets_every_source() {
        let streaks = SilentStreaks::default();
        streaks.bump("a");
        streaks.bump("b");
        streaks.clear();
        assert_eq!(streaks.get("a"), 0);
        assert_eq!(streaks.get("b"), 0);
        assert_eq!(streaks.bump("a"), 1);
    }

    // `stop_other_pipelines(keep)` runs after EVERY silent activation (the #370
    // reap): it must keep the active source's streak and forget the siblings'.
    #[test]
    fn retain_only_keeps_just_the_given_source() {
        let streaks = SilentStreaks::default();
        streaks.bump("a");
        streaks.bump("b");
        streaks.bump("b");
        streaks.bump("c");
        streaks.retain_only("b");
        assert_eq!(streaks.get("a"), 0, "a was reaped");
        assert_eq!(streaks.get("b"), 2, "the kept source keeps its streak");
        assert_eq!(streaks.get("c"), 0, "c was reaped");
    }

    #[test]
    fn silent_error_carries_the_attempt_number_and_the_name() {
        let streaks = SilentStreaks::default();
        for expected in 1..=3u32 {
            let err = silent_error(&streaks, "id", "RESOLUME-SNV (SP-program)".to_string());
            assert_eq!(attempt_of(&err), expected);
            assert!(
                err.to_string().contains("RESOLUME-SNV (SP-program)"),
                "the error still names the source; got {err}"
            );
        }
    }

    #[test]
    fn reaching_streaming_ends_the_streak() {
        let streaks = SilentStreaks::default();
        for _ in 0..3 {
            silent_error(&streaks, "id", "NAME".to_string());
        }
        let before = log_pipeline_streaming(&streaks, "id", "NAME", "vah264enc", true);
        assert_eq!(before, 3, "the line reports the silent attempts before it");
        assert_eq!(streaks.get("id"), 0, "streaming ends the streak");
        let next = silent_error(&streaks, "id", "NAME".to_string());
        assert_eq!(attempt_of(&next), 1, "a new silence starts over at 1");
    }

    #[test]
    fn a_pipeline_that_was_never_silent_reports_zero_attempts_before() {
        let streaks = SilentStreaks::default();
        assert_eq!(
            log_pipeline_streaming(&streaks, "id", "NAME", "x264enc", false),
            0
        );
    }

    #[test]
    fn timestamp_guard_label_names_both_states() {
        assert_eq!(timestamp_guard_label(true), "installed");
        assert_eq!(timestamp_guard_label(false), "not-installed");
    }
}
