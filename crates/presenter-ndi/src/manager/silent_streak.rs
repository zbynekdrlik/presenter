//! Per-source silent-start streak (#858).
//!
//! A silent broadcaster (SNV's Resolume is off for hours between events) makes
//! every `start_pipeline` build + start a pipeline, wait ~8 s and time out as
//! `PipelineStartError::SourceSilent`; the 30 s reconnect ticker then repeats it.
//! Nothing counted the repeats, so nothing could rate-limit the log lines they
//! produce (5 INFO lines per tick, 600/h, the whole journal).
//!
//! The manager counts the repeats HERE — the consecutive `SourceSilent` outcomes
//! per source — and hands the count out as `SourceSilent { attempt }`. Every line
//! about a silent attempt is gated on it with [`should_log_silent_attempt`] (the
//! `should_log_contention` / resolume #484 idiom), and the ONE line that ends a
//! streak, [`log_pipeline_streaming`], reports how long it was.
//!
//! A std mutex, held only for an increment / reset / retain and never across an
//! `.await` (the `active` map's rule, `ndi-manager-locking.md`, #741).

use std::collections::HashMap;
use std::sync::Mutex;

use super::PipelineStartError;

/// Whether a silent start attempt (1-based) logs at INFO: the first attempt and
/// then only the power-of-two milestones, so a long silence is ~log2(N)+1 lines
/// (a 24 h night of 30 s ticks: 12) instead of one per tick. Every other attempt
/// logs at DEBUG. `0` is not a power of two, so it never logs. Pure → unit-tested.
pub fn should_log_silent_attempt(attempt: u32) -> bool {
    attempt.is_power_of_two()
}

/// Consecutive silent start outcomes per source id.
#[derive(Default)]
pub(in crate::manager) struct SilentStreaks {
    streaks: Mutex<HashMap<String, u32>>,
}

impl SilentStreaks {
    /// Count one more silent attempt for `source_id` and return its 1-based number.
    pub(in crate::manager) fn bump(&self, source_id: &str) -> u32 {
        let mut streaks = self.streaks.lock().unwrap_or_else(|p| p.into_inner());
        let streak = streaks.entry(source_id.to_string()).or_insert(0);
        *streak = streak.saturating_add(1);
        *streak
    }

    /// End `source_id`'s streak (it reached Streaming, or was stopped); returns
    /// the length it had, `0` when there was none.
    pub(in crate::manager) fn reset(&self, source_id: &str) -> u32 {
        self.streaks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(source_id)
            .unwrap_or(0)
    }

    /// Forget every source's streak (`stop_all`).
    pub(in crate::manager) fn clear(&self) {
        self.streaks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    /// Forget every streak but `keep_id`'s (`stop_other_pipelines`, the #370 reap
    /// that runs after EVERY silent activation and must keep that source's count).
    pub(in crate::manager) fn retain_only(&self, keep_id: &str) {
        self.streaks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|id, _| id == keep_id);
    }

    /// The current streak length of `source_id` (`0` = none).
    #[cfg(test)]
    fn get(&self, source_id: &str) -> u32 {
        self.streaks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(source_id)
            .copied()
            .unwrap_or(0)
    }
}

/// The `start_pipeline` outcome for a silent broadcaster: counts the attempt and
/// carries its number, so the caller's log line can be gated on it.
pub(in crate::manager) fn silent_error(
    streaks: &SilentStreaks,
    source_id: &str,
    ndi_name: String,
) -> PipelineStartError {
    PipelineStartError::SourceSilent {
        ndi_name,
        attempt: streaks.bump(source_id),
    }
}

/// `installed` / `not-installed` for the streaming line's `timestamp_guard` field.
fn timestamp_guard_label(installed: bool) -> &'static str {
    if installed {
        "installed"
    } else {
        "not-installed"
    }
}

/// The ONE INFO line per real pipeline (#858): `source_id` was promoted to
/// Streaming. It ends the silent streak and reports its length, and it is the
/// anchor of the #854 "pipeline age" read (the build-time lines are DEBUG now).
/// Returns the silent attempts the source had before this pipeline.
pub(in crate::manager) fn log_pipeline_streaming(
    streaks: &SilentStreaks,
    source_id: &str,
    ndi_name: &str,
    encoder: &str,
    guard_installed: bool,
) -> u32 {
    let silent_attempts_before = streaks.reset(source_id);
    tracing::info!(
        source_id = %source_id,
        ndi_name = %ndi_name,
        encoder = %encoder,
        timestamp_guard = timestamp_guard_label(guard_installed),
        silent_attempts_before,
        "NDI pipeline streaming"
    );
    silent_attempts_before
}

#[cfg(test)]
mod tests {
    use super::*;

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
