//! Pipeline-clock time exposure for the `/ndi/time` HTTP endpoint (#510, T3
//! of the NDI true-latency rework —
//! `docs/superpowers/specs/2026-06-30-ndi-true-latency-design.md` §3).
//!
//! Every encoder pipeline is pinned to `gst::SystemClock::obtain()`
//! (`pipeline/build.rs`) and every consumer pipeline shares that SAME clock
//! instance (`pipeline/consumers.rs`). `rtpbin`'s `ntp-time-source` is set to
//! `clock-time`, so the RTCP Sender Reports encode this exact clock's raw
//! value — NOT `SystemTime::now()` wall-clock, NOT an NTP-disciplined time.
//! `pipeline_clock_now_ms()` reads that SAME process-wide clock singleton, so
//! a browser's NTP-style round trip against `/ndi/time` yields an offset in
//! the SAME domain the SR timestamps live in — no absolute wall-clock sync
//! required for the offset to be meaningful (design §3 "why this drops
//! dantesync off the critical path").

use anyhow::Result;
use gstreamer as gst;
use gstreamer::prelude::*;

/// Current server pipeline-clock time, in milliseconds, as an `f64` (matching
/// the browser's `performance.now()` / `DOMHighResTimeStamp` scale so the two
/// are directly comparable in an NTP-style round-trip calculation).
///
/// Calls `presenter_ndi::init()` first (idempotent, cheap after the first
/// process-wide call — see `lib.rs`) so this works even before any NDI
/// pipeline has ever been built: `gst::SystemClock::obtain()` requires
/// `gstreamer::init()` to have run at least once, but needs no pipeline, no
/// encoder, and no NDI source to return a valid, monotonically advancing time.
pub fn pipeline_clock_now_ms() -> Result<f64> {
    crate::init()?;
    let clock = gst::SystemClock::obtain();
    Ok(clock.time().nseconds() as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_clock_now_ms_is_finite_and_monotonic() {
        let t1 = pipeline_clock_now_ms().expect("first pipeline clock read");
        assert!(
            t1.is_finite() && t1 >= 0.0,
            "pipeline clock must be a finite, non-negative ms value, got {t1}"
        );
        let t2 = pipeline_clock_now_ms().expect("second pipeline clock read");
        assert!(
            t2 >= t1,
            "pipeline clock must never go backwards: t1={t1} t2={t2}"
        );
    }
}
