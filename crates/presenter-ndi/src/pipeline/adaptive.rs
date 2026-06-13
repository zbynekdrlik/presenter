//! Pure AIMD bitrate controller for the compat (weak-TV) VP8 stream — RED stub.
//!
//! Stub returns START unchanged so the test module compiles; the behavioural
//! tests fail until the real algorithm lands (RED → GREEN).

/// Minimum encoder target bitrate (bits/s) — safety floor, never the resting
/// state under the quality policy.
pub const MIN_BPS: i32 = 200_000;
/// Maximum encoder target bitrate (bits/s) — the compat 480p ceiling and the
/// start-high value.
pub const MAX_BPS: i32 = 900_000;
/// Initial bitrate — start HIGH per the quality policy.
pub const START_BPS: i32 = MAX_BPS;

/// EWMA weight on the newest observation (`ewma = 0.35*prev + 0.65*observed`).
/// VDO.Ninja's loss-smoothing constant.
const EWMA_ALPHA: f64 = 0.65;
/// Smoothed loss above this fraction (>2 %) is treated as congestion → decrease.
const LOSS_HIGH: f64 = 0.02;
/// Smoothed loss below this fraction (<0.5 %) signals headroom → probe up.
const LOSS_LOW: f64 = 0.005;
/// Multiplicative-decrease factor applied on congestion.
const MD_FACTOR: f64 = 0.85;
/// Additive-increase step (bits/s) applied when probing up.
const AI_STEP_BPS: i32 = 50_000;
/// Minimum seconds between two increases (don't probe up too eagerly).
const INCREASE_MIN_INTERVAL_S: f64 = 10.0;
/// Seconds to hold after a decrease before any increase (anti-thrash).
const DECREASE_COOLDOWN_S: f64 = 5.0;

/// Per-consumer adaptive bitrate controller for the compat (weak-TV) VP8
/// stream. Pure AIMD over EWMA packet loss — the homegrown equivalent of
/// libwebrtc GCC (rtpgccbwe is unavailable on our hosts). Bitrate-only:
/// resolution is FIXED (live resolution change triggers the decoder
/// port-reconfig that kills the Vestel OMX — addendum 2). See #387.
pub struct CompatBitrateController {
    bitrate: i32,
}

/// Result of one [`CompatBitrateController::update`] call.
pub struct BitrateDecision {
    /// The (possibly unchanged) target bitrate the caller should apply.
    pub target_bitrate_bps: i32,
    /// `true` if the caller should apply a new value to `vp8enc`.
    pub changed: bool,
}

impl CompatBitrateController {
    /// Start at MAX (quality policy: start high).
    pub fn new() -> Self {
        Self { bitrate: START_BPS }
    }

    /// Feed one observation. RED stub — no adaptation yet.
    pub fn update(&mut self, _observed_loss: f64, _rtt_ms: f64, _dt_secs: f64) -> BitrateDecision {
        BitrateDecision {
            target_bitrate_bps: self.bitrate,
            changed: false,
        }
    }

    /// Current target bitrate in bits/s.
    pub fn current_bitrate_bps(&self) -> i32 {
        self.bitrate
    }
}

