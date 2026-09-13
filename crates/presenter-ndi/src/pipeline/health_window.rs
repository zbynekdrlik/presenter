//! Trailing-30s window NDI fan-out delivery health (#768 D3).
//!
//! Lane 1 (`health.rs`) exposes the drop ratio + pushed FPS **cumulative since
//! the counter's origin** — right for the post-deploy self-heal gate (a freshly
//! built pipeline, where cumulative == recent) but it DILUTES a mid-life
//! degradation on a long-lived pipeline: a 6-hour healthy session that then
//! drops 75% for five minutes reports a cumulative ratio near 0.0, so the
//! CURRENT health an external watchdog (airuleset#1005 polls
//! `/healthz.ndi_pipelines[]`) needs stays invisible. This module adds a small
//! trailing window so the ratio reflects the last ~30s, not all of history.
//!
//! Design (see the #768 D3 design comment):
//!   - One [`HealthWindow`] per consumer link, holding a bounded ring of
//!     `(Instant, cumulative_pushed, cumulative_dropped)` samples. A single
//!     link's counters are strictly monotonic, so its windowed delta is always
//!     non-negative.
//!   - The per-PIPELINE 30s value is folded from each LIVE session's own
//!     (monotonic) windowed delta via [`aggregate_windowed`], never from the
//!     raw summed counters across sessions — a summed counter DECREASES when a
//!     consumer leaves, which would produce a negative delta and a garbage
//!     ratio. Folding per-session deltas cannot.
//!   - Sampling is READ-DRIVEN (no dedicated thread): `record` is called on the
//!     existing `/ndi/snapshot` and `/healthz` read paths, min-spacing-gated so
//!     the many pollers that hit `/healthz` do not oversample. The metric is
//!     `null` until >= 2 in-window samples exist, and a poller slower than the
//!     window (> ~15s) will read `null` (you cannot measure a 30s window
//!     sampling slower than it — the airuleset#1005 watchdog polls well under
//!     that, and operator tabs keep `/healthz` warm continuously).
//!
//! Everything here is pure and side-effect-free (no libndi/GPU), so it is
//! unit-testable on every CI host with synthetic `Instant`s — the Tier-0 seam
//! discipline of `ndi-manager-locking.md`.

use std::time::{Duration, Instant};

use super::health::{drop_ratio, pushed_fps};

/// Length of the trailing window. 30s is short enough to surface a five-minute
/// mid-life drop episode while long enough to smooth per-GOP jitter.
pub const WINDOW: Duration = Duration::from_secs(30);

/// Minimum spacing between recorded samples on the read-driven path. `/healthz`
/// is polled by every operator tab plus the reload guard plus the deploy gates;
/// this dedups those hits so the ring stays small and the samples span the
/// window rather than clustering in one instant.
pub const MIN_SAMPLE_SPACING: Duration = Duration::from_secs(2);

/// Hard safety cap on retained samples. Eviction-by-age already bounds the ring
/// to `WINDOW / MIN_SAMPLE_SPACING` (~16); this is a belt against a pathological
/// clock so the `Vec` can never grow without bound.
const MAX_SAMPLES: usize = 32;

/// One `(timestamp, cumulative pushed, cumulative dropped)` reading of a single
/// consumer link's delivery counters.
#[derive(Debug, Clone, Copy)]
struct Sample {
    at: Instant,
    pushed: u64,
    dropped: u64,
}

/// A bounded ring of delivery-counter samples for ONE consumer link, from which
/// a trailing-window drop ratio + pushed FPS are derived.
#[derive(Debug, Default)]
pub struct HealthWindow {
    samples: Vec<Sample>,
}

impl HealthWindow {
    /// Record a sample of the link's cumulative counters at `now`, IF at least
    /// [`MIN_SAMPLE_SPACING`] has elapsed since the last recorded sample (a
    /// too-soon call is a no-op — read-driven sampling dedups many pollers).
    /// Then evict every sample strictly older than [`WINDOW`] relative to `now`.
    pub fn record(&mut self, now: Instant, pushed: u64, dropped: u64) {
        if let Some(last) = self.samples.last() {
            if now.duration_since(last.at) < MIN_SAMPLE_SPACING {
                return;
            }
        }
        self.samples.push(Sample {
            at: now,
            pushed,
            dropped,
        });
        // Keep only in-window samples (retains the newest even if it is the
        // only one left — `windowed_delta` returns None below 2 samples).
        self.samples.retain(|s| now.duration_since(s.at) <= WINDOW);
        if self.samples.len() > MAX_SAMPLES {
            let excess = self.samples.len() - MAX_SAMPLES;
            self.samples.drain(0..excess);
        }
    }

