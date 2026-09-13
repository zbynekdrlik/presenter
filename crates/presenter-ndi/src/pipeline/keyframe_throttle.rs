//! Join-time forced-IDR coalescing for the shared encoder (#768 D5).
//!
//! Every WHEP consumer join calls `request_keyframe(producer)` so the fresh
//! consumer starts decoding immediately instead of waiting up to one GOP for
//! the next scheduled keyframe. That forced IDR lands on the SHARED encoder,
//! so it affects EVERY consumer, not just the joiner.
//!
//! Under reconnect churn this becomes a load problem: the incident's PP data
//! (2026-09-13) showed a Sharp TV re-creating its WHEP session ~every 45s (13
//! POST / 10 min) and sd1l on SNV ~every 24s — each join forcing another IDR
//! on the shared encoder. An IDR outside the scheduled GOP is a bitrate spike
//! and re-arms `needs_keyframe` for the other consumers' StreamProducer links.
//!
//! `KeyframeThrottle` coalesces those: it forces at most ONE IDR per
//! `min_interval` (one GOP, ~2s) per pipeline. A join inside the window skips
//! the forced request and rides the next scheduled/natural keyframe (≤ one
//! GOP away) instead. This does NOT touch the browser-PLI path — the browser's
//! own force-key-unit is forwarded upstream inside `StreamProducer`, not through
//! `request_keyframe`, so it is out of scope here (and the incident showed
//! keyframe pressure is not the fan-out DROP cause; that is D2).

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One GOP at `key-int-max=60` / 30fps = 2s. A forced IDR is coalesced to at
/// most one per this interval per pipeline; a join inside the window waits for
/// the next scheduled keyframe (which is at most one GOP away).
pub(super) const IDR_MIN_INTERVAL: Duration = Duration::from_secs(2);

/// Per-pipeline coalescer for join-time forced IDRs. Cheap interior mutability
/// (`std::sync::Mutex<Option<Instant>>`) so it lives behind `&self` on
/// `NdiPipeline` and the decision is made in `add_consumer` before the blocking
/// builder runs.
pub(super) struct KeyframeThrottle {
    last: Mutex<Option<Instant>>,
    min_interval: Duration,
}

impl KeyframeThrottle {
    /// A throttle coalescing to at most one forced IDR per one GOP
    /// (`IDR_MIN_INTERVAL`).
    pub(super) fn new() -> Self {
        Self::with_interval(IDR_MIN_INTERVAL)
    }

    /// A throttle with an explicit `min_interval` — for tests.
    pub(super) fn with_interval(min_interval: Duration) -> Self {
        Self {
            last: Mutex::new(None),
            min_interval,
        }
    }

    /// Should a forced IDR be issued at `now`?
    ///
    /// Returns `true` — and records `now` as the last-forced instant — when no
    /// IDR was forced within `min_interval` (the first join ever, or the first
    /// after the window elapsed). Returns `false` (skip; ride the next
    /// scheduled keyframe) for a join that lands inside the window since the
    /// last forced IDR. Uses `saturating_duration_since` so an out-of-order
    /// `now` can never panic.
    pub(super) fn should_force(&self, now: Instant) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        match *last {
            Some(prev) if now.saturating_duration_since(prev) < self.min_interval => false,
            _ => {
                *last = Some(now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_join_forces_an_idr() {
        let throttle = KeyframeThrottle::new();
        assert!(
            throttle.should_force(Instant::now()),
            "the very first join must force an IDR (fresh consumer needs one)"
        );
    }

    #[test]
    fn join_inside_the_window_is_coalesced() {
        let throttle = KeyframeThrottle::with_interval(Duration::from_secs(2));
        let base = Instant::now();
        assert!(throttle.should_force(base), "first join forces");
        assert!(
            !throttle.should_force(base + Duration::from_millis(500)),
            "a join 500ms later (inside the 2s GOP window) must be coalesced — \
             ride the next scheduled keyframe instead of forcing another IDR"
        );
        assert!(
            !throttle.should_force(base + Duration::from_millis(1_999)),
            "a join just before the window closes is still coalesced"
        );
    }

    #[test]
    fn join_after_the_window_forces_again() {
        let throttle = KeyframeThrottle::with_interval(Duration::from_secs(2));
        let base = Instant::now();
        assert!(throttle.should_force(base), "first join forces");
        assert!(
            !throttle.should_force(base + Duration::from_millis(1_000)),
            "inside window: coalesced"
        );
        assert!(
            throttle.should_force(base + Duration::from_millis(2_001)),
            "a join after the 2s window (measured from the LAST forced IDR) forces again"
        );
    }

    #[test]
    fn window_is_measured_from_the_last_forced_idr_not_the_last_call() {
        // A coalesced (skipped) join must NOT reset the window — otherwise a
        // steady stream of joins every 1.9s would never force an IDR at all.
        let throttle = KeyframeThrottle::with_interval(Duration::from_secs(2));
        let base = Instant::now();
        assert!(throttle.should_force(base), "t=0 forces");
        assert!(
            !throttle.should_force(base + Duration::from_millis(1_500)),
            "t=1.5s coalesced"
        );
        assert!(
            throttle.should_force(base + Duration::from_millis(2_100)),
            "t=2.1s forces (2.1s since the last FORCE at t=0, not since t=1.5s)"
        );
    }
}
