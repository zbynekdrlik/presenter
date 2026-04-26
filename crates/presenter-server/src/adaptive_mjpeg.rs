//! Per-connection adaptive controller for `/ndi/mjpeg`.
//!
//! Maintains a sliding 30-second window of "slow events". A slow event is
//! either a `broadcast::RecvError::Lagged` OR a successful `Ok` recv that
//! arrives more than `SLOW_MULTIPLIER × tier_interval` after the previous
//! Ok (caught by the caller and reported via `on_lag`). The latter is
//! essential because hyper's body buffer can absorb tens of MB before TCP
//! backpressure ever overflows the broadcast queue, so `RecvError::Lagged`
//! alone is an unreliable signal for "this client is slow".
//!
//! Demotes one tier when the window holds 5+ slow events; promotes one
//! tier after 60 seconds of zero slow events at the current tier.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use presenter_ndi::Tier;

const LAG_WINDOW: Duration = Duration::from_secs(30);
const LAG_DEMOTE_THRESHOLD: usize = 5;
const PROMOTE_AFTER: Duration = Duration::from_secs(60);
const SLOW_MULTIPLIER: u32 = 2;

/// A single Lagged event with this many dropped frames is treated as
/// definitive evidence of a slow client — demote immediately, do not wait
/// for accumulated count. 30 ≈ 1 second of lost stream at 30 fps. Real
/// observed slow-client events drop ~600+ frames per recv, so this
/// triggers cleanly without false-positives from typical network blips
/// (which produce ≤5 dropped frames).
const SEVERE_DROP_THRESHOLD: u64 = 30;

