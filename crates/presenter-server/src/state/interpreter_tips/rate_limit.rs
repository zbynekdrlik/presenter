//! In-memory tip rate limits (#852 spec §9.8): at most `TIP_RATE_PER_SENDER`
//! accepted tips per `TIP_RATE_WINDOW_SECS` per (sender IP, sender name) —
//! church devices behind the Cloudflare tunnel share one public IP, so the
//! name is part of the key (case-insensitive) — and `TIP_RATE_OVERALL`
//! overall. Each key keeps a sliding window of ACCEPTED instants, so "the 6th
//! tip within 60 s" is refused exactly and `Retry-After` is the moment the
//! oldest one leaves. Pure: the caller passes `now` (tokio `Instant`). Reset
//! on restart (spec).

use presenter_core::interpreter_tips::{
    TIP_RATE_OVERALL, TIP_RATE_PER_SENDER, TIP_RATE_WINDOW_SECS,
};
use std::collections::{HashMap, VecDeque};
use tokio::time::{Duration, Instant};

/// Which limit refused a tip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RateScope {
    Sender,
    Overall,
}

impl RateScope {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Sender => "sender",
            Self::Overall => "overall",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RateVerdict {
    Allowed,
    Refused {
        scope: RateScope,
        retry_after_secs: u64,
        /// `Some(n)` = log this refusal now (one WARN per window per key);
        /// `n` refusals were suppressed since the previous WARN.
        warn: Option<u32>,
    },
}

/// One key's accepted instants and its WARN bookkeeping.
#[derive(Debug, Default)]
struct Window {
    accepted: VecDeque<Instant>,
    last_warn: Option<Instant>,
    suppressed: u32,
}

impl Window {
    fn forget_older_than(&mut self, now: Instant, window: Duration) {
        while self
            .accepted
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= window)
        {
            self.accepted.pop_front();
        }
    }

    /// Whole seconds (≥ 1) until the oldest accepted instant leaves.
    fn retry_after_secs(&self, now: Instant, window: Duration) -> u64 {
        let wait = self.accepted.front().map_or(Duration::ZERO, |oldest| {
            (*oldest + window).saturating_duration_since(now)
        });
        (wait.as_secs() + u64::from(wait.subsec_nanos() > 0)).max(1)
    }

    fn warn_now(&mut self, now: Instant, window: Duration) -> Option<u32> {
        if self
            .last_warn
            .is_some_and(|at| now.saturating_duration_since(at) < window)
        {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        self.last_warn = Some(now);
        Some(std::mem::take(&mut self.suppressed))
    }

    /// Nothing left to remember: no accepted instant in the window, the WARN
    /// window is over, and no refusal is waiting to be reported — a key
    /// holding suppressed refusals stays until its next WARN carries the count.
    fn is_idle(&self, now: Instant, window: Duration) -> bool {
        self.accepted.is_empty()
            && self.suppressed == 0
            && self
                .last_warn
                .is_none_or(|at| now.saturating_duration_since(at) >= window)
    }
}

/// The limiter behind `InterpreterTipsState` (one per `AppState`).
#[derive(Debug)]
pub(super) struct TipRateLimiter {
    window: Duration,
    per_sender: usize,
    overall_cap: usize,
    senders: HashMap<String, Window>,
    overall: Window,
}

impl TipRateLimiter {
    pub(super) fn new() -> Self {
        Self {
            window: Duration::from_secs(TIP_RATE_WINDOW_SECS),
            per_sender: TIP_RATE_PER_SENDER,
            overall_cap: TIP_RATE_OVERALL,
            senders: HashMap::new(),
            overall: Window::default(),
        }
    }