    /// The delta from the oldest in-window sample to the newest:
    /// `(pushed_delta, dropped_delta, secs)`. `None` with fewer than 2 samples
    /// or a non-positive span. Deltas use `saturating_sub` as a belt — a single
    /// link's counters are monotonic, so this only guards a counter reset.
    pub fn windowed_delta(&self) -> Option<(u64, u64, f64)> {
        if self.samples.len() < 2 {
            return None;
        }
        let newest = self.samples.last()?;
        let oldest = self.samples.first()?;
        let secs = newest.at.duration_since(oldest.at).as_secs_f64();
        if secs <= 0.0 {
            return None;
        }
        Some((
            newest.pushed.saturating_sub(oldest.pushed),
            newest.dropped.saturating_sub(oldest.dropped),
            secs,
        ))
    }

    /// Trailing-window drop ratio (`dropped_delta / (pushed+dropped)_delta`),
    /// reusing lane 1's [`drop_ratio`]. `None` until >= 2 in-window samples.
    pub fn drop_ratio_30s(&self) -> Option<f64> {
        let (pd, dd, _secs) = self.windowed_delta()?;
        Some(drop_ratio(pd, dd))
    }

    /// Trailing-window pushed FPS (`pushed_delta / secs`), reusing lane 1's
    /// [`pushed_fps`]. `None` until >= 2 in-window samples.
    pub fn pushed_fps_30s(&self) -> Option<f64> {
        let (pd, _dd, secs) = self.windowed_delta()?;
        Some(pushed_fps(pd, secs))
    }
}

/// Fold per-consumer windowed deltas (each the output of
/// [`HealthWindow::windowed_delta`]) into the per-PIPELINE trailing-window
/// metrics `(drop_ratio_30s, pushed_fps_30s)`. Both are `None` when NO live
/// session has a window yet. Folding per-session monotonic deltas — never the
/// raw summed counters — is what keeps the aggregate correct when a consumer
/// leaves (module docs). The pipeline FPS is the SUM of the consumers' FPS
/// (total frames/s the pipeline is fanning out).
pub fn aggregate_windowed<I>(per_session: I) -> (Option<f64>, Option<f64>)
where
    I: IntoIterator<Item = Option<(u64, u64, f64)>>,
{
    let mut any = false;
    let mut sum_pushed = 0u64;
    let mut sum_dropped = 0u64;
    let mut sum_fps = 0.0f64;
    for delta in per_session {
        if let Some((pd, dd, secs)) = delta {
            any = true;
            sum_pushed = sum_pushed.saturating_add(pd);
            sum_dropped = sum_dropped.saturating_add(dd);
            sum_fps += pushed_fps(pd, secs);
        }
    }
    if any {
        (
            Some(drop_ratio(sum_pushed, sum_dropped)),
            Some((sum_fps * 10.0).round() / 10.0),
        )
    } else {
        (None, None)
    }
}

/// Cheap per-pipeline delivery totals for the `/healthz` path (#768): the
/// cumulative counters + consumer count PLUS the trailing-30s aggregate (D3).
/// Returned by `NdiPipeline::consumer_delivery_totals` — no network, pure
/// counter reads, so `/healthz` stays cheap (`ai-health-endpoint.md`).
#[derive(Debug, Clone, Copy)]
pub struct PipelineDeliveryTotals {
    pub pushed: u64,
    pub dropped: u64,
    pub consumers: usize,
    pub drop_ratio_30s: Option<f64>,
    pub pushed_fps_30s: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Instant {
        Instant::now()
    }

    #[test]
    fn empty_window_is_null() {
        let w = HealthWindow::default();
        assert_eq!(w.drop_ratio_30s(), None);
        assert_eq!(w.pushed_fps_30s(), None);
        assert_eq!(w.windowed_delta(), None);
    }

    #[test]
    fn one_sample_is_null() {
        let mut w = HealthWindow::default();
        w.record(base(), 100, 0);
        assert_eq!(w.drop_ratio_30s(), None);
        assert_eq!(w.pushed_fps_30s(), None);
    }

