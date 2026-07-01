//! Server-side NDI ingest-timing probe (#509 / T0 of the true-latency design
//! `docs/superpowers/specs/2026-06-30-ndi-true-latency-design.md`).
//!
//! A read-only buffer pad probe on the raw NDI frames entering the encoder
//! (post-demux, pre-scale) that periodically logs the frame-ARRIVAL cadence at
//! the server. `ndisrc` runs `timestamp-mode=receive-time` (see
//! `build::build_ndi_source`), so each buffer's PTS IS this server's clock at
//! the instant the NDI frame arrived — the inter-frame PTS gap is exactly the
//! camera→NDI→server delivery cadence.
//!
//! T0 must LOCATE where the recurring "lags/jumps after a while" lives: a smooth
//! ingest cadence here (steady ~33 ms gaps, no large gaps) proves the drift is
//! downstream — server→display (encode/network/jitter-buffer/decode) — while
//! jumpy ingest gaps localize it upstream (camera→NDI→server). T4's metric is a
//! server→display number; this probe is what tells us whether such a number can
//! even explain the complaint.
//!
//! The accounting is a pure, unit-tested accumulator; the pad probe is thin glue
//! that never mutates the buffer and always returns `Ok`.

use std::sync::Mutex;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;

/// Interval between ingest-cadence log lines (media time).
const REPORT_INTERVAL: Duration = Duration::from_secs(5);
/// An inter-arrival gap over this counts as a perceptible ingest hitch.
const GAP_THRESHOLD: Duration = Duration::from_millis(100);

/// Emitted once per interval by [`IngestTimingAccumulator::record`].
#[derive(Debug, Clone, PartialEq)]
pub(super) struct IngestTimingReport {
    /// Frames observed in this interval (both with and without a PTS).
    pub frames: u64,
    /// Subset of `frames` that carried no PTS. `receive-time` mode should make
    /// this 0; a non-zero value is itself a finding (ndisrc not stamping).
    pub frames_no_pts: u64,
    /// Wall span of the interval, from the first→last PTS delta (ms).
    pub span_ms: f64,
    /// Mean arrival rate over the interval (frames / span).
    pub fps: f64,
    /// Largest inter-arrival gap seen this interval (ms). A big value = an
    /// ingest-side stall/jump → the drift is upstream (camera→NDI→server).
    pub max_gap_ms: f64,
    /// Count of inter-arrival gaps over the threshold (perceptible ingest hitches).
    pub gaps_over_threshold: u64,
    /// The gap threshold in effect (ms), for interpreting `gaps_over_threshold`.
    pub gap_threshold_ms: f64,
}

/// Pure accumulator for NDI frame-arrival cadence. Fed one PTS per buffer by the
/// pad probe; returns `Some(report)` (and resets) once an interval's worth of
/// media time has elapsed since the interval's first frame.
pub(super) struct IngestTimingAccumulator {
    interval_ns: u64,
    gap_threshold_ns: u64,
    // --- per-interval state ---
    first_pts: Option<u64>,
    last_pts: Option<u64>,
    frames: u64,
    frames_no_pts: u64,
    max_gap_ns: u64,
    gaps_over_threshold: u64,
}

impl IngestTimingAccumulator {
    pub(super) fn new(interval: Duration, gap_threshold: Duration) -> Self {
        Self {
            interval_ns: interval.as_nanos() as u64,
            gap_threshold_ns: gap_threshold.as_nanos() as u64,
            first_pts: None,
            last_pts: None,
            frames: 0,
            frames_no_pts: 0,
            max_gap_ns: 0,
            gaps_over_threshold: 0,
        }
    }

    /// Record one buffer's PTS (nanoseconds on the server's receive clock).
    /// Returns `Some(report)` once `interval` of media time has elapsed since
    /// the interval's first frame, resetting for the next interval.
    pub(super) fn record(&mut self, pts_ns: Option<u64>) -> Option<IngestTimingReport> {
        self.frames += 1;
        let Some(pts) = pts_ns else {
            // A frame with no PTS can't contribute to timing; count it (a
            // finding in itself) and leave the interval state untouched.
            self.frames_no_pts += 1;
            return None;
        };
        if let Some(last) = self.last_pts {
            let gap = pts.saturating_sub(last);
            self.max_gap_ns = self.max_gap_ns.max(gap);
            if gap > self.gap_threshold_ns {
                self.gaps_over_threshold += 1;
            }
        }
        self.last_pts = Some(pts);
        let first = *self.first_pts.get_or_insert(pts);
        let span = pts.saturating_sub(first);
        if span < self.interval_ns {
            return None;
        }
        let report = IngestTimingReport {
            frames: self.frames,
            frames_no_pts: self.frames_no_pts,
            span_ms: span as f64 / 1_000_000.0,
            fps: if span > 0 {
                self.frames as f64 / (span as f64 / 1_000_000_000.0)
            } else {
                0.0
            },
            max_gap_ms: self.max_gap_ns as f64 / 1_000_000.0,
            gaps_over_threshold: self.gaps_over_threshold,
            gap_threshold_ms: self.gap_threshold_ns as f64 / 1_000_000.0,
        };
        // Reset: the current frame seeds the next interval.
        self.first_pts = Some(pts);
        self.frames = 0;
        self.frames_no_pts = 0;
        self.max_gap_ns = 0;
        self.gaps_over_threshold = 0;
        Some(report)
    }
}