    /// Count a tip that is about to be stored, or refuse it. A refused tip
    /// is never counted.
    pub(super) fn check(
        &mut self,
        client_ip: &str,
        sender_name: &str,
        now: Instant,
    ) -> RateVerdict {
        let window = self.window;
        self.senders.retain(|_, sender| {
            sender.forget_older_than(now, window);
            !sender.is_idle(now, window)
        });
        self.overall.forget_older_than(now, window);
        let sender = self
            .senders
            .entry(sender_key(client_ip, sender_name))
            .or_default();
        if sender.accepted.len() >= self.per_sender {
            return RateVerdict::Refused {
                scope: RateScope::Sender,
                retry_after_secs: sender.retry_after_secs(now, window),
                warn: sender.warn_now(now, window),
            };
        }
        if self.overall.accepted.len() >= self.overall_cap {
            return RateVerdict::Refused {
                scope: RateScope::Overall,
                retry_after_secs: self.overall.retry_after_secs(now, window),
                warn: self.overall.warn_now(now, window),
            };
        }
        sender.accepted.push_back(now);
        self.overall.accepted.push_back(now);
        RateVerdict::Allowed
    }
}

fn sender_key(client_ip: &str, sender_name: &str) -> String {
    format!("{client_ip}\u{1f}{}", sender_name.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const IP: &str = "203.0.113.9";

    fn secs(base: Instant, n: u64) -> Instant {
        base + Duration::from_secs(n)
    }

    #[test]
    fn the_sixth_tip_within_a_minute_waits_for_the_oldest_to_leave() {
        let mut limiter = TipRateLimiter::new();
        let base = Instant::now();
        for n in 0..5 {
            assert_eq!(
                limiter.check(IP, "Peter", secs(base, n * 10)),
                RateVerdict::Allowed,
                "tip {n}"
            );
        }
        assert_eq!(
            limiter.check(IP, "Peter", secs(base, 50)),
            RateVerdict::Refused {
                scope: RateScope::Sender,
                retry_after_secs: 10,
                warn: Some(0)
            }
        );
        assert_eq!(
            limiter.check(IP, "Peter", secs(base, 60)),
            RateVerdict::Allowed,
            "the first tip left the window; the refused one was never counted"
        );
        assert!(matches!(
            limiter.check(IP, "Peter", secs(base, 61)),
            RateVerdict::Refused {
                scope: RateScope::Sender,
                ..
            }
        ));
    }

    #[test]
    fn the_name_is_part_of_the_key_and_its_case_is_not() {
        let mut limiter = TipRateLimiter::new();
        let now = Instant::now();
        for _ in 0..5 {
            assert_eq!(limiter.check(IP, "Peter", now), RateVerdict::Allowed);
        }
        assert!(matches!(
            limiter.check(IP, "peter", now),
            RateVerdict::Refused {
                scope: RateScope::Sender,
                ..
            }
        ));
        assert_eq!(
            limiter.check(IP, "Jana", now),
            RateVerdict::Allowed,
            "another šepkár behind the same public IP"
        );
        assert_eq!(
            limiter.check("192.0.2.7", "Peter", now),
            RateVerdict::Allowed,
            "the same name on another device"
        );
    }

    #[test]
    fn thirty_tips_a_minute_overall() {
        let mut limiter = TipRateLimiter::new();
        let now = Instant::now();
        for sender in 0..6 {
            for _ in 0..5 {
                assert_eq!(
                    limiter.check(IP, &format!("s{sender}"), now),
                    RateVerdict::Allowed
                );
            }
        }
        assert_eq!(
            limiter.check(IP, "s6", now),
            RateVerdict::Refused {
                scope: RateScope::Overall,
                retry_after_secs: 60,
                warn: Some(0)
            }
        );
    }

    #[test]
    fn one_warn_per_minute_per_key_counting_the_suppressed_ones() {
        let mut window = Window::default();
        let base = Instant::now();
        let minute = Duration::from_secs(60);
        assert_eq!(window.warn_now(base, minute), Some(0));
        assert_eq!(window.warn_now(secs(base, 10), minute), None);
        assert_eq!(window.warn_now(secs(base, 59), minute), None);
        assert_eq!(window.warn_now(secs(base, 60), minute), Some(2));
        assert_eq!(window.warn_now(secs(base, 61), minute), None);
    }

    #[test]
    fn idle_senders_are_forgotten() {
        let mut limiter = TipRateLimiter::new();
        let base = Instant::now();
        assert_eq!(limiter.check(IP, "Peter", base), RateVerdict::Allowed);
        assert_eq!(
            limiter.check(IP, "Jana", secs(base, 61)),
            RateVerdict::Allowed
        );
        assert_eq!(
            limiter.senders.len(),
            1,
            "Peter's window emptied, so his entry is gone"
        );
    }

    #[test]
    fn retry_after_rounds_up_to_whole_seconds() {
        let mut limiter = TipRateLimiter::new();
        let base = Instant::now();
        for _ in 0..5 {
            assert_eq!(limiter.check(IP, "Peter", base), RateVerdict::Allowed);
        }
        let verdict = limiter.check(IP, "Peter", base + Duration::from_millis(59_500));
        assert!(
            matches!(
                verdict,
                RateVerdict::Refused {
                    retry_after_secs: 1,
                    ..
                }
            ),
            "{verdict:?}"
        );
    }

    #[test]
    fn retry_after_counts_a_started_second_as_a_whole_one() {
        let mut limiter = TipRateLimiter::new();
        let base = Instant::now();
        for _ in 0..5 {
            assert_eq!(limiter.check(IP, "Peter", base), RateVerdict::Allowed);
        }
        assert_eq!(
            limiter.check(IP, "Peter", base + Duration::from_millis(50_500)),
            RateVerdict::Refused {
                scope: RateScope::Sender,
                retry_after_secs: 10,
                warn: Some(0)
            },
            "9.5 s left rounds up to 10, not down to 9"
        );
    }

    #[test]
    fn a_sender_with_unreported_refusals_is_not_forgotten() {
        let mut limiter = TipRateLimiter::new();
        let base = Instant::now();
        for _ in 0..5 {
            assert_eq!(limiter.check(IP, "Peter", base), RateVerdict::Allowed);
        }
        assert!(matches!(
            limiter.check(IP, "Peter", secs(base, 1)),
            RateVerdict::Refused { warn: Some(0), .. }
        ));
        assert!(matches!(
            limiter.check(IP, "Peter", secs(base, 2)),
            RateVerdict::Refused { warn: None, .. }
        ));
        assert_eq!(
            limiter.check(IP, "Jana", secs(base, 125)),
            RateVerdict::Allowed
        );
        assert_eq!(
            limiter.senders.len(),
            2,
            "Peter's second refusal was never reported, so his entry stays"
        );
        for _ in 0..5 {
            assert_eq!(
                limiter.check(IP, "Peter", secs(base, 125)),
                RateVerdict::Allowed
            );
        }
        assert!(
            matches!(
                limiter.check(IP, "Peter", secs(base, 126)),
                RateVerdict::Refused { warn: Some(1), .. }
            ),
            "the next WARN carries the refusal that was suppressed before"
        );
    }
}
