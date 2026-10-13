//! Guarded drift self-heal — the BACKSTOP behind the encoder timestamp guard
//! (#854).
//!
//! The supervisor's existing 30 s tick reads the shared encoder's trailing-30s
//! lead (from the lead meter on the producer appsink, after the guard) and
//! feeds it here. When the window's MINIMUM lead is >= 300 ms on two
//! consecutive ticks, the pipeline is rebuilt through the normal dead-state
//! path. A frame-counter lead is carried by every frame (min ≈ max), so a
//! single future-stamped buffer can never trigger it; with the guard working
//! the lead stays <= the guard's 50 ms deadband (below even the 100 ms clear
//! level) and this never fires — a firing is a regression signal.
//!
//! Loop-proofing (every rebuild blanks every TV on the site for ~10-20 s,
//! #385):
//!   - the trigger is a PIPELINE-level signal at the shared encoder output,
//!     never consumer drop ratios (join drops and reconnect churn would loop
//!     rebuilds — the #263 lockstep lesson);
//!   - drift rebuilds are >= 5 min apart (the #337 `COOL_OFF_WINDOW`) and at
//!     most 3 per rolling hour; after that it is alarm-only;
//!   - the memory lives in `NdiManager` (keyed by source), so it survives a
//!     supervisor being replaced by the reconnect ticker.
//!
//! Pure data, synthetic `Instant`s, no I/O: unit-tested on any CI host.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::supervisor::SupervisorState;

/// Window-minimum lead (ms) that counts a tick as "over". The consumer queue
/// still has ~200 ms headroom here (it overflows at 500 ms), so the rebuild
/// lands before any frame drops.
const LEAD_TRIGGER_MS: i64 = 300;

/// Below this the lead is cleared (hysteresis: 100..300 holds the count).
const LEAD_CLEAR_MS: i64 = 100;

/// Consecutive over ticks (30 s windows) before a rebuild.
const SUSTAIN_TICKS: u32 = 2;

/// Drift rebuilds allowed per rolling [`BUDGET_WINDOW`].
const MAX_REBUILDS_PER_HOUR: usize = 3;

const BUDGET_WINDOW: Duration = Duration::from_secs(60 * 60);

/// Minimum spacing between two drift rebuilds — the #337 cool-off window.
const MIN_SPACING: Duration = SupervisorState::COOL_OFF_WINDOW;

/// Why a due rebuild was not done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::manager) enum SuppressReason {
    /// The previous drift rebuild was under [`MIN_SPACING`] ago.
    Spacing { remaining: Duration },
    /// [`MAX_REBUILDS_PER_HOUR`] drift rebuilds already in the trailing hour.
    HourlyBudget,
}

/// What the supervisor should do after one tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::manager) enum DriftAction {
    /// Nothing to do.
    Idle,
    /// A rebuild is due: request it, and [`DriftGuard::record_rebuild`] it only
    /// once the pipeline confirmed the request.
    Rebuild { lead_min_ms: i64 },
    /// A rebuild is due but rate-limited; `log` is the power-of-two gate over
    /// the suppression streak (`log-flood-backoff.md`).
    Suppressed {
        lead_min_ms: i64,
        reason: SuppressReason,
        log: bool,
    },
    /// The lead dropped back under [`LEAD_CLEAR_MS`] after being over.
    Recovered,
}

/// Per-source drift memory.
#[derive(Debug, Default)]
pub(in crate::manager) struct DriftGuard {
    over_ticks: u32,
    fires: VecDeque<Instant>,
    suppressed_streak: u32,
    total_fires: u32,
}

