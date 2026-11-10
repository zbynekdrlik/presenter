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

/// One source's silent streak: how many starts in a row were silent, and — for the
/// LAST of them — the advertised address no connection reached (#925).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SilentStreak {
    /// Consecutive silent starts (`0` = none / it ended).
    pub attempts: u32,
    /// The finder's address for the source when the last silent start's route
    /// found NO address that answered a TCP connect (see [`unreached_address`]);
    /// `None` when a connection was possible (or nothing was advertised).
    pub unreached_address: Option<String>,
}

/// Consecutive silent start outcomes per source id.
#[derive(Default)]
pub(in crate::manager) struct SilentStreaks {
    streaks: Mutex<HashMap<String, SilentStreak>>,
}

impl SilentStreaks {
    /// Count one more silent attempt for `source_id`, remember the address this
    /// attempt could not reach (`None` = it could connect), and return the
    /// attempt's 1-based number.
    pub(in crate::manager) fn bump(&self, source_id: &str, unreached: Option<String>) -> u32 {
        let mut streaks = self.streaks.lock().unwrap_or_else(|p| p.into_inner());
        let streak = streaks.entry(source_id.to_string()).or_default();
        streak.attempts = streak.attempts.saturating_add(1);
        streak.unreached_address = unreached;
        streak.attempts
    }

