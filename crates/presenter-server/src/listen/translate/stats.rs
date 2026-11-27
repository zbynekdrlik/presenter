//! What a guest translation costs and how it performs (#999 L1): the daily
//! minute cap shared by every language ([`DayMeter`]) and one session's
//! per-minute summary ([`MinuteStats`], the R2 metrics: listeners, the
//! first-voiced latency after the preacher starts again, stalls).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::NaiveDate;

use super::TARGET;

/// Audio minutes sent to Gemini per local day, summed over every language
/// (~$0.037 per language-minute: 480 min ≈ $18 — two long services with
/// four languages). A constant on purpose (owner: no setting needed); the
/// count lives in memory, so a restart starts the day's count again.
pub const DAILY_CAP_MINUTES: u64 = 480;

/// The day's sent audio, shared by every session.
#[derive(Clone, Default)]
pub struct DayMeter {
    inner: Arc<Mutex<(Option<NaiveDate>, u64)>>,
}

impl DayMeter {
    /// Counts `ms` of audio sent on `today`; `false` once the day's cap is
    /// reached (the audio is then NOT sent).
    pub fn spend(&self, today: NaiveDate, ms: u64) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.0 != Some(today) {
            *guard = (Some(today), 0);
        }
        if guard.1 >= DAILY_CAP_MINUTES * 60_000 {
            return false;
        }
        guard.1 = guard.1.saturating_add(ms);
        true
    }

    /// Whether `today`'s cap is already reached.
    pub fn exhausted(&self, today: NaiveDate) -> bool {
        let guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        guard.0 == Some(today) && guard.1 >= DAILY_CAP_MINUTES * 60_000
    }

    /// Minutes counted for `today`.
    pub fn minutes(&self, today: NaiveDate) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.0 == Some(today) {
            guard.1 / 60_000
        } else {
            0
        }
    }
}

/// One session's counters since the last summary.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MinuteStats {
    pub sent_ms: u64,
    pub output_ms: u64,
    pub output_chars: u64,
    /// Speech starts after a pause (the moments the room reacts to).
    pub onsets: u64,
    /// Speech start → the first translated SPEECH (output above
    /// `session::OUTPUT_VOICED_DBFS`) after the model's own pause, per onset
    /// (#999 L6a; the model streams its silence too, and the speech before
    /// its pause is the previous phrase's backlog).
    pub first_voiced_ms: Vec<u64>,
    /// Translated audio stopped for [`STALL_AFTER`] while the preacher spoke.
    pub stalls: u64,
    pub reconnects: u64,
    pub dropped_frames: u64,
    /// A replaced connection's speech dropped because the new one already spoke.
    pub overlap_dropped_ms: u64,
}

/// No translated audio this long while the preacher speaks = a stall (the
/// model's steady lag is ~3–4 s, R2).
pub const STALL_AFTER: Duration = Duration::from_secs(6);

impl MinuteStats {
    /// The minute's INFO line, then the counters start again. A minute with
    /// nothing sent and nothing played logs nothing (a silent mic all day).
    pub fn log_and_reset(&mut self, language: &str, listeners: usize, connected: bool) {
        if self.sent_ms > 0
            || self.output_ms > 0
            || self.stalls > 0
            || self.reconnects > 0
            || self.dropped_frames > 0
        {
            let mut latencies = self.first_voiced_ms.clone();
            latencies.sort_unstable();
            let median = latencies.get(latencies.len() / 2).copied();
            tracing::info!(
                target: TARGET,
                language,
                listeners,
                connected,
                sent_ms = self.sent_ms,
                output_ms = self.output_ms,
                output_chars = self.output_chars,
                onsets = self.onsets,
                first_voiced_timed = latencies.len(),
                first_voiced_ms_median = median,
                first_voiced_ms_max = latencies.last().copied(),
                stalls = self.stalls,
                reconnects = self.reconnects,
                dropped_frames = self.dropped_frames,
                overlap_dropped_ms = self.overlap_dropped_ms,
                "guest translation minute summary"
            );
        }
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).expect("date")
    }

    #[test]
    fn the_cap_stops_spending_and_a_new_day_starts_at_zero() {
        let meter = DayMeter::default();
        let cap_ms = DAILY_CAP_MINUTES * 60_000;
        assert!(meter.spend(day(27), cap_ms - 100));
        assert!(!meter.exhausted(day(27)));
        assert!(meter.spend(day(27), 100));
        assert!(meter.exhausted(day(27)));
        assert!(!meter.spend(day(27), 100), "nothing more today");
        assert_eq!(meter.minutes(day(27)), DAILY_CAP_MINUTES);
        assert!(!meter.exhausted(day(28)));
        assert!(meter.spend(day(28), 100));
        assert_eq!(meter.minutes(day(28)), 0);
        assert_eq!(meter.minutes(day(27)), 0, "only today's count is kept");
    }

    #[test]
    fn a_summary_resets_the_counters() {
        let mut stats = MinuteStats {
            sent_ms: 60_000,
            first_voiced_ms: vec![3_400, 3_100, 5_000],
            stalls: 1,
            ..MinuteStats::default()
        };
        stats.log_and_reset("en", 2, true);
        assert_eq!(stats, MinuteStats::default());
    }
}