impl Default for CompatBitrateController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- shared observation helpers (not #[test] — exempt from integrity) ---

    /// Loss fraction representing a CLEAN link (well below LOSS_LOW=0.005).
    const CLEAN: f64 = 0.0;
    /// Sustained loss that unambiguously exceeds LOSS_HIGH at EWMA steady state
    /// (0.05 observed → ewma converges to 0.05 > 0.02).
    const HEAVY: f64 = 0.05;
    /// Stable RTT used everywhere — RTT does not drive v1 decisions.
    const RTT: f64 = 30.0;

    /// Feed `ticks` updates of `loss` at `dt` seconds each; return last decision.
    fn run(c: &mut CompatBitrateController, loss: f64, dt: f64, ticks: usize) -> BitrateDecision {
        let mut last = c.update(loss, RTT, dt);
        for _ in 1..ticks {
            last = c.update(loss, RTT, dt);
        }
        last
    }

    #[test]
    fn starts_at_max_before_any_update() {
        let c = CompatBitrateController::new();
        assert_eq!(c.current_bitrate_bps(), MAX_BPS);
        assert_eq!(MAX_BPS, 900_000);
        assert_eq!(START_BPS, MAX_BPS);
    }

    #[test]
    fn sustained_loss_drives_monotonically_down_to_exactly_min_and_clamps() {
        let mut c = CompatBitrateController::new();
        let mut prev = c.current_bitrate_bps();
        // 1 s ticks; many ticks of heavy loss.
        for _ in 0..200 {
            let d = c.update(HEAVY, RTT, 1.0);
            // Never below the floor, never increases under loss.
            assert!(d.target_bitrate_bps >= MIN_BPS);
            assert!(d.target_bitrate_bps <= prev);
            prev = d.target_bitrate_bps;
        }
        assert_eq!(c.current_bitrate_bps(), MIN_BPS);
    }

    #[test]
    fn clamped_at_min_reports_unchanged() {
        let mut c = CompatBitrateController::new();
        run(&mut c, HEAVY, 1.0, 200);
        assert_eq!(c.current_bitrate_bps(), MIN_BPS);
        // Already at floor: another heavy tick must not move it and must report
        // changed=false.
        let d = c.update(HEAVY, RTT, 1.0);
        assert_eq!(d.target_bitrate_bps, MIN_BPS);
        assert!(!d.changed);
    }

    #[test]
    fn clean_link_from_start_does_not_increase_above_max() {
        let mut c = CompatBitrateController::new();
        // Already at MAX; clean link for a long time must not exceed MAX.
        for _ in 0..100 {
            let d = c.update(CLEAN, RTT, 1.0);
            assert_eq!(d.target_bitrate_bps, MAX_BPS);
            assert!(!d.changed);
        }
        assert_eq!(c.current_bitrate_bps(), MAX_BPS);
    }

    #[test]
    fn lowered_state_recovers_up_to_exactly_max_and_clamps() {
        let mut c = CompatBitrateController::new();
        run(&mut c, HEAVY, 1.0, 200); // drive to MIN
        assert_eq!(c.current_bitrate_bps(), MIN_BPS);
        // Clean link; large dt so cooldown + increase-interval are satisfied
        // every tick. Probe up in AI_STEP increments to exactly MAX, then clamp.
        let mut prev = c.current_bitrate_bps();
        for _ in 0..100 {
            let d = c.update(CLEAN, RTT, 20.0);
            assert!(d.target_bitrate_bps <= MAX_BPS);
            assert!(d.target_bitrate_bps >= prev);
            prev = d.target_bitrate_bps;
        }
        assert_eq!(c.current_bitrate_bps(), MAX_BPS);
    }

    #[test]
    fn anti_thrash_no_increase_until_decrease_cooldown_elapsed() {
        let mut c = CompatBitrateController::new();
        // One decrease (heavy tick) → cooldown armed.
        let dec = c.update(HEAVY, RTT, 1.0);
        assert!(dec.changed);
        let after_decrease = c.current_bitrate_bps();
        // Clean ticks that DO satisfy the 10 s increase interval (12 s) but NOT
        // the 5 s decrease cooldown yet (only 4 s elapsed) → no increase.
        let d = c.update(CLEAN, RTT, 4.0);
        assert_eq!(d.target_bitrate_bps, after_decrease);
        assert!(!d.changed);
        // Cross the 5 s cooldown AND the 10 s increase interval → increase.
        let d2 = c.update(CLEAN, RTT, 8.0);
        assert!(d2.changed);
        assert_eq!(d2.target_bitrate_bps, after_decrease + AI_STEP_BPS);
    }

    #[test]
    fn increase_cadence_one_step_per_ten_second_window() {
        let mut c = CompatBitrateController::new();
        run(&mut c, HEAVY, 1.0, 200); // to MIN; cooldown armed at last decrease
        let base = c.current_bitrate_bps();
        // Let the 5 s decrease cooldown pass with small clean ticks below the
        // 10 s increase interval: 4 s + 4 s = 8 s since last increase (interval
        // resets are only on increase). Neither crosses the 10 s boundary alone.
        let a = c.update(CLEAN, RTT, 4.0);
        assert!(!a.changed); // 4 s < 10 s interval
        let b = c.update(CLEAN, RTT, 4.0);
        // 8 s total since start; still < 10 s interval → no step yet.
        assert!(!b.changed);
        // Now cross 10 s: 8 + 4 = 12 s → exactly ONE step.
        let cdec = c.update(CLEAN, RTT, 4.0);
        assert!(cdec.changed);
        assert_eq!(cdec.target_bitrate_bps, base + AI_STEP_BPS);
        // Immediately after, a sub-interval tick must NOT step again.
        let d = c.update(CLEAN, RTT, 4.0);
        assert!(!d.changed);
        assert_eq!(d.target_bitrate_bps, base + AI_STEP_BPS);
    }

    #[test]
    fn single_loss_spike_below_ewma_threshold_does_not_decrease() {
        let mut c = CompatBitrateController::new();
        // One spike of 0.02 observed: ewma = 0.35*0 + 0.65*0.02 = 0.013 < 0.02.
        let d = c.update(0.02, RTT, 1.0);
        assert!(!d.changed, "0.013 ewma must not trip LOSS_HIGH=0.02");
        assert_eq!(d.target_bitrate_bps, MAX_BPS);
    }

    #[test]
    fn sustained_loss_above_threshold_eventually_decreases() {
        let mut c = CompatBitrateController::new();
        // HEAVY=0.05 converges ewma toward 0.05 > 0.02. First tick already
        // 0.0325 > 0.02 → decrease immediately.
        let d = c.update(HEAVY, RTT, 1.0);
        assert!(d.changed);
        assert!(d.target_bitrate_bps < MAX_BPS);
    }

    #[test]
    fn changed_flag_true_only_when_value_moves() {
        let mut c = CompatBitrateController::new();
        // No-op tick at MAX with clean link.
        let d0 = c.update(CLEAN, RTT, 1.0);
        assert!(!d0.changed);
        // Decrease tick moves the value.
        let d1 = c.update(HEAVY, RTT, 1.0);
        assert!(d1.changed);
        assert_ne!(d1.target_bitrate_bps, MAX_BPS);
    }

    #[test]
    fn deterministic_same_inputs_same_outputs() {
        let mut a = CompatBitrateController::new();
        let mut b = CompatBitrateController::new();
        // Mixed sequence applied identically to two independent controllers.
        let seq = [
            (0.05, 1.0),
            (0.0, 4.0),
            (0.0, 8.0),
            (0.03, 1.0),
            (0.0, 20.0),
        ];
        for &(loss, dt) in &seq {
            let da = a.update(loss, RTT, dt);
            let db = b.update(loss, RTT, dt);
            assert_eq!(da.target_bitrate_bps, db.target_bitrate_bps);
            assert_eq!(da.changed, db.changed);
        }
        assert_eq!(a.current_bitrate_bps(), b.current_bitrate_bps());
    }

    #[test]
    fn rtt_does_not_drive_decisions_in_v1() {
        // Identical loss/dt with wildly different RTT → identical bitrate path.
        let mut low_rtt = CompatBitrateController::new();
        let mut high_rtt = CompatBitrateController::new();
        for _ in 0..50 {
            let a = low_rtt.update(0.05, 5.0, 1.0);
            let b = high_rtt.update(0.05, 5000.0, 1.0);
            assert_eq!(a.target_bitrate_bps, b.target_bitrate_bps);
        }
        assert_eq!(
            low_rtt.current_bitrate_bps(),
            high_rtt.current_bitrate_bps()
        );
    }

    #[test]
    fn decrease_is_multiplicative_by_md_factor() {
        let mut c = CompatBitrateController::new();
        // First heavy tick: ewma 0.0325 > LOSS_HIGH → bitrate * 0.85, rounded.
        let d = c.update(HEAVY, RTT, 1.0);
        let expected = (MAX_BPS as f64 * MD_FACTOR).round() as i32;
        assert_eq!(d.target_bitrate_bps, expected);
        assert_eq!(expected, 765_000);
    }
}