/// Returns the threshold for treating an inter-Ok gap as a "slow tick" at the given tier.
pub fn slow_tick_threshold(tier: Tier) -> Duration {
    let fps = match tier {
        Tier::L0 => 30,
        Tier::L1 | Tier::L2 => 15,
        Tier::L3 => 10,
    };
    Duration::from_millis(1000 * SLOW_MULTIPLIER as u64 / fps)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptDecision {
    Stay,
    Demote(Tier),
    Promote(Tier),
}

pub struct AdaptController {
    tier: Tier,
    lag_events: VecDeque<Instant>,
    last_lag_at: Option<Instant>,
    entered_tier_at: Instant,
}

impl AdaptController {
    pub fn new(initial: Tier) -> Self {
        let now = Instant::now();
        Self {
            tier: initial,
            lag_events: VecDeque::new(),
            last_lag_at: None,
            entered_tier_at: now,
        }
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Called when a successful frame is received. Returns Promote if conditions met.
    pub fn on_frame(&mut self, now: Instant) -> AdaptDecision {
        self.trim_window(now);
        if self.tier != Tier::L0 && now.duration_since(self.entered_tier_at) >= PROMOTE_AFTER {
            // 60 s smooth at this tier and we have a higher tier to try.
            if self
                .last_lag_at
                .map_or(true, |t| now.duration_since(t) >= PROMOTE_AFTER)
            {
                if let Some(next) = self.tier.promote() {
                    self.tier = next;
                    self.entered_tier_at = now;
                    self.lag_events.clear();
                    return AdaptDecision::Promote(next);
                }
            }
        }
        AdaptDecision::Stay
    }

    /// Called when a slow event is observed — either a `RecvError::Lagged(n)`
    /// or an Ok recv that arrived past `slow_tick_threshold`. `dropped` is the
    /// number of frames the client missed: directly the `n` from `Lagged(n)`,
    /// or for slow ticks an estimate derived from `elapsed_ms × tier_fps`.
    ///
    /// A single event with `dropped >= SEVERE_DROP_THRESHOLD` triggers an
    /// immediate demote — no false-positive risk because typical network
    /// blips drop a handful of frames at most. Smaller events are
    /// accumulated and trigger demote at `LAG_DEMOTE_THRESHOLD` count
    /// inside the 30-second window.
    pub fn on_lag(&mut self, now: Instant, dropped: u64) -> AdaptDecision {
        self.lag_events.push_back(now);
        self.last_lag_at = Some(now);
        self.trim_window(now);
        let severe = dropped >= SEVERE_DROP_THRESHOLD;
        if severe || self.lag_events.len() >= LAG_DEMOTE_THRESHOLD {
            if let Some(next) = self.tier.demote() {
                self.tier = next;
                self.entered_tier_at = now;
                self.lag_events.clear();
                return AdaptDecision::Demote(next);
            }
            // At floor (L3): no further demote possible. Clear the window so
            // events don't accumulate forever — trim_window already bounds
            // growth via the 30 s window, but with high-frequency slow-tick
            // events this could hold hundreds of timestamps. Clearing is
            // semantically correct: we've already responded to the lag as
            // much as we can.
            self.lag_events.clear();
        }
        AdaptDecision::Stay
    }

    fn trim_window(&mut self, now: Instant) {
        while let Some(front) = self.lag_events.front() {
            if now.duration_since(*front) > LAG_WINDOW {
                self.lag_events.pop_front();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: drop=1 means non-severe. Tests using this exercise the
    /// accumulated-threshold path, not the severe-immediate path.
    fn add_lags(
        c: &mut AdaptController,
        t0: Instant,
        count: usize,
        spacing_ms: u64,
    ) -> Vec<AdaptDecision> {
        let mut out = Vec::new();
        for i in 0..count {
            out.push(c.on_lag(t0 + Duration::from_millis(i as u64 * spacing_ms), 1));
        }
        out
    }

    #[test]
    fn five_lags_in_30s_demotes() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L0);
        let decisions = add_lags(&mut c, t0, 5, 1000);
        assert_eq!(decisions[..4], [AdaptDecision::Stay; 4]);
        assert_eq!(decisions[4], AdaptDecision::Demote(Tier::L1));
        assert_eq!(c.tier(), Tier::L1);
    }

    #[test]
    fn single_severe_lag_demotes_immediately() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L0);
        // SEVERE_DROP_THRESHOLD is 30; pass 600 (matches real-world observation)
        let d = c.on_lag(t0, 600);
        assert_eq!(d, AdaptDecision::Demote(Tier::L1));
        assert_eq!(c.tier(), Tier::L1);
    }

    #[test]
    fn small_lag_below_severe_threshold_does_not_demote_alone() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L0);
        let d = c.on_lag(t0, 5);
        assert_eq!(d, AdaptDecision::Stay);
        assert_eq!(c.tier(), Tier::L0);
    }

    #[test]
    fn lags_outside_window_dont_count() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L0);
        // 4 lags at the start
        add_lags(&mut c, t0, 4, 1000);
        // 1 lag 60 seconds later — first 4 are now outside window, so total in window is 1
        let d = c.on_lag(t0 + Duration::from_secs(60), 1);
        assert_eq!(d, AdaptDecision::Stay);
        assert_eq!(c.tier(), Tier::L0);
    }

    #[test]
    fn promote_after_60s_clean_at_l1() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L1);
        // No lag events; pass time by reporting frames
        let d1 = c.on_frame(t0 + Duration::from_secs(30));
        assert_eq!(d1, AdaptDecision::Stay);
        let d2 = c.on_frame(t0 + Duration::from_secs(61));
        assert_eq!(d2, AdaptDecision::Promote(Tier::L0));
        assert_eq!(c.tier(), Tier::L0);
    }

    #[test]
    fn promote_blocked_by_recent_lag() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L1);
        // Lag at +5s — resets entered_tier_at? Actually NO: lag at L1 doesn't change tier (it's fewer than 5 in window).
        c.on_lag(t0 + Duration::from_secs(5), 1);
        // At +61s, window holds zero events (30s window), but last_lag_at was 56s ago — less than 60s.
        let d = c.on_frame(t0 + Duration::from_secs(61));
        assert_eq!(d, AdaptDecision::Stay);
        // At +66s (61s after the lag), promote allowed.
        let d2 = c.on_frame(t0 + Duration::from_secs(66));
        assert_eq!(d2, AdaptDecision::Promote(Tier::L0));
    }

    #[test]
    fn floor_l3_cannot_demote() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L3);
        // 5 rapid lags
        let decisions = add_lags(&mut c, t0, 5, 100);
        assert_eq!(decisions[4], AdaptDecision::Stay, "L3 has no demote target");
        assert_eq!(c.tier(), Tier::L3);
    }

    #[test]
    fn floor_l3_cannot_demote_even_on_severe_lag() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L3);
        let d = c.on_lag(t0, 1000);
        assert_eq!(d, AdaptDecision::Stay);
        assert_eq!(c.tier(), Tier::L3);
    }

    #[test]
    fn ceiling_l0_cannot_promote() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L0);
        let d = c.on_frame(t0 + Duration::from_secs(120));
        assert_eq!(d, AdaptDecision::Stay);
    }

    #[test]
    fn slow_tick_threshold_scales_with_tier() {
        // L0 = 30 fps → interval 33ms → threshold 66ms
        assert_eq!(slow_tick_threshold(Tier::L0), Duration::from_millis(66));
        // L1 = 15 fps → interval 66ms → threshold 132ms (rounds to 133ms via integer math)
        assert_eq!(slow_tick_threshold(Tier::L1), Duration::from_millis(133));
        // L2 = 15 fps → 133ms
        assert_eq!(slow_tick_threshold(Tier::L2), Duration::from_millis(133));
        // L3 = 10 fps → 200ms
        assert_eq!(slow_tick_threshold(Tier::L3), Duration::from_millis(200));
    }
}
