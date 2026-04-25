//! Per-connection adaptive controller for `/ndi/mjpeg`.
//!
//! Keeps a sliding 30-second window of `broadcast::RecvError::Lagged`
//! events. Demotes one tier when the window holds 5+ events; promotes
//! one tier after 60 seconds of zero lag at the current tier.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use presenter_ndi::Tier;

const LAG_WINDOW: Duration = Duration::from_secs(30);
const LAG_DEMOTE_THRESHOLD: usize = 5;
const PROMOTE_AFTER: Duration = Duration::from_secs(60);

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
            if self.last_lag_at.map_or(true, |t| now.duration_since(t) >= PROMOTE_AFTER) {
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

    /// Called when broadcast::RecvError::Lagged is observed.
    pub fn on_lag(&mut self, now: Instant) -> AdaptDecision {
        self.lag_events.push_back(now);
        self.last_lag_at = Some(now);
        self.trim_window(now);
        if self.lag_events.len() >= LAG_DEMOTE_THRESHOLD {
            if let Some(next) = self.tier.demote() {
                self.tier = next;
                self.entered_tier_at = now;
                self.lag_events.clear();
                return AdaptDecision::Demote(next);
            }
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

    fn add_lags(c: &mut AdaptController, t0: Instant, count: usize, spacing_ms: u64) -> Vec<AdaptDecision> {
        let mut out = Vec::new();
        for i in 0..count {
            out.push(c.on_lag(t0 + Duration::from_millis(i as u64 * spacing_ms)));
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
    fn lags_outside_window_dont_count() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L0);
        // 4 lags at the start
        add_lags(&mut c, t0, 4, 1000);
        // 1 lag 60 seconds later — first 4 are now outside window, so total in window is 1
        let d = c.on_lag(t0 + Duration::from_secs(60));
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
        c.on_lag(t0 + Duration::from_secs(5));
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
    fn ceiling_l0_cannot_promote() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L0);
        let d = c.on_frame(t0 + Duration::from_secs(120));
        assert_eq!(d, AdaptDecision::Stay);
    }
}