impl DriftGuard {
    /// One supervisor tick. `lead_min_ms` is the window minimum of the shared
    /// encoder's lead; `None` (no frames in the window — a source stall, or a
    /// fresh pipeline) is neutral: a stall is not drift.
    pub(in crate::manager) fn on_tick(
        &mut self,
        now: Instant,
        lead_min_ms: Option<i64>,
    ) -> DriftAction {
        let Some(lead) = lead_min_ms else {
            return DriftAction::Idle;
        };
        if lead < LEAD_CLEAR_MS {
            let was_over = self.over_ticks > 0;
            self.over_ticks = 0;
            self.suppressed_streak = 0;
            return if was_over {
                DriftAction::Recovered
            } else {
                DriftAction::Idle
            };
        }
        if lead < LEAD_TRIGGER_MS {
            // Hysteresis band: hold the count, neither clear nor advance.
            return DriftAction::Idle;
        }
        self.over_ticks = self.over_ticks.saturating_add(1);
        if self.over_ticks < SUSTAIN_TICKS {
            return DriftAction::Idle;
        }
        while self
            .fires
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= BUDGET_WINDOW)
        {
            self.fires.pop_front();
        }
        let since_last = self
            .fires
            .back()
            .map(|at| now.saturating_duration_since(*at));
        let reason = match since_last {
            Some(since) if since < MIN_SPACING => Some(SuppressReason::Spacing {
                remaining: MIN_SPACING - since,
            }),
            _ if self.fires.len() >= MAX_REBUILDS_PER_HOUR => Some(SuppressReason::HourlyBudget),
            _ => None,
        };
        match reason {
            None => {
                // Due, not yet done: nothing is recorded until the pipeline
                // confirms the request (`record_rebuild`), so a refused one
                // spends neither the spacing nor the budget.
                self.suppressed_streak = 0;
                DriftAction::Rebuild { lead_min_ms: lead }
            }
            Some(reason) => {
                self.suppressed_streak = self.suppressed_streak.saturating_add(1);
                DriftAction::Suppressed {
                    lead_min_ms: lead,
                    reason,
                    log: self.suppressed_streak.is_power_of_two(),
                }
            }
        }
    }

    /// Record a drift rebuild the pipeline CONFIRMED
    /// (`request_self_heal_rebuild` returned true): it starts the 5 min
    /// spacing, spends one of the hour's rebuilds and counts in
    /// `driftRebuilds`, and the fresh pipeline needs two new over windows. A
    /// refused request (the pipeline left Streaming first) is never recorded.
    pub(in crate::manager) fn record_rebuild(&mut self, now: Instant) {
        self.fires.push_back(now);
        self.total_fires = self.total_fires.saturating_add(1);
        self.over_ticks = 0;
        self.suppressed_streak = 0;
    }

    /// Drift rebuilds for this source since process start (`/healthz`
    /// `driftRebuilds`).
    pub(in crate::manager) fn total_fires(&self) -> u32 {
        self.total_fires
    }

    /// Drift rebuilds inside the trailing hour (as of the last tick).
    pub(in crate::manager) fn fires_in_last_hour(&self) -> usize {
        self.fires.len()
    }
}

