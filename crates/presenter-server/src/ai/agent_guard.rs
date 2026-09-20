//! Per-turn agent guard (#784): bound `run_agent`'s loop by wall-clock time and
//! by consecutive identical validation rejections, ending the turn with a clear
//! Slovak operator message instead of spinning into the 120 s client timeout.
//!
//! **Why this exists:** at PP (2026-09-20) the operator asked for a
//! non-contiguous passage; every rendering of the reference tripped the (then
//! strict) `bible_validator` and `run_agent` (`MAX_ITERATIONS = 100`, no
//! wall-clock budget, no cap on repeated identical rejections) kept re-asking
//! the model for > 4 minutes until iteration 24 hit the 120 s client timeout —
//! surfacing as `AI chat request failed` and flipping the header badge to
//! `connected:false` for the 15-min fold, even though the backend was healthy.
//!
//! The normaliser (`bible_validator::normalize_reference`) fixes THAT request;
//! this guard bounds every FUTURE format/model mismatch to seconds. A GaveUp is
//! deliberately NOT recorded into `AiCallHealth` (`ai::last_error`) — it is not
//! a transport outage, so it must never flip the "AI down" badge.
//!
//! Pure and clock-driven so both the budget check and the consecutive-rejection
//! counter are unit-testable without the full loop or a real 90 s wait — the
//! same testability pattern as `context_budget::parse_context_budget_bytes` and
//! `last_error::AiCallHealth::active_failure(window)`.

use std::time::{Duration, Instant};

/// Default per-turn wall-clock budget when `PRESENTER_AI_TURN_BUDGET_SECS` is
/// unset or invalid. Comfortably under the 120 s client timeout
/// (`ai::client`), so the guard always ends the turn with a friendly message
/// BEFORE the transport-level timeout that flips the badge.
pub(crate) const DEFAULT_TURN_BUDGET_SECS: u64 = 90;

/// How many CONSECUTIVE identical-rule validation rejections end the turn.
/// Three strikes of the SAME rule proves the model cannot satisfy this
/// validator for this request — retrying more only spins toward the timeout.
pub(crate) const MAX_CONSECUTIVE_REJECTIONS: u32 = 3;

/// What the guard decides after a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentOutcome {
    /// Keep looping.
    Continue,
    /// Stop the turn and show this Slovak message to the operator as the
    /// assistant's final response (NOT an error — the backend is healthy).
    GaveUp { reason: String },
}

/// Parse `PRESENTER_AI_TURN_BUDGET_SECS`. A missing / non-numeric / zero value
/// falls back to [`DEFAULT_TURN_BUDGET_SECS`] (a `0` budget would give up before
/// the first provider call, disabling the agent — never what an operator wants).
/// Pure (takes the raw string) so it is unit-testable without env mutation.
pub(crate) fn parse_turn_budget_secs(raw: Option<&str>) -> Duration {
    let secs = raw
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_TURN_BUDGET_SECS);
    Duration::from_secs(secs)
}

/// The env-driven per-turn budget used in production (`run_agent`).
pub(crate) fn turn_budget() -> Duration {
    parse_turn_budget_secs(
        std::env::var("PRESENTER_AI_TURN_BUDGET_SECS")
            .ok()
            .as_deref(),
    )
}

/// Slovak give-up message for the wall-clock budget path.
fn budget_give_up_message(elapsed: Duration) -> String {
    format!(
        "AI nestihla dokončiť požiadavku v časovom limite ({} s). \
         Skús zadanie zjednodušiť alebo rozdeliť.",
        elapsed.as_secs()
    )
}

/// Slovak give-up message for the repeated-rejection path.
fn rejection_give_up_message(rule: &str) -> String {
    format!(
        "AI nevedela zložiť biblický odkaz vo formáte, ktorý validátor prijme \
         (pravidlo: {rule}). Skús zadanie zjednodušiť alebo rozdeliť."
    )
}

/// Bounds a single agent turn. Constructed once per turn; `check_budget` runs
/// before every provider call and `observe_rejection` runs after every tool
/// round.
pub(crate) struct AgentGuard {
    start: Instant,
    budget: Duration,
    max_consecutive: u32,
    last_rule: Option<String>,
    consecutive: u32,
}

impl AgentGuard {
    /// Explicit budget + threshold — used by tests for a deterministic, fast
    /// budget without touching env or waiting a real 90 s.
    pub(crate) fn new(budget: Duration, max_consecutive: u32) -> Self {
        Self {
            start: Instant::now(),
            budget,
            max_consecutive,
            last_rule: None,
            consecutive: 0,
        }
    }

    /// Production constructor: env budget + [`MAX_CONSECUTIVE_REJECTIONS`].
    pub(crate) fn from_env() -> Self {
        Self::new(turn_budget(), MAX_CONSECUTIVE_REJECTIONS)
    }

    /// Called BEFORE each provider call. `GaveUp` once the turn's wall-clock
    /// budget is spent — never a network error, so the badge is untouched.
    pub(crate) fn check_budget(&self) -> AgentOutcome {
        let elapsed = self.start.elapsed();
        if elapsed >= self.budget {
            AgentOutcome::GaveUp {
                reason: budget_give_up_message(elapsed),
            }
        } else {
            AgentOutcome::Continue
        }
    }