    /// End `source_id`'s streak (it reached Streaming, or was stopped); returns
    /// the length it had, `0` when there was none.
    pub(in crate::manager) fn reset(&self, source_id: &str) -> u32 {
        self.streaks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(source_id)
            .map_or(0, |streak| streak.attempts)
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

    /// The current streak of `source_id` (`attempts == 0` = none).
    pub(in crate::manager) fn streak(&self, source_id: &str) -> SilentStreak {
        self.streaks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(source_id)
            .cloned()
            .unwrap_or_default()
    }

    /// The current streak length of `source_id` (`0` = none).
    #[cfg(test)]
    fn get(&self, source_id: &str) -> u32 {
        self.streak(source_id).attempts
    }
}

/// The `start_pipeline` outcome for a silent broadcaster: counts the attempt and
/// carries its number, so the caller's log line can be gated on it.
pub(in crate::manager) fn silent_error(
    streaks: &SilentStreaks,
    source_id: &str,
    ndi_name: String,
    advertised_address: Option<String>,
    connect_address: Option<String>,
) -> PipelineStartError {
    PipelineStartError::SourceSilent {
        ndi_name,
        attempt: streaks.bump(
            source_id,
            unreached_address(advertised_address.as_deref(), connect_address.as_deref()),
        ),
        advertised_address,
        connect_address,
    }
}

/// The silent attempt (1-based) on which a source the finder still SEES gets its
/// one WARN (#925): attempt 2 = it stayed silent through a whole reconnect cycle
/// (~40 s) while being on the network — a sender that is merely off disappears
/// from the finder instead.
pub const SILENT_BUT_ADVERTISED_WARN_ATTEMPT: u32 = 2;

/// Whether a silent attempt logs the one "on the network at X but sends no
/// frames" WARN (#925): exactly once per silent streak, and only when the finder
/// reported an address for the source. Pure → unit-tested.
pub fn should_warn_silent_but_advertised(attempt: u32, advertised_address: Option<&str>) -> bool {
    attempt == SILENT_BUT_ADVERTISED_WARN_ATTEMPT && advertised_address.is_some()
}

/// The advertised address a silent start could NOT connect to (#925): the finder
/// reported one, and the route found no address of the sender that answered a TCP
/// connect — neither the advertised one nor an mDNS alternative — so ndisrc fell
/// back to the name (`connect_route::resolve_with`). When an address DID answer
/// (`connect` is set), the server can reach the sender and the silence is the
/// sender's own (#448): `None`. Pure → unit-tested.
pub fn unreached_address(advertised: Option<&str>, connect: Option<&str>) -> Option<String> {
    match (advertised, connect) {
        (Some(advertised), None) => Some(advertised.to_string()),
        _ => None,
    }
}

/// The address the Settings row names as "on the network, but cannot connect"
/// (#925), or `None` while that is not the diagnosis: the streak reached the
/// WARN attempt ([`SILENT_BUT_ADVERTISED_WARN_ATTEMPT`], a whole reconnect cycle)
/// and its last attempt reached no address of the sender. It ends with the
/// streak (every promote / stop). Pure → unit-tested.
pub fn unreachable_address(streak: &SilentStreak) -> Option<&str> {
    if streak.attempts >= SILENT_BUT_ADVERTISED_WARN_ATTEMPT {
        streak.unreached_address.as_deref()
    } else {
        None
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
    connect_address: Option<&str>,
) -> u32 {
    let silent_attempts_before = streaks.reset(source_id);
    tracing::info!(
        source_id = %source_id,
        ndi_name = %ndi_name,
        encoder = %encoder,
        timestamp_guard = timestamp_guard_label(guard_installed),
        silent_attempts_before,
        // #925: `-` = connected by name (the SDK's own address choice).
        connect_address = connect_address.unwrap_or("-"),
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
        assert_eq!(streaks.bump("a", None), 1);
        assert_eq!(streaks.bump("a", None), 2);
        assert_eq!(streaks.bump("a", None), 3);
        assert_eq!(streaks.get("a"), 3);
    }

    #[test]
    fn streaks_are_counted_per_source() {
        let streaks = SilentStreaks::default();
        assert_eq!(streaks.bump("a", None), 1);
        assert_eq!(streaks.bump("a", None), 2);
        assert_eq!(streaks.bump("b", None), 1, "b has its own streak");
        assert_eq!(streaks.bump("a", None), 3, "b's attempt did not touch a");
        assert_eq!(streaks.get("b"), 1);
        assert_eq!(streaks.get("unknown"), 0);
    }

    #[test]
    fn reset_reports_the_streak_it_ended_and_restarts_the_numbering() {
        let streaks = SilentStreaks::default();
        for _ in 0..3 {
            streaks.bump("a", None);
        }
        assert_eq!(streaks.reset("a"), 3, "the streak that just ended");
        assert_eq!(streaks.get("a"), 0);
        assert_eq!(
            streaks.bump("a", None),
            1,
            "numbering restarts after a reset"
        );
        assert_eq!(streaks.reset("a"), 1);
        assert_eq!(streaks.reset("a"), 0, "no streak to end");
        assert_eq!(streaks.reset("never-silent"), 0);
    }

    #[test]
    fn reset_touches_only_the_given_source() {
        let streaks = SilentStreaks::default();
        streaks.bump("a", None);
        streaks.bump("b", None);
        streaks.bump("b", None);
        assert_eq!(streaks.reset("a"), 1);
        assert_eq!(streaks.get("b"), 2, "b's streak is untouched");
    }

    #[test]
    fn clear_forgets_every_source() {
        let streaks = SilentStreaks::default();
        streaks.bump("a", None);
        streaks.bump("b", None);
        streaks.clear();
        assert_eq!(streaks.get("a"), 0);
        assert_eq!(streaks.get("b"), 0);
        assert_eq!(streaks.bump("a", None), 1);
    }

    // `stop_other_pipelines(keep)` runs after EVERY silent activation (the #370
    // reap): it must keep the active source's streak and forget the siblings'.
    #[test]
    fn retain_only_keeps_just_the_given_source() {
        let streaks = SilentStreaks::default();
        streaks.bump("a", None);
        streaks.bump("b", None);
        streaks.bump("b", None);
        streaks.bump("c", None);
        streaks.retain_only("b");
        assert_eq!(streaks.get("a"), 0, "a was reaped");
        assert_eq!(streaks.get("b"), 2, "the kept source keeps its streak");
        assert_eq!(streaks.get("c"), 0, "c was reaped");
    }

    #[test]
    fn silent_error_carries_the_attempt_number_and_the_name() {
        let streaks = SilentStreaks::default();
        for expected in 1..=3u32 {
            let err = silent_error(
                &streaks,
                "id",
                "RESOLUME-SNV (SP-program)".to_string(),
                None,
                None,
            );
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
            silent_error(&streaks, "id", "NAME".to_string(), None, None);
        }
        let before = log_pipeline_streaming(&streaks, "id", "NAME", "vah264enc", true, None);
        assert_eq!(before, 3, "the line reports the silent attempts before it");
        assert_eq!(streaks.get("id"), 0, "streaming ends the streak");
        let next = silent_error(&streaks, "id", "NAME".to_string(), None, None);
        assert_eq!(attempt_of(&next), 1, "a new silence starts over at 1");
    }

    /// #925: the silent outcome carries the finder's address for the source, so
    /// the caller can tell "on the network but unreachable" from "off".
    #[test]
    fn silent_error_carries_the_advertised_address() {
        let streaks = SilentStreaks::default();
        let err = silent_error(
            &streaks,
            "id",
            "RESOLUME-SNV (SP-program)".to_string(),
            Some("10.77.7.107:5961".to_string()),
            Some("10.77.9.201:5961".to_string()),
        );
        match err {
            PipelineStartError::SourceSilent {
                advertised_address,
                connect_address,
                ..
            } => {
                assert_eq!(advertised_address.as_deref(), Some("10.77.7.107:5961"));
                assert_eq!(connect_address.as_deref(), Some("10.77.9.201:5961"));
            }
            PipelineStartError::Failed(_) | PipelineStartError::Superseded => {
                panic!("expected SourceSilent, got {err:?}")
            }
        }
    }

    /// #925: one WARN per silent streak, on attempt 2, and only for a source the
    /// finder still sees (an advertised address) — a sender that is off has none.
    #[test]
    fn the_unreachable_warn_fires_once_per_streak_and_only_when_advertised() {
        let advertised = Some("10.77.7.107:5961");
        let warned: Vec<u32> = (0..=64)
            .filter(|attempt| should_warn_silent_but_advertised(*attempt, advertised))
            .collect();
        assert_eq!(warned, vec![SILENT_BUT_ADVERTISED_WARN_ATTEMPT]);
        assert_eq!(SILENT_BUT_ADVERTISED_WARN_ATTEMPT, 2);
        assert!((0..=64).all(|attempt| !should_warn_silent_but_advertised(attempt, None)));
    }

    /// #925: "could not connect" = advertised, and no address answered (name-only
    /// fallback); an address that answered (the advertised one or an alternative)
    /// means the server CAN reach the sender.
    #[test]
    fn unreached_only_when_no_address_of_the_sender_answered() {
        let advertised = Some("10.77.7.107:5961");
        assert_eq!(
            unreached_address(advertised, None).as_deref(),
            Some("10.77.7.107:5961")
        );
        assert_eq!(unreached_address(advertised, advertised), None);
        assert_eq!(
            unreached_address(advertised, Some("10.77.9.201:5961")),
            None
        );
        assert_eq!(unreached_address(None, None), None);
    }

    /// #925: the visible state starts with the WARN attempt, lasts for the rest of
    /// the streak, follows the LAST attempt's route, and ends with the streak.
    #[test]
    fn the_unreachable_state_follows_the_streak_and_its_last_route() {
        let streaks = SilentStreaks::default();
        let addr = || Some("10.77.7.107:5961".to_string());
        silent_error(&streaks, "id", "N".to_string(), addr(), None);
        assert_eq!(
            unreachable_address(&streaks.streak("id")),
            None,
            "attempt 1"
        );
        silent_error(&streaks, "id", "N".to_string(), addr(), None);
        assert_eq!(
            unreachable_address(&streaks.streak("id")),
            Some("10.77.7.107:5961"),
            "attempt 2 = the WARN attempt"
        );
        assert!(should_warn_silent_but_advertised(2, addr().as_deref()));
        // The next attempt connected (the sender came back on a routed address)
        // but is still silent: the server CAN reach it — no longer "unreachable".
        silent_error(&streaks, "id", "N".to_string(), addr(), addr());
        assert_eq!(unreachable_address(&streaks.streak("id")), None);
        silent_error(&streaks, "id", "N".to_string(), addr(), None);
        assert!(unreachable_address(&streaks.streak("id")).is_some());
        log_pipeline_streaming(&streaks, "id", "N", "x264enc", false, None);
        assert_eq!(
            streaks.streak("id"),
            SilentStreak::default(),
            "streaming ends it"
        );
    }

    #[test]
    fn a_pipeline_that_was_never_silent_reports_zero_attempts_before() {
        let streaks = SilentStreaks::default();
        assert_eq!(
            log_pipeline_streaming(&streaks, "id", "NAME", "x264enc", false, None),
            0
        );
    }

    #[test]
    fn timestamp_guard_label_names_both_states() {
        assert_eq!(timestamp_guard_label(true), "installed");
        assert_eq!(timestamp_guard_label(false), "not-installed");
    }
}
