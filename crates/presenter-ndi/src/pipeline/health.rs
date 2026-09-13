//! Cheap, pure NDI fan-out delivery metrics (#768).
//!
//! The 2026-09-13 SNV incident (a boot-restore pipeline dropping ~75% of every
//! consumer's encoded frames in the `StreamProducer`→appsrc bridge) was
//! diagnosable ONLY by hand-reading raw `buffersPushed`/`buffersDropped`
//! counters off `/ndi/snapshot` and computing the ratio in one's head — nothing
//! about the drop rate was exposed as a single actionable signal, and NOTHING
//! about it left the box (`/healthz` carried only pipeline `state`). This module
//! turns those raw counters into two derived metrics — a **drop ratio** and a
//! **pushed FPS** — so both `/ndi/snapshot/{id}` (per session + per pipeline)
//! and `/healthz.ndi_pipelines[]` (per pipeline) carry them, and the post-deploy
//! self-heal gate can read the ratio and rebuild a sick pipeline automatically.
//!
//! Both metrics are **cumulative since the counter's origin** (a session since
//! it joined; a pipeline aggregate over its live sessions). That is exactly
//! right for the two consumers here: a pipeline that has been sick since birth
//! (the boot-restore case) reports its full ratio, and the post-deploy gate
//! reads a freshly-built pipeline where cumulative == recent. A trailing-window
//! variant is a possible later refinement, not needed for either case.
//!
//! The functions are pure and side-effect-free so they are unit-testable on
//! every CI host without libndi/GPU (this crate is Tier-0, no local compile —
//! per `ndi-manager-locking.md` a pure seam is the pre-CI safety net).

use super::PipelineState;

/// Fraction (0.0–1.0) of encoded frames dropped in the `StreamProducer`→appsrc
/// bridge, cumulative since the counters' origin. `0.0` when nothing has flowed
/// yet (avoids a 0/0 NaN that would poison the JSON and the deploy-gate `jq`).
/// Rounded to 4 decimals for a clean, stable API value.
pub fn drop_ratio(pushed: u64, dropped: u64) -> f64 {
    let total = pushed.saturating_add(dropped);
    if total == 0 {
        return 0.0;
    }
    round_to(dropped as f64 / total as f64, 10_000.0)
}

/// Average frames/s forwarded to a consumer since it joined (`pushed / age`).
/// `0.0` for a non-positive age (a just-created session, or a clock anomaly) so
/// the value is always finite. Rounded to 1 decimal (fps needs no more).
pub fn pushed_fps(pushed: u64, age_secs: f64) -> f64 {
    if age_secs <= 0.0 {
        return 0.0;
    }
    round_to(pushed as f64 / age_secs, 10.0)
}

/// Round `x` to the precision implied by `scale` (e.g. `10_000.0` → 4 decimals).
fn round_to(x: f64, scale: f64) -> f64 {
    (x * scale).round() / scale
}

/// Per-pipeline delivery health for `/healthz.ndi_pipelines[]` — the cheap
/// aggregate (state + summed counters over the pipeline's live consumers), read
/// WITHOUT the per-session RTCP get-stats round trip that `PipelineSnapshot`
/// does (that endpoint is polled by every operator tab + the stage reload guard
/// + the deploy gates, so it must stay cheap — the `ai-health-endpoint.md`
/// shared-computation discipline).
#[derive(Debug, Clone)]
pub struct PipelineDropHealth {
    pub source_id: String,
    pub state: PipelineState,
    /// Cumulative drop ratio aggregated over the pipeline's live consumers.
    pub drop_ratio: f64,
    /// Number of live WHEP consumers on this pipeline.
    pub consumers: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_ratio_healthy_pipeline_is_zero() {
        // 30 fps for 2 minutes, nothing dropped — the API-activate healthy case.
        assert_eq!(drop_ratio(3600, 0), 0.0);
    }

    #[test]
    fn drop_ratio_matches_the_incident_75_percent() {
        // The #768 incident: ~15 frames pushed + ~45 dropped per 2s GOP =
        // exactly 0.75 (25% pushed). This is the value the self-heal gate trips on.
        assert_eq!(drop_ratio(15, 45), 0.75);
    }

    #[test]
    fn drop_ratio_no_traffic_is_zero_not_nan() {
        // A just-joined consumer (0 pushed, 0 dropped) must be 0.0, never NaN —
        // a NaN would break serde_json AND the deploy gate's `jq` comparison.
        let r = drop_ratio(0, 0);
        assert_eq!(r, 0.0);
        assert!(r.is_finite());
    }

    #[test]
    fn drop_ratio_all_dropped_is_one() {
        assert_eq!(drop_ratio(0, 100), 1.0);
    }

    #[test]
    fn drop_ratio_is_rounded_to_four_decimals() {
        // 1/3 → 0.3333 (not 0.33333333…), a stable clean API value.
        assert_eq!(drop_ratio(2, 1), 0.3333);
    }

    #[test]
    fn pushed_fps_healthy_is_thirty() {
        // 3600 frames over 120s = 30 fps.
        assert_eq!(pushed_fps(3600, 120.0), 30.0);
    }

    #[test]
    fn pushed_fps_incident_is_about_ten() {
        // The incident: ~1140 frames over 120s ≈ 9.5 fps (the ~10 fps the
        // owner saw on the stage TVs instead of 30).
        assert_eq!(pushed_fps(1140, 120.0), 9.5);
    }

    #[test]
    fn pushed_fps_zero_age_is_zero_not_infinity() {
        // A brand-new session (age 0) must be 0.0, never inf/NaN.
        let f = pushed_fps(100, 0.0);
        assert_eq!(f, 0.0);
        assert!(f.is_finite());
    }
}
