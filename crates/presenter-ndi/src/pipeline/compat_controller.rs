//! Per-consumer COMPAT adaptive-bitrate control (#387): the AIMD loop that
//! drives each compat consumer's own `vp8enc` `target-bitrate` from THAT
//! consumer's RTCP loss/RTT, plus the RTCP-loss sampling helpers it reads.
//!
//! Split out of `consumers.rs` (file-size cap) as a cohesive leaf: it owns no
//! `ConsumerBranch` state — `consumers::spawn_controller_if_compat` wires the
//! returned task + bitrate handle onto the branch. The DECISION logic is unit-
//! tested in `adaptive.rs`; this WIRING (stat-read → controller → set
//! target-bitrate) is proven by the lab tc-netem functional check.

use std::sync::Arc;

use gstreamer as gst;
use gstreamer::prelude::*;

use super::adaptive::{BitrateDecision, CompatBitrateController};

/// How often the per-consumer compat AIMD loop samples RTCP and steps the
/// controller (#387). 1.5 s matches the controller's anti-thrash cadence
/// (10 s increase interval, 5 s post-decrease cooldown) while reacting to a
/// loss spike within a couple of ticks.
const CONTROLLER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

/// Spawn the per-consumer adaptive-bitrate task (#387): every
/// `CONTROLLER_INTERVAL` it reads this consumer's webrtcbin RTCP
/// remote-inbound stats (the peer's view of OUR stream — packets-lost/-received
/// deltas → loss fraction, round-trip-time), feeds them to the per-consumer
/// `CompatBitrateController` AIMD step, and — when the decision moves — sets
/// the consumer's own `vp8enc` `target-bitrate` LIVE (vp8enc takes bits/s i32;
/// no caps change → NO decoder port-reconfig, so the Vestel OMX is never
/// killed — addendum 2). The latest value is mirrored into `bitrate` so
/// `snapshot` can read it. The task never ends on its own; the explicit abort
/// (by `ConsumerBranch`/`WhepSession` Drop) is its only exit.
///
/// get-stats blocks on a promise, so each sample runs in `spawn_blocking` off
/// the async runtime (never blocking this task). The DECISION logic is unit-
/// tested in `adaptive.rs` (13 tests); this WIRING (stat-read → controller →
/// set target-bitrate) is proven by the lab tc-netem functional check, not a
/// unit test (no live RTCP in unit tests).
pub(super) fn spawn_compat_bitrate_controller(
    webrtcbin: gst::Element,
    encoder: gst::Element,
    bitrate: Arc<std::sync::atomic::AtomicI32>,
    session_id: String,
    rt: &tokio::runtime::Handle,
) -> tokio::task::JoinHandle<()> {
    rt.spawn(async move {
        let mut controller = CompatBitrateController::new();
        // Mirror the controller's start value (== the vp8enc's start
        // target-bitrate) so the snapshot reflects it before the first tick.
        bitrate.store(
            controller.current_bitrate_bps(),
            std::sync::atomic::Ordering::Relaxed,
        );
        // None until the FIRST real RR sample establishes the baseline — so the
        // first observation is a true delta, never first-real-vs-zero.
        let mut prev: Option<LossSample> = None;
        let mut interval = tokio::time::interval(CONTROLLER_INTERVAL);
        // Skip the immediate first tick — wait one interval so RTCP has a chance
        // to arrive before the first observation.
        interval.tick().await;
        loop {
            interval.tick().await;
            let wb = webrtcbin.clone();
            let join = tokio::task::spawn_blocking(move || read_loss_sample(&wb)).await;
            // None = no RR this tick (pre-connect, peer quiet, or get-stats
            // timeout). Treat as NO OBSERVATION: skip WITHOUT updating `prev`,
            // so a timed-out read never becomes a baseline that fabricates a
            // huge seq-delta (and a spurious decrease) on the next real read.
            // Quality policy: reduce ONLY on measured loss.
            let Ok(Some(sample)) = join else {
                continue;
            };
            // First real sample only establishes the baseline.
            let Some(prev_sample) = prev.replace(sample.clone()) else {
                continue;
            };
            let (observed_loss, dt) = prev_sample.delta(&sample);
            let decision: BitrateDecision = controller.update(observed_loss, sample.rtt_ms, dt);
            tracing::debug!(
                session_id = %session_id,
                observed_loss,
                rtt_ms = sample.rtt_ms,
                target_bitrate_bps = decision.target_bitrate_bps,
                changed = decision.changed,
                "compat AIMD tick"
            );
            if decision.changed {
                // Live, no caps change → no Vestel OMX port-reconfig.
                encoder.set_property("target-bitrate", decision.target_bitrate_bps);
            }
            bitrate.store(
                decision.target_bitrate_bps,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    })
}

/// One RTCP-loss observation for the per-consumer AIMD loop (#387): the peer's
/// cumulative packets-lost, the count of packets the peer has acknowledged
/// (extended-highest-seq from the RR block — a proxy for packets RECEIVED+LOST
/// the peer has seen), the current RTT (ms), and a monotonic capture time so
/// the loop derives a real `dt`. Cumulative counters → the controller wants a
/// per-interval FRACTION, so [`LossSample::delta`] differences two samples.
#[derive(Clone)]
struct LossSample {
    /// Cumulative packets the peer reported lost (`remote-inbound-rtp`
    /// `packets-lost`), or the RR block's `rb-packetslost` fallback.
    packets_lost: i64,
    /// Extended highest sequence number the peer has received
    /// (`rb-exthighestseq`) — advances by the count of packets the peer has
    /// seen (received + lost) since the stream start.
    ext_highest_seq: u64,
    /// Current round-trip-time (ms); 0.0 when no RR is present yet.
    rtt_ms: f64,
    /// When this sample was captured (for the controller's `dt_secs`).
    captured: std::time::Instant,
}

impl Default for LossSample {
    fn default() -> Self {
        Self {
            packets_lost: 0,
            ext_highest_seq: 0,
            rtt_ms: 0.0,
            captured: std::time::Instant::now(),
        }
    }
}

impl LossSample {
    /// Loss FRACTION and elapsed seconds between `self` (older) and `next`
    /// (newer). `observed_loss = lost_delta / max(1, seq_delta)`, clamped to
    /// `0.0..=1.0` (the denominator is the packets the peer saw in the window:
    /// `ext_highest_seq` advances by received+lost). A non-advancing or
    /// regressing sequence (reconnect, stats reset, or no fresh RR) yields a
    /// clean 0.0 observation — never a spurious decrease.
    fn delta(&self, next: &LossSample) -> (f64, f64) {
        let dt = next
            .captured
            .saturating_duration_since(self.captured)
            .as_secs_f64()
            .max(0.001);
        let lost_delta = (next.packets_lost - self.packets_lost).max(0);
        let seq_delta = next.ext_highest_seq.saturating_sub(self.ext_highest_seq);
        let observed_loss = if seq_delta == 0 {
            0.0
        } else {
            (lost_delta as f64 / seq_delta as f64).clamp(0.0, 1.0)
        };
        (observed_loss, dt)
    }
}

/// Read one [`LossSample`] from a webrtcbin's RTCP remote-inbound stats (#387).
/// Reuses the `get-stats` promise idiom of [`rtcp_remote_inbound`] /
/// `reaper::peer_rr_fingerprint`: the peer's RR carries `round-trip-time` +
/// `packets-lost` in `remote-inbound-rtp`, and the nested
/// `gst-rtpsource-stats` RR block carries `rb-exthighestseq` (packets the peer
/// has seen) + `rb-packetslost`. Returns `None` when no RR is present yet (the
/// promise timed out, or the peer hasn't sent a receiver report) — the caller
/// treats `None` as NO OBSERVATION and skips the tick WITHOUT moving its
/// baseline, so a not-yet-connected or momentarily-quiet consumer never
/// fabricates a spurious decrease.
fn read_loss_sample(webrtcbin: &gst::Element) -> Option<LossSample> {
    let captured = std::time::Instant::now();
    let (tx, rx) = std::sync::mpsc::channel();
    let promise = gst::Promise::with_change_func(move |reply| {
        if let Ok(Some(stats)) = reply {
            let _ = tx.send(stats.to_owned());
        }
    });
    webrtcbin.emit_by_name::<()>("get-stats", &[&None::<gst::Pad>, &promise]);
    let stats = rx
        .recv_timeout(std::time::Duration::from_millis(500))
        .ok()?;
    let mut sample = LossSample {
        captured,
        ..Default::default()
    };
    let mut saw_rr = false;
    for (_field, value) in stats.iter() {
        let Ok(s) = value.get::<gst::Structure>() else {
            continue;
        };
        if s.has_field("round-trip-time") {
            saw_rr = true;
            if let Ok(rtt) = s.get::<f64>("round-trip-time") {
                sample.rtt_ms = rtt * 1000.0;
            }
            if let Some(lost) = stats_i64(&s, "packets-lost") {
                sample.packets_lost = lost;
            }
        }
        if let Ok(nested) = s.get::<gst::Structure>("gst-rtpsource-stats") {
            if nested.get::<bool>("have-rb").unwrap_or(false) {
                saw_rr = true;
                if let Some(seq) = stats_u64(&nested, "rb-exthighestseq") {
                    sample.ext_highest_seq = seq;
                }
                // Prefer the RR block's packets-lost when the top-level field
                // is absent on this GStreamer version.
                if sample.packets_lost == 0 {
                    if let Some(lost) = stats_i64(&nested, "rb-packetslost") {
                        sample.packets_lost = lost;
                    }
                }
            }
        }
    }
    // No RR fields at all → no real observation this tick.
    saw_rr.then_some(sample)
}

/// Read a numeric stats field as i64, tolerating i64/u64/i32/u32 across
/// GStreamer versions (webrtcbin uses different reprs for different counters).
fn stats_i64(s: &gst::Structure, field: &str) -> Option<i64> {
    s.get::<i64>(field)
        .ok()
        .or_else(|| s.get::<u64>(field).ok().map(|v| v as i64))
        .or_else(|| s.get::<i32>(field).ok().map(i64::from))
        .or_else(|| s.get::<u32>(field).ok().map(i64::from))
}

/// Read a numeric stats field as u64, tolerating the u64/i64/u32 representation
/// webrtcbin uses across GStreamer versions (mirrors `reaper::stats_u64`).
fn stats_u64(s: &gst::Structure, field: &str) -> Option<u64> {
    s.get::<u64>(field)
        .ok()
        .or_else(|| s.get::<i64>(field).ok().map(|v| v.max(0) as u64))
        .or_else(|| s.get::<u32>(field).ok().map(u64::from))
}