    #[test]
    fn steady_thirty_fps_zero_drops_is_ratio_zero() {
        // 30 fps, no drops: pushed grows 150 per 5s, dropped stays 0.
        let b = base();
        let mut w = HealthWindow::default();
        w.record(b, 0, 0);
        w.record(b + Duration::from_secs(5), 150, 0);
        w.record(b + Duration::from_secs(10), 300, 0);
        assert_eq!(w.drop_ratio_30s(), Some(0.0));
        assert_eq!(w.pushed_fps_30s(), Some(30.0));
    }

    #[test]
    fn min_spacing_suppresses_a_too_soon_sample() {
        let b = base();
        let mut w = HealthWindow::default();
        w.record(b, 0, 0);
        // < 2s later — ignored, still one sample → null.
        w.record(b + Duration::from_millis(1500), 45, 0);
        assert_eq!(w.drop_ratio_30s(), None, "second sample must be suppressed");
        // >= 2s later — recorded, now two samples → a value.
        w.record(b + Duration::from_secs(3), 90, 0);
        assert_eq!(w.drop_ratio_30s(), Some(0.0));
        assert_eq!(w.pushed_fps_30s(), Some(30.0)); // 90 pushed over 3s
    }

    #[test]
    fn eviction_bounds_the_window_to_thirty_seconds() {
        // Record every 5s for 60s; only the last ~30s stay in the window.
        let b = base();
        let mut w = HealthWindow::default();
        for k in 0..=12u64 {
            w.record(b + Duration::from_secs(5 * k), 150 * k, 0);
        }
        let (_pd, _dd, secs) = w.windowed_delta().expect("has a window");
        assert!(
            secs <= WINDOW.as_secs_f64(),
            "windowed span {secs}s must be <= 30s after eviction"
        );
    }

    #[test]
    fn mid_life_episode_reads_current_ratio_not_diluted_history() {
        // The whole point of the trailing window (#768 D3): a long healthy run
        // followed by a 75% drop episode must read ~0.75 once the window fills
        // with episode samples — where the CUMULATIVE ratio would still be ~0.0.
        let b = base();
        let mut w = HealthWindow::default();
        // Healthy phase t=0..35s (30 fps, 0 drops).
        for k in 0..=7u64 {
            w.record(b + Duration::from_secs(5 * k), 150 * k, 0);
        }
        // 75% drop episode t=40..70s: per 5s add 40 pushed + 120 dropped
        // (120 / 160 = 0.75). Cumulative starts from the healthy tail (1050, 0).
        let mut pushed = 1050u64;
        let mut dropped = 0u64;
        for j in 1..=7u64 {
            pushed += 40;
            dropped += 120;
            w.record(b + Duration::from_secs(35 + 5 * j), pushed, dropped);
        }
        // The window now holds only episode samples (t=40..70) — healthy ones
        // (t <= 35) are > 30s older than t=70 and were evicted.
        assert_eq!(
            w.drop_ratio_30s(),
            Some(0.75),
            "trailing window must report the CURRENT episode, not diluted history"
        );

        // Recovery: healthy again t=75..105 → episode samples roll out → 0.0.
        let mut pushed = 1330u64; // episode tail pushed (1050 + 40*7)
        for j in 1..=7u64 {
            pushed += 150; // 30 fps, no new drops
            w.record(b + Duration::from_secs(70 + 5 * j), pushed, dropped);
        }
        assert_eq!(
            w.drop_ratio_30s(),
            Some(0.0),
            "once the episode rolls past the window, the ratio returns to 0.0"
        );
    }

    #[test]
    fn aggregate_is_null_when_no_session_has_a_window() {
        let (dr, fps) = aggregate_windowed([None, None]);
        assert_eq!(dr, None);
        assert_eq!(fps, None);
    }

    #[test]
    fn aggregate_folds_per_session_deltas() {
        // Session A: 150 pushed / 0 dropped over 5s (30 fps healthy).
        // Session B: 30 pushed / 90 dropped over 5s (6 fps, 0.75 drop).
        // Pipeline: dropped 90 of (180+90=270) = 0.3333; fps 30 + 6 = 36.0.
        let (dr, fps) = aggregate_windowed([Some((150, 0, 5.0)), Some((30, 90, 5.0))]);
        assert_eq!(dr, Some(0.3333));
        assert_eq!(fps, Some(36.0));
    }

    #[test]
    fn aggregate_ignores_sessions_without_a_window() {
        // One session with a window, one without — the None is skipped.
        let (dr, fps) = aggregate_windowed([Some((300, 0, 10.0)), None]);
        assert_eq!(dr, Some(0.0));
        assert_eq!(fps, Some(30.0));
    }
}
