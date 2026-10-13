//! Time-windowed min/max store (#854).
//!
//! A fixed ring of one-second buckets, each holding the min and max value
//! recorded during that second. The extrema over the trailing N seconds are
//! read from the buckets whose second still falls inside the span, so the
//! result depends only on WHEN values were recorded — never on who reads it or
//! how often (unlike the read-driven `health_window`, which samples on the
//! read path and needs >= 2 spaced reads).
//!
//! Two users:
//!   - the encoder-side lead meter (`encoder_timing.rs`), which records how far
//!     each shared-encoder output buffer is stamped ahead of the clock;
//!   - the per-consumer link probe (`link_probe.rs`), which records each
//!     forwarded buffer's lateness for the `futureTimestamps` verdict.
//!
//! Pure data, no I/O, no clock reads: callers pass the second index, so it is
//! unit-tested with plain integers on any CI host (Tier-0 seam discipline).

/// Number of one-second buckets. 32 > the 30 s window, so a full window is
/// always available without a bucket being reused inside it.
const BUCKETS: usize = 32;

/// One second's extrema.
#[derive(Debug, Clone, Copy)]
struct Bucket {
    sec: u64,
    min: i64,
    max: i64,
}

/// Ring of [`BUCKETS`] one-second min/max buckets keyed by `sec % BUCKETS`.
/// A bucket holding an older second is overwritten when its slot comes round
/// again, so memory stays fixed regardless of how long the store lives.
#[derive(Debug)]
pub(crate) struct SecondBuckets {
    buckets: [Option<Bucket>; BUCKETS],
}

impl Default for SecondBuckets {
    fn default() -> Self {
        Self {
            buckets: [None; BUCKETS],
        }
    }
}

impl SecondBuckets {
    /// Record `value` in the bucket for second `sec`. A slot still holding a
    /// different (older) second is reset first.
    pub(crate) fn record(&mut self, sec: u64, value: i64) {
        let idx = (sec % BUCKETS as u64) as usize;
        if let Some(bucket) = self.buckets[idx].as_mut().filter(|b| b.sec == sec) {
            bucket.min = bucket.min.min(value);
            bucket.max = bucket.max.max(value);
            return;
        }
        self.buckets[idx] = Some(Bucket {
            sec,
            min: value,
            max: value,
        });
    }

    /// `(min, max)` over the seconds `now_sec - span_secs + 1 ..= now_sec`
    /// (the current, partial second included). `None` when nothing was
    /// recorded inside the span. `span_secs` is capped at the ring size.
    pub(crate) fn extrema(&self, now_sec: u64, span_secs: u64) -> Option<(i64, i64)> {
        let span = span_secs.min(BUCKETS as u64);
        let mut out: Option<(i64, i64)> = None;
        for bucket in self.buckets.iter().flatten() {
            if bucket.sec > now_sec || now_sec - bucket.sec >= span {
                continue;
            }
            out = Some(match out {
                None => (bucket.min, bucket.max),
                Some((lo, hi)) => (lo.min(bucket.min), hi.max(bucket.max)),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_has_no_extrema() {
        assert_eq!(SecondBuckets::default().extrema(10, 30), None);
    }

    #[test]
    fn extrema_cover_every_value_inside_the_span() {
        let mut w = SecondBuckets::default();
        w.record(100, 5);
        w.record(100, -7);
        w.record(110, 40);
        w.record(129, 12);
        assert_eq!(w.extrema(129, 30), Some((-7, 40)));
    }

    #[test]
    fn values_age_out_of_the_span() {
        let mut w = SecondBuckets::default();
        w.record(100, -1500);
        w.record(120, 33);
        // Second 100 is 30 s before 130: outside a 30 s span (130-29 ..= 130).
        assert_eq!(w.extrema(130, 30), Some((33, 33)));
        // At 129 it is still inside (129-29 = 100).
        assert_eq!(w.extrema(129, 30), Some((-1500, 33)));
        // Everything aged out.
        assert_eq!(w.extrema(200, 30), None);
    }

    #[test]
    fn a_reused_slot_drops_the_old_second() {
        let mut w = SecondBuckets::default();
        w.record(3, -900);
        // Second 35 maps to the same slot as 3 (35 % 32 == 3): the old value
        // must not leak into the new second's bucket.
        w.record(35, 20);
        assert_eq!(w.extrema(35, 30), Some((20, 20)));
        assert_eq!(w.extrema(35, 32), Some((20, 20)));
    }

    #[test]
    fn future_buckets_are_ignored() {
        let mut w = SecondBuckets::default();
        w.record(50, 7);
        assert_eq!(w.extrema(49, 30), None);
        assert_eq!(w.extrema(50, 30), Some((7, 7)));
    }

    #[test]
    fn span_is_capped_at_the_ring_size() {
        let mut w = SecondBuckets::default();
        w.record(0, 1);
        w.record(40, 2);
        // A 1000 s span reads at most 32 seconds back: second 0 is gone.
        assert_eq!(w.extrema(40, 1000), Some((2, 2)));
    }

    #[test]
    fn one_second_span_reads_only_the_current_second() {
        let mut w = SecondBuckets::default();
        w.record(9, -50);
        w.record(10, 4);
        assert_eq!(w.extrema(10, 1), Some((4, 4)));
    }
}