    /// Called AFTER each tool round with the validation rule of a rejection this
    /// round, or `None` when the round produced no slide-validation rejection.
    /// `None` resets the streak; an identical rule increments it; a DIFFERENT
    /// rule restarts the streak at that new rule. `GaveUp` once the SAME rule
    /// has been hit `max_consecutive` times in a row.
    pub(crate) fn observe_rejection(&mut self, rejected_rule: Option<&str>) -> AgentOutcome {
        match rejected_rule {
            None => {
                self.last_rule = None;
                self.consecutive = 0;
                AgentOutcome::Continue
            }
            Some(rule) => {
                if self.last_rule.as_deref() == Some(rule) {
                    self.consecutive += 1;
                } else {
                    self.last_rule = Some(rule.to_string());
                    self.consecutive = 1;
                }
                if self.consecutive >= self.max_consecutive {
                    AgentOutcome::GaveUp {
                        reason: rejection_give_up_message(rule),
                    }
                } else {
                    AgentOutcome::Continue
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_budget_defaults_when_unset_or_invalid() {
        assert_eq!(
            parse_turn_budget_secs(None),
            Duration::from_secs(DEFAULT_TURN_BUDGET_SECS)
        );
        assert_eq!(
            parse_turn_budget_secs(Some("not-a-number")),
            Duration::from_secs(DEFAULT_TURN_BUDGET_SECS)
        );
        // A zero budget would disable the agent — rejected, falls back.
        assert_eq!(
            parse_turn_budget_secs(Some("0")),
            Duration::from_secs(DEFAULT_TURN_BUDGET_SECS)
        );
    }

    #[test]
    fn parse_budget_accepts_an_explicit_override() {
        assert_eq!(parse_turn_budget_secs(Some("30")), Duration::from_secs(30));
        assert_eq!(
            parse_turn_budget_secs(Some("  45 ")),
            Duration::from_secs(45)
        );
    }

    #[test]
    fn budget_not_exceeded_while_time_remains() {
        let guard = AgentGuard::new(Duration::from_secs(3600), MAX_CONSECUTIVE_REJECTIONS);
        assert_eq!(guard.check_budget(), AgentOutcome::Continue);
    }

    #[test]
    fn budget_exceeded_gives_up_with_a_slovak_time_message() {
        // A zero budget is (by any positive elapsed) already spent — proves the
        // `elapsed >= budget` gate without a real wait.
        let guard = AgentGuard::new(Duration::ZERO, MAX_CONSECUTIVE_REJECTIONS);
        match guard.check_budget() {
            AgentOutcome::GaveUp { reason } => {
                assert!(
                    reason.contains("časovom limite"),
                    "budget give-up must name the time limit in Slovak: {reason}"
                );
            }
            AgentOutcome::Continue => panic!("a spent budget must give up"),
        }
    }

    #[test]
    fn three_consecutive_identical_rejections_give_up() {
        let mut guard = AgentGuard::new(Duration::from_secs(3600), 3);
        assert_eq!(
            guard.observe_rejection(Some("reference_format_requires_parens")),
            AgentOutcome::Continue,
            "first strike continues"
        );
        assert_eq!(
            guard.observe_rejection(Some("reference_format_requires_parens")),
            AgentOutcome::Continue,
            "second strike continues"
        );
        match guard.observe_rejection(Some("reference_format_requires_parens")) {
            AgentOutcome::GaveUp { reason } => assert!(
                reason.contains("pravidlo: reference_format_requires_parens"),
                "rejection give-up must name the rule: {reason}"
            ),
            AgentOutcome::Continue => panic!("third identical strike must give up"),
        }
    }

    #[test]
    fn a_different_rule_restarts_the_streak() {
        let mut guard = AgentGuard::new(Duration::from_secs(3600), 3);
        guard.observe_rejection(Some("reference_format_requires_parens"));
        guard.observe_rejection(Some("reference_format_requires_parens"));
        // A different rule resets to a streak of 1 — must NOT give up here.
        assert_eq!(
            guard.observe_rejection(Some("missing_verse_number_prefix")),
            AgentOutcome::Continue,
            "a different rule restarts the streak, not gives up"
        );
        assert_eq!(
            guard.observe_rejection(Some("missing_verse_number_prefix")),
            AgentOutcome::Continue
        );
    }

    #[test]
    fn a_successful_round_resets_the_streak() {
        let mut guard = AgentGuard::new(Duration::from_secs(3600), 3);
        guard.observe_rejection(Some("reference_format_requires_parens"));
        guard.observe_rejection(Some("reference_format_requires_parens"));
        // A round with no rejection clears the streak entirely.
        assert_eq!(guard.observe_rejection(None), AgentOutcome::Continue);
        // ...so the next two identical rejections are only strikes 1 and 2.
        assert_eq!(
            guard.observe_rejection(Some("reference_format_requires_parens")),
            AgentOutcome::Continue
        );
        assert_eq!(
            guard.observe_rejection(Some("reference_format_requires_parens")),
            AgentOutcome::Continue
        );
    }
}