/// Install the read-only ingest-timing pad probe on `pad` (the raw-video pad
/// feeding the encoder). `label` identifies the NDI source in the logs. The
/// probe never mutates the buffer and always returns `Ok` — it must never
/// disturb the live pipeline.
pub(super) fn install_probe(pad: &gst::Pad, label: &str) {
    let acc = Mutex::new(IngestTimingAccumulator::new(REPORT_INTERVAL, GAP_THRESHOLD));
    let label = label.to_string();
    pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
        if let Some(gst::PadProbeData::Buffer(buffer)) = &info.data {
            let pts_ns = buffer.pts().map(|c| c.nseconds());
            if let Ok(mut acc) = acc.lock() {
                if let Some(report) = acc.record(pts_ns) {
                    tracing::info!(
                        ndi_source = %label,
                        frames = report.frames,
                        frames_no_pts = report.frames_no_pts,
                        span_ms = report.span_ms,
                        arrival_fps = report.fps,
                        max_arrival_gap_ms = report.max_gap_ms,
                        arrival_gaps_over_threshold = report.gaps_over_threshold,
                        gap_threshold_ms = report.gap_threshold_ms,
                        "NDI ingest receive-time cadence (server-side; PTS=receive-time)"
                    );
                }
            }
        }
        gst::PadProbeReturn::Ok
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc_5s_100ms() -> IngestTimingAccumulator {
        IngestTimingAccumulator::new(Duration::from_secs(5), Duration::from_millis(100))
    }

    const NS_PER_MS: u64 = 1_000_000;

    #[test]
    fn steady_30fps_reports_smooth_cadence_no_large_gaps() {
        let mut acc = acc_5s_100ms();
        let mut report = None;
        // 6 seconds of steady 30fps frames (33ms apart) — one interval must fire.
        for i in 0..180u64 {
            let pts = i * 33 * NS_PER_MS;
            if let Some(r) = acc.record(Some(pts)) {
                report = Some(r);
                break;
            }
        }
        let r = report.expect("a 5s interval must emit within 6s of frames");
        assert!(
            r.span_ms >= 5000.0,
            "span {} should reach the interval",
            r.span_ms
        );
        assert!(
            (r.fps - 30.0).abs() < 2.0,
            "arrival fps {} should be ~30",
            r.fps
        );
        assert!(
            r.max_gap_ms < 100.0,
            "steady cadence must have no gap over threshold, got max {}",
            r.max_gap_ms
        );
        assert_eq!(r.gaps_over_threshold, 0);
        assert_eq!(r.frames_no_pts, 0);
    }

    #[test]
    fn a_large_arrival_gap_is_flagged() {
        let mut acc = acc_5s_100ms();
        let mut pts = 0u64;
        let mut report = None;
        // Steady frames, but inject one 400ms stall partway through.
        for i in 0..180u64 {
            let step = if i == 60 { 400 } else { 33 };
            pts += step * NS_PER_MS;
            if let Some(r) = acc.record(Some(pts)) {
                report = Some(r);
                break;
            }
        }
        let r = report.expect("interval emits");
        assert!(
            r.max_gap_ms >= 399.0,
            "the 400ms stall must surface as max gap, got {}",
            r.max_gap_ms
        );
        assert!(
            r.gaps_over_threshold >= 1,
            "the stall must count as a gap over the 100ms threshold"
        );
    }

    #[test]
    fn frames_without_pts_are_counted_not_timed() {
        let mut acc = acc_5s_100ms();
        // A frame with no PTS must not panic, must be counted, and must not
        // seed/advance the interval timing.
        assert_eq!(acc.record(None), None);
        let mut report = None;
        for i in 0..180u64 {
            let pts = i * 33 * NS_PER_MS;
            if let Some(r) = acc.record(Some(pts)) {
                report = Some(r);
                break;
            }
        }
        let r = report.expect("interval emits after PTS frames start");
        assert_eq!(
            r.frames_no_pts, 1,
            "the single no-PTS frame is counted once"
        );
    }
}