/// Log one tick's outcome: WARN on a rebuild, WARN/ERROR on a (power-of-two
/// gated) suppression, INFO on recovery, DEBUG while the lead is >= 100 ms.
pub(in crate::manager) fn log_drift_action(
    source_id: &str,
    lead: Option<(i64, i64)>,
    action: DriftAction,
    fires_last_hour: usize,
    total_fires: u32,
) {
    let lead_min = lead.map(|(min, _)| min);
    let lead_max = lead.map(|(_, max)| max);
    match action {
        DriftAction::Idle => {
            if lead_min.is_some_and(|min| min >= LEAD_CLEAR_MS) {
                tracing::debug!(
                    source_id = %source_id,
                    lead_min_ms = ?lead_min,
                    lead_max_ms = ?lead_max,
                    "NDI drift check: shared encoder timestamps lead the clock (#854)"
                );
            }
        }
        DriftAction::Rebuild { lead_min_ms } => {
            tracing::warn!(
                source_id = %source_id,
                lead_min_ms,
                lead_max_ms = ?lead_max,
                fires_last_hour,
                total_fires,
                "NDI self-heal: shared encoder timestamps lead the clock by >= {lead_min_ms} ms \
                 for two windows — rebuilding the pipeline (#854)"
            );
        }
        DriftAction::Suppressed {
            lead_min_ms,
            reason,
            log,
        } => {
            if !log {
                return;
            }
            match reason {
                SuppressReason::Spacing { remaining } => tracing::warn!(
                    source_id = %source_id,
                    lead_min_ms,
                    remaining_s = remaining.as_secs(),
                    fires_last_hour,
                    total_fires,
                    "NDI self-heal due but the last drift rebuild was under 5 min ago — \
                     waiting (#854)"
                ),
                SuppressReason::HourlyBudget => tracing::error!(
                    source_id = %source_id,
                    lead_min_ms,
                    fires_last_hour,
                    total_fires,
                    "NDI self-heal suppressed: 3 drift rebuilds in the last hour — alarm only; \
                     the shared encoder timestamps still lead the clock (#854)"
                ),
            }
        }
        DriftAction::Recovered => {
            tracing::info!(
                source_id = %source_id,
                lead_min_ms = ?lead_min,
                "NDI drift check: shared encoder lead back under 100 ms (#854)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_secs(30);

    fn is_rebuild(action: DriftAction) -> bool {
        matches!(action, DriftAction::Rebuild { .. })
    }

    /// One tick whose due rebuild the pipeline confirms
    /// (`request_self_heal_rebuild` returned true, the normal case), recorded
    /// the way the supervisor records it.
    fn tick(g: &mut DriftGuard, at: Instant, lead_min_ms: Option<i64>) -> DriftAction {
        let action = g.on_tick(at, lead_min_ms);
        if is_rebuild(action) {
            g.record_rebuild(at);
        }
        action
    }

    #[test]
    fn a_due_rebuild_is_recorded_only_once_confirmed() {
        // `on_tick` only reports a rebuild as due; the supervisor records it
        // after `request_self_heal_rebuild` confirms it. A pipeline that left
        // Streaming first is not rebuilt, so nothing may be spent on it.
        let t0 = Instant::now();
        let mut g = DriftGuard::default();
        assert_eq!(g.on_tick(t0, Some(400)), DriftAction::Idle);
        assert_eq!(
            g.on_tick(t0 + TICK, Some(400)),
            DriftAction::Rebuild { lead_min_ms: 400 }
        );
        assert_eq!(g.total_fires(), 0, "unconfirmed rebuild in driftRebuilds");
        assert_eq!(
            g.fires_in_last_hour(),
            0,
            "unconfirmed rebuild spent the hourly budget"
        );
        // Neither the 5 min spacing nor the budget was consumed: still due.
        assert_eq!(
            g.on_tick(t0 + 2 * TICK, Some(400)),
            DriftAction::Rebuild { lead_min_ms: 400 }
        );
        // Confirmed this time.
        g.record_rebuild(t0 + 2 * TICK);
        assert_eq!(g.total_fires(), 1);
        assert_eq!(g.fires_in_last_hour(), 1);
        // A confirmed rebuild restarts the count and starts the spacing.
        assert_eq!(g.on_tick(t0 + 3 * TICK, Some(400)), DriftAction::Idle);
        assert!(matches!(
            g.on_tick(t0 + 4 * TICK, Some(400)),
            DriftAction::Suppressed {
                reason: SuppressReason::Spacing { .. },
                ..
            }
        ));
    }

    #[test]
    fn a_lead_sustained_over_two_windows_rebuilds() {
        let t0 = Instant::now();
        let mut g = DriftGuard::default();
        assert_eq!(tick(&mut g, t0, Some(300)), DriftAction::Idle);
        assert_eq!(
            tick(&mut g, t0 + TICK, Some(300)),
            DriftAction::Rebuild { lead_min_ms: 300 }
        );
        assert_eq!(g.total_fires(), 1);
        assert_eq!(g.fires_in_last_hour(), 1);
        // The count restarts after a rebuild: the fresh pipeline needs two
        // new over windows.
        assert_eq!(tick(&mut g, t0 + 2 * TICK, Some(400)), DriftAction::Idle);
    }

    #[test]
    fn a_lead_just_under_the_trigger_never_rebuilds() {
        let t0 = Instant::now();
        let mut g = DriftGuard::default();
        for k in 0..10u32 {
            assert_eq!(tick(&mut g, t0 + TICK * k, Some(299)), DriftAction::Idle);
        }
        assert_eq!(g.total_fires(), 0);
    }

    #[test]
    fn a_lead_capped_at_the_guard_deadband_never_counts() {
        // #854 deadband: with the timestamp guard working the shared encoder's
        // lead is at most D = 50 ms — under the 100 ms clear level, so it
        // clears an over count and never starts one.
        let d = crate::pipeline::encoder_timing::TIMESTAMP_DEADBAND_MS;
        let t0 = Instant::now();
        let mut g = DriftGuard::default();
        assert_eq!(tick(&mut g, t0, Some(400)), DriftAction::Idle);
        assert_eq!(tick(&mut g, t0 + TICK, Some(d)), DriftAction::Recovered);
        for k in 2..20u32 {
            assert_eq!(tick(&mut g, t0 + TICK * k, Some(d)), DriftAction::Idle);
        }
        assert_eq!(g.total_fires(), 0);
    }

    #[test]
    fn a_single_over_window_followed_by_a_clear_one_does_not_count() {
        let t0 = Instant::now();
        let mut g = DriftGuard::default();
        assert_eq!(tick(&mut g, t0, Some(800)), DriftAction::Idle);
        assert_eq!(tick(&mut g, t0 + TICK, Some(-20)), DriftAction::Recovered);
        assert_eq!(tick(&mut g, t0 + 2 * TICK, Some(800)), DriftAction::Idle);
        assert!(is_rebuild(tick(&mut g, t0 + 3 * TICK, Some(800))));
    }

    #[test]
    fn the_hysteresis_band_holds_the_count() {
        let t0 = Instant::now();
        let mut g = DriftGuard::default();
        assert_eq!(tick(&mut g, t0, Some(400)), DriftAction::Idle);
        // 100 ms is inside the band (only < 100 clears).
        assert_eq!(tick(&mut g, t0 + TICK, Some(100)), DriftAction::Idle);
        assert_eq!(tick(&mut g, t0 + 2 * TICK, Some(299)), DriftAction::Idle);
        assert!(is_rebuild(tick(&mut g, t0 + 3 * TICK, Some(400))));
    }

    #[test]
    fn ninety_nine_ms_clears_the_count() {
        let t0 = Instant::now();
        let mut g = DriftGuard::default();
        assert_eq!(tick(&mut g, t0, Some(400)), DriftAction::Idle);
        assert_eq!(tick(&mut g, t0 + TICK, Some(99)), DriftAction::Recovered);
        assert_eq!(tick(&mut g, t0 + 2 * TICK, Some(400)), DriftAction::Idle);
    }

    #[test]
    fn recovered_is_reported_only_after_being_over() {
        let t0 = Instant::now();
        let mut g = DriftGuard::default();
        assert_eq!(tick(&mut g, t0, Some(-5)), DriftAction::Idle);
        assert_eq!(tick(&mut g, t0 + TICK, Some(150)), DriftAction::Idle);
        assert_eq!(tick(&mut g, t0 + 2 * TICK, Some(10)), DriftAction::Idle);
    }

    #[test]
    fn no_frames_in_the_window_is_neutral() {
        let t0 = Instant::now();
        let mut g = DriftGuard::default();
        assert_eq!(tick(&mut g, t0, None), DriftAction::Idle);
        assert_eq!(tick(&mut g, t0 + TICK, Some(400)), DriftAction::Idle);
        // A stall between two over windows neither clears nor advances.
        assert_eq!(tick(&mut g, t0 + 2 * TICK, None), DriftAction::Idle);
        assert!(is_rebuild(tick(&mut g, t0 + 3 * TICK, Some(400))));
    }

    #[test]
    fn drift_rebuilds_are_at_least_five_minutes_apart() {
        // Shifted forward so `t0 - TICK` can never underflow the clock.
        let t0 = Instant::now() + TICK;
        let mut g = DriftGuard::default();
        tick(&mut g, t0 - TICK, Some(400));
        assert!(is_rebuild(tick(&mut g, t0, Some(400))));
        assert_eq!(tick(&mut g, t0 + TICK, Some(400)), DriftAction::Idle);
        assert_eq!(
            tick(&mut g, t0 + 2 * TICK, Some(400)),
            DriftAction::Suppressed {
                lead_min_ms: 400,
                reason: SuppressReason::Spacing {
                    remaining: Duration::from_secs(240)
                },
                log: true,
            }
        );
        let just_before = t0 + Duration::from_secs(4 * 60 + 59);
        assert!(matches!(
            tick(&mut g, just_before, Some(400)),
            DriftAction::Suppressed {
                reason: SuppressReason::Spacing { remaining },
                ..
            } if remaining == Duration::from_secs(1)
        ));
        assert!(is_rebuild(tick(
            &mut g,
            t0 + Duration::from_secs(5 * 60),
            Some(400)
        )));
        assert_eq!(g.total_fires(), 2);
    }

    #[test]
    fn at_most_three_drift_rebuilds_per_rolling_hour() {
        let t0 = Instant::now() + TICK;
        let minute = |m: u64| t0 + Duration::from_secs(m * 60);
        let mut g = DriftGuard::default();
        for m in [0u64, 5, 10] {
            assert_eq!(
                tick(&mut g, minute(m) - TICK / 2, Some(500)),
                DriftAction::Idle
            );
            assert!(
                is_rebuild(tick(&mut g, minute(m), Some(500))),
                "rebuild at {m} min"
            );
        }
        assert_eq!(g.fires_in_last_hour(), 3);
        // From 15 min on, a rebuild is due every tick but the hour is spent.
        assert_eq!(
            tick(&mut g, minute(15) - TICK / 2, Some(500)),
            DriftAction::Idle
        );
        let mut logged = Vec::new();
        let mut at = minute(15);
        while at < minute(60) {
            match tick(&mut g, at, Some(500)) {
                DriftAction::Suppressed {
                    reason: SuppressReason::HourlyBudget,
                    log,
                    ..
                } => logged.push(log),
                other => panic!("expected the hourly budget at {at:?}, got {other:?}"),
            }
            at += TICK;
        }
        // Power-of-two log gate over the suppression streak.
        assert_eq!(
            &logged[..9],
            &[true, true, false, true, false, false, false, true, false]
        );
        // The first rebuild ages out of the hour exactly at 60 min.
        assert!(is_rebuild(tick(&mut g, minute(60), Some(500))));
        assert_eq!(g.total_fires(), 4);
    }
}
