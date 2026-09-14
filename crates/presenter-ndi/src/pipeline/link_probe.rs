//! Per-consumer-link drop DISCRIMINATOR probe (#768 D2b).
//!
//! Lane 3 refuted the base_time/timeline-offset root-cause hypothesis by code
//! and concluded the ONE missing measurement is a per-consumer-link
//! discriminator AT DROP TIME: is a drop an appsrc queue overflow (downstream
//! draining slower than realtime) or a `needs_keyframe` / DISCONT wait
//! (forwarding gated on the next IDR), and how late is the forwarded buffer
//! relative to the consumer pipeline's own running-time.
//!
//! GROUND TRUTH for the drop count, NOT the appsrc signal (the #1 review
//! finding): `gstreamer_utils::StreamProducer::add_consumer` INSTALLS an
//! `enough-data` CALLBACK on the consumer appsrc (`streamproducer.rs`
//! `StreamConsumer::new` → `appsrc.set_callbacks(...enough_data...)`), which
//! increments the `ConsumptionLink`'s `dropped()` counter and posts a
//! `dropped-buffer` element message on every overflow. Per the GStreamer
//! appsrc contract a slot with an installed callback NO LONGER emits its
//! signal, so an `emit-signals` + `connect("enough-data", …)` handler on the
//! SAME appsrc never fires in production — reading overflow from the signal
//! would leave the count stuck at 0 and make `QueueOverflow` unreachable. So
//! the drop count is taken from `ConsumptionLink::dropped()`, threaded in at
//! snapshot time. Only the `need-data` slot is left NULL by `StreamProducer`,
//! so THAT signal does fire and is the one signal we still connect (a
//! healthy-pull indicator).
//!
//! `dropped()` aggregates BOTH drop paths in `StreamProducer`: the `enough-data`
//! callback (queue overflow) AND `process_sample` dropping non-keyframe samples
//! while `needs_keyframe` is armed (a keyframe wait). So `dropped()>0` alone
//! cannot tell the two apart — the LATENESS gate is the real discriminator: a
//! genuine overflow backs the appsrc queue up so forwarded buffers fall well
//! behind the consumer clock (high lateness), whereas a pure keyframe wait just
//! holds for the next IDR without a backed-up queue (low/zero lateness). Hence
//! `classify` requires `dropped()>0` AND lateness > 250 ms for `QueueOverflow`.
//!
//! Everything else is measured from a src-pad BUFFER probe, which fires
//! reliably regardless of callbacks: DISCONT / keyframe buffer counts, buffer
//! lateness vs the consumer clock, and the appsrc's `current-level-time` at
//! forward time (the max is the queue-depth evidence). All cheap: plain
//! atomics, no allocation per buffer, and a `debug!` rate-limited to at most
//! once per 5 s per session (`log-flood-backoff.md`).
//!
//! The whole module is pure/atomic and the classifier is unit-tested, so it is
//! verifiable on every CI host without libndi/GPU — this crate is Tier-0 (no
//! local compile), so a pure seam is the pre-CI safety net
//! (`ndi-manager-locking.md`).

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

/// Lateness (ms) beyond which a queue-full condition is read as a genuine
/// overflow rather than a startup transient — half the 500 ms appsrc
/// `max-time` bound. A buffer more than this far in the PAST relative to the
/// consumer clock means the queue is genuinely backing up.
const LATENESS_OVERFLOW_MS: i64 = 250;

/// How a consumer link's drops are best explained, from the counters below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkVerdict {
    /// The appsrc queue overflowed (`ConsumptionLink::dropped()` climbed) and
    /// buffers fell behind the consumer clock — downstream is draining slower
    /// than realtime.
    QueueOverflow,
    /// Forwarding was keyframe-gated: DISCONT-flagged buffers appeared on the
    /// src pad (the producer re-armed `needs_keyframe` and resumed at an IDR)
    /// WITHOUT a sustained overflow.
    KeyframeWait,
    /// Neither signature present — the link looks healthy.
    Healthy,
}

/// Classify a consumer link's drop signature (#768 D2b). Pure so it is
/// unit-testable on any CI host. `dropped_buffers` is `ConsumptionLink::dropped()`,
/// which aggregates BOTH the `enough-data` overflow drops AND the keyframe-wait
/// drops (module doc). `QueueOverflow` therefore requires BOTH a real drop AND
/// buffers falling well behind the consumer clock — the lateness gate is what
/// isolates a genuine backed-up-queue overflow from a keyframe wait, whose drops
/// carry near-zero lateness.
pub fn classify(dropped_buffers: u64, discont_buffers: u64, lateness_max_ms: i64) -> LinkVerdict {
    // Queue overflow is the root when the producer actually dropped buffers for
    // this link (dropped()>0) AND buffers fell well behind the consumer clock —
    // downstream draining slower than realtime, the #768 incident signature. It
    // takes precedence over a keyframe wait: an overflow re-arms needs_keyframe
    // inside the same callback, producing the DISCONT buffers at the next IDR.
    if dropped_buffers > 0 && lateness_max_ms > LATENESS_OVERFLOW_MS {
        LinkVerdict::QueueOverflow
    } else if discont_buffers > 0 {
        // Forwarding resumed at an IDR after a gap, with no sustained overflow.
        LinkVerdict::KeyframeWait
    } else {
        LinkVerdict::Healthy
    }
}

/// Pure lateness (ms): how far a forwarded buffer's RUNNING-TIME is behind the
/// consumer clock. `now_running_ns` is the consumer appsrc element's
/// `current_running_time()`; `buffer_running_ns` is `segment.to_running_time(pts)`
/// — NOT the raw PTS. Positive = buffer in the past (a realtime buffer sits
/// ~one frame / ~33 ms behind at 30 fps); ~0 = healthy; negative = ahead of the
/// clock (a genuine future-timestamp a sync sink would hold on).
///
/// #768 lane 6: the previous probe subtracted the RAW `buffer.pts()`, which for
/// the ndisrc → StreamProducer → appsrc path carries a constant ~1 h segment
/// base — so the old number read a fixed −3 600 000 ms on every session even
/// while media flowed at realtime 30 fps. Applying the segment (via
/// [`LinkProbe::buffer_lateness_ms`]) yields the same running-time GStreamer's
/// own sinks synchronise on, which the incident's continuous realtime flow
/// proves is ≈ now — so the corrected lateness is ≈ 0, not −3 600 000. Split out
/// pure so it is unit-tested on any CI host without gst.
pub(crate) fn lateness_ms(now_running_ns: i64, buffer_running_ns: i64) -> i64 {
    (now_running_ns - buffer_running_ns) / 1_000_000
}

/// Cheap per-consumer-link diagnostic counters (#768 D2b). All fields are
/// atomics updated from GStreamer streaming threads (the appsrc `need-data`
/// signal handler and the src-pad buffer probe) and read losslessly by the
/// snapshot reader. The OVERFLOW count is NOT stored here — it is read from the
/// `ConsumptionLink::dropped()` ground truth at snapshot time (see the module
/// doc). Lateness is tracked in milliseconds; `lateness_min_ms`/`max` carry
/// sentinels (`i64::MAX`/`MIN`) until the first buffer so `min`/`max` are
/// reported as `None` while nothing has flowed.
#[derive(Debug)]
pub struct LinkProbe {
    need_data_events: AtomicU64,
    discont_buffers: AtomicU64,
    keyframe_buffers: AtomicU64,
    total_buffers: AtomicU64,
    max_queue_level_ms: AtomicU64,
    lateness_min_ms: AtomicI64,
    lateness_max_ms: AtomicI64,
    lateness_last_ms: AtomicI64,
    /// The active TIME segment on the consumer appsrc src pad, captured from the
    /// sticky SEGMENT event (#768 lane 6). Used to convert a buffer PTS to
    /// RUNNING-TIME (`segment.to_running_time(pts)`) so `lateness_ms` measures
    /// the same running-time GStreamer's own sinks synchronise on — NOT the raw
    /// PTS, which carried the ndisrc/appsrc segment's constant ~1 h base and made
    /// the old raw `now − pts` read a bogus −3 600 000 ms (module doc).
    active_segment: std::sync::Mutex<Option<gst::FormattedSegment<gst::ClockTime>>>,
}

impl Default for LinkProbe {
    fn default() -> Self {
        Self {
            need_data_events: AtomicU64::new(0),
            discont_buffers: AtomicU64::new(0),
            keyframe_buffers: AtomicU64::new(0),
            total_buffers: AtomicU64::new(0),
            max_queue_level_ms: AtomicU64::new(0),
            lateness_min_ms: AtomicI64::new(i64::MAX),
            lateness_max_ms: AtomicI64::new(i64::MIN),
            lateness_last_ms: AtomicI64::new(0),
            active_segment: std::sync::Mutex::new(None),
        }
    }
}

impl LinkProbe {
    /// Record one appsrc `need-data` emission (the queue drained below its low
    /// watermark and is asking for more — the healthy steady state). This is
    /// the one appsrc signal `StreamProducer` leaves free (no callback), so it
    /// fires reliably with `emit-signals=true`.
    pub(crate) fn record_need_data(&self) {
        self.need_data_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one buffer forwarded out of the appsrc src pad: whether it is
    /// DISCONT-flagged, whether it is a keyframe (`!DELTA_UNIT`), its lateness
    /// (ms) relative to the consumer pipeline's current running-time (positive
    /// = in the past; negative = in the future = a sync sink holds), and the
    /// appsrc's `current-level-time` (ms) at that moment (the running max is
    /// the queue-depth evidence). Fired from the src-pad probe, which is
    /// unaffected by the callback-vs-signal contract.
    pub(crate) fn record_buffer(
        &self,
        discont: bool,
        keyframe: bool,
        lateness_ms: i64,
        queue_level_ms: u64,
    ) {
        self.total_buffers.fetch_add(1, Ordering::Relaxed);
        if discont {
            self.discont_buffers.fetch_add(1, Ordering::Relaxed);
        }
        if keyframe {
            self.keyframe_buffers.fetch_add(1, Ordering::Relaxed);
        }
        self.max_queue_level_ms
            .fetch_max(queue_level_ms, Ordering::Relaxed);
        self.lateness_min_ms
            .fetch_min(lateness_ms, Ordering::Relaxed);
        self.lateness_max_ms
            .fetch_max(lateness_ms, Ordering::Relaxed);
        self.lateness_last_ms.store(lateness_ms, Ordering::Relaxed);
    }

    /// Store the active TIME segment seen on the consumer appsrc src pad (#768
    /// lane 6). The SEGMENT event is sticky and always delivered before buffers,
    /// so by the time [`Self::buffer_lateness_ms`] runs the segment is present.
    /// Cheap: one Mutex swap on a rare event (a poisoned lock is recovered
    /// rather than propagated — a diagnostic must never poison the streaming
    /// thread).
    pub(crate) fn record_segment(&self, segment: gst::FormattedSegment<gst::ClockTime>) {
        *self
            .active_segment
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(segment);
    }

    /// Lateness (ms) of a forwarded buffer vs the consumer clock, using the
    /// stored segment to convert PTS → RUNNING-TIME (`segment.to_running_time`)
    /// before the subtraction — the #768 lane 6 fix for the constant −3 600 000
    /// ms artifact (the raw PTS carried the segment's ~1 h base). Falls back to
    /// the raw PTS when no segment has been captured yet (its start/base are 0
    /// in that degenerate case, so the two agree), and to 0 when the
    /// running-time or PTS is unknown (pts outside the segment, or an unclocked
    /// pipeline) — the same defensive default the old raw path used.
    pub(crate) fn buffer_lateness_ms(
        &self,
        now_running: Option<gst::ClockTime>,
        pts: Option<gst::ClockTime>,
    ) -> i64 {
        let running = {
            let seg = self
                .active_segment
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            match seg.as_ref() {
                // Convert PTS → running-time via the active segment (the fix).
                Some(s) => s.to_running_time(pts),
                // No segment captured yet: raw PTS (degenerate start=0/base=0).
                None => pts,
            }
        };
        match (now_running, running) {
            (Some(now), Some(rt)) => lateness_ms(now.nseconds() as i64, rt.nseconds() as i64),
            _ => 0,
        }
    }

    /// Render the diagnostic snapshot (a cheap set of atomic loads plus the
    /// classifier verdict). `dropped_buffers` is the ground-truth drop count
    /// from `ConsumptionLink::dropped()`, supplied by the caller because
    /// the appsrc `enough-data` signal is suppressed by `StreamProducer`'s
    /// callback (module doc). Lateness min/max/last are `None` until the first
    /// buffer has flowed.
    pub(crate) fn to_snapshot(&self, dropped_buffers: u64) -> LinkProbeSnapshot {
        let total = self.total_buffers.load(Ordering::Relaxed);
        let discont_buffers = self.discont_buffers.load(Ordering::Relaxed);
        let lateness_max = self.lateness_max_ms.load(Ordering::Relaxed);
        let lateness_ms = if total == 0 {
            LatenessSnapshot {
                min: None,
                max: None,
                last: None,
            }
        } else {
            LatenessSnapshot {
                min: Some(self.lateness_min_ms.load(Ordering::Relaxed)),
                max: Some(lateness_max),
                last: Some(self.lateness_last_ms.load(Ordering::Relaxed)),
            }
        };
        // The classifier only trusts lateness once a buffer has flowed.
        let lateness_for_verdict = if total == 0 { 0 } else { lateness_max };
        LinkProbeSnapshot {
            dropped_buffers,
            need_data_events: self.need_data_events.load(Ordering::Relaxed),
            discont_buffers,
            keyframe_buffers: self.keyframe_buffers.load(Ordering::Relaxed),
            max_queue_level_ms: self.max_queue_level_ms.load(Ordering::Relaxed),
            lateness_ms,
            verdict: classify(dropped_buffers, discont_buffers, lateness_for_verdict),
        }
    }
}

/// Lateness (ms) of forwarded buffers relative to the consumer clock, as
/// min/max/last over the session. All `None` until the first buffer. Computed
/// as `running_time_now − segment.to_running_time(buffer.pts())` (#768 lane 6):
/// the buffer PTS is converted to RUNNING-TIME via the pad's active segment, so
/// a non-zero segment base/offset — the ndisrc/appsrc path carries a constant
/// ~1 h base, which the earlier raw `now − pts` mis-read as ≈ −3 600 000 ms on
/// every session — is removed. It is the same running-time GStreamer's own sinks
/// synchronise on, so a realtime buffer reads ≈0 (≈ one frame behind).
/// Diagnostic, not a hard SLA.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatenessSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<i64>,
}

/// Serialized per-consumer-link diagnostic for `GET /ndi/snapshot/{id}`
/// `sessions[].link` (#768 D2b). `verdict` classifies the drop signature so an
/// operator or the deploy self-heal log can read it WITHOUT doing the math.
/// `droppedBuffers` is the ground-truth drop count from the
/// `ConsumptionLink` (== the session's `buffersDropped`, restated here so the
/// link block is self-contained).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkProbeSnapshot {
    pub dropped_buffers: u64,
    pub need_data_events: u64,
    pub discont_buffers: u64,
    pub keyframe_buffers: u64,
    pub max_queue_level_ms: u64,
    pub lateness_ms: LatenessSnapshot,
    pub verdict: LinkVerdict,
}

/// If `info` carries a SEGMENT event, capture its TIME segment on `probe` and
/// return `true` (the caller returns early — an event is not a buffer). Returns
/// `false` for a buffer, which the caller then measures. Split out of `attach`
/// to keep that fn under the length cap (#768 lane 6).
fn capture_segment_event(probe: &LinkProbe, info: &gst::PadProbeInfo) -> bool {
    if let Some(gst::PadProbeData::Event(ev)) = &info.data {
        if let gst::EventView::Segment(seg_ev) = ev.view() {
            if let Some(seg) = seg_ev.segment().downcast_ref::<gst::ClockTime>() {
                probe.record_segment(seg.clone());
            }
        }
        return true;
    }
    false
}

/// Attach the #768 D2b diagnostic probe to a consumer `appsrc`: the `need-data`
/// signal (the one slot `StreamProducer` leaves free) and a src-pad buffer
/// probe (DISCONT / keyframe / lateness / queue level). Creates the
/// [`LinkProbe`] and returns it for storage on the `WhepSession`; the overflow
/// count is read separately from `ConsumptionLink::dropped()` at snapshot time.
/// The buffer probe holds a WEAK appsrc ref, so it never forms a reference
/// cycle keeping the appsrc alive after the consumer pipeline is torn down.
pub(crate) fn attach(appsrc: &gst_app::AppSrc, session_id: &str) -> Arc<LinkProbe> {
    let probe = Arc::new(LinkProbe::default());
    // Additive: enable signal emission without touching the callback slot. Only
    // `need-data` actually fires — `StreamProducer::add_consumer` later installs
    // an `enough-data` callback that suppresses that signal (module doc).
    appsrc.set_property("emit-signals", true);

    let p_need = probe.clone();
    appsrc.connect("need-data", false, move |_args| {
        p_need.record_need_data();
        None
    });

    let Some(src_pad) = appsrc.static_pad("src") else {
        tracing::warn!(
            session_id = %session_id,
            "consumer appsrc has no src pad; #768 D2b buffer probe not attached"
        );
        return probe;
    };
    let appsrc_weak = appsrc.downgrade();
    let p_buf = probe.clone();
    let sid = session_id.to_string();
    let base = Instant::now();
    let last_log_ms = AtomicI64::new(-5_000);
    src_pad.add_probe(
        gst::PadProbeType::BUFFER | gst::PadProbeType::EVENT_DOWNSTREAM,
        move |_pad, info| {
            // A SEGMENT event: capture the TIME segment (so buffer lateness is
            // measured against RUNNING-TIME, not the raw PTS — #768 lane 6) and
            // return early; an event is not a buffer.
            if capture_segment_event(&p_buf, info) {
                return gst::PadProbeReturn::Ok;
            }
            if let Some(buffer) = info.buffer() {
                let flags = buffer.flags();
                let discont = flags.contains(gst::BufferFlags::DISCONT);
                let keyframe = !flags.contains(gst::BufferFlags::DELTA_UNIT);
                let (lateness_ms, queue_level_ms) = appsrc_weak
                    .upgrade()
                    .map(|src| {
                        let queue_level_ms = src.current_level_time().mseconds();
                        // Segment-aware: convert PTS → running-time via the captured
                        // segment before subtracting the consumer clock (#768 lane 6).
                        let lateness_ms = p_buf.buffer_lateness_ms(
                            src.upcast_ref::<gst::Element>().current_running_time(),
                            buffer.pts(),
                        );
                        (lateness_ms, queue_level_ms)
                    })
                    .unwrap_or((0, 0));
                p_buf.record_buffer(discont, keyframe, lateness_ms, queue_level_ms);

                // Rate-limited (≤ 1 per 5 s per session) so a per-buffer signal
                // never floods the log (`log-flood-backoff.md`). dropped() is read
                // here for the log line only; the snapshot reads it authoritatively.
                let elapsed = base.elapsed().as_millis() as i64;
                let prev = last_log_ms.load(Ordering::Relaxed);
                if elapsed - prev >= 5_000
                    && last_log_ms
                        .compare_exchange(prev, elapsed, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                {
                    let snap = p_buf.to_snapshot(0);
                    tracing::debug!(
                        session_id = %sid,
                        need_data = snap.need_data_events,
                        discont = snap.discont_buffers,
                        keyframes = snap.keyframe_buffers,
                        max_queue_ms = snap.max_queue_level_ms,
                        lateness_max_ms = ?snap.lateness_ms.max,
                        "#768 D2b link probe (verdict computed at snapshot with dropped())"
                    );
                }
            }
            gst::PadProbeReturn::Ok
        },
    );
    probe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_healthy_when_no_signals() {
        assert_eq!(classify(0, 0, 0), LinkVerdict::Healthy);
    }

    #[test]
    fn classify_queue_overflow_on_drops_with_lateness() {
        // The link actually dropped buffers AND they are well behind the
        // consumer clock = the #768 incident signature (downstream drains
        // slower than realtime).
        assert_eq!(classify(50, 3, 400), LinkVerdict::QueueOverflow);
    }

    #[test]
    fn classify_keyframe_wait_on_discont_without_drops() {
        // DISCONT-flagged buffers but no overflow drops = forwarding gated on
        // the next IDR, not a slow consumer.
        assert_eq!(classify(0, 5, -10), LinkVerdict::KeyframeWait);
    }

    #[test]
    fn classify_keyframe_wait_with_dropped_but_low_lateness() {
        // The REAL keyframe-wait signature: process_sample drops non-IDR samples
        // while needs_keyframe is armed, so dropped()>0 — but the queue is NOT
        // backed up (low lateness). The lateness gate keeps this KeyframeWait,
        // NOT QueueOverflow. This is why dropped()>0 alone can't decide.
        assert_eq!(classify(6, 3, 30), LinkVerdict::KeyframeWait);
    }

    #[test]
    fn classify_drops_without_lateness_is_not_overflow() {
        // A drop with near-zero lateness (a brief blip, not sustained backup)
        // must NOT be read as an overflow; with no DISCONT it is Healthy.
        assert_eq!(classify(1, 0, 20), LinkVerdict::Healthy);
    }

    #[test]
    fn classify_overflow_takes_precedence_over_keyframe_wait() {
        // Both signatures present (overflow re-arming needs_keyframe → DISCONT
        // at the next IDR): the queue overflow is the root, so it wins.
        assert_eq!(classify(30, 4, 450), LinkVerdict::QueueOverflow);
    }

    #[test]
    fn classify_healthy_session_with_join_drops_is_not_overflow_768_lane7() {
        // #768 lane 7 RED: the SNV v0.4.279 live incident. A HEALTHY session
        // (`dropRatio30s == 0.0`) whose ONLY drops were join-time — dropped=87,
        // discont=2, latenessMs.max=256 (the join queue backed up to ~500 ms
        // waiting for the first IDR, so the segment-corrected lateness read
        // ~+256 ms) — reads `QueueOverflow` under the since-join cumulative
        // classifier, because `dropped>0 && lateness_max>250`. It MUST read
        // `Healthy`: the recalibration keys the verdict off drops IN THE
        // TRAILING WINDOW, not aged-out join transients. FAILS on the current
        // cumulative classifier (returns QueueOverflow).
        assert_eq!(classify(87, 2, 256), LinkVerdict::Healthy);
    }

    #[test]
    fn to_snapshot_empty_probe_is_healthy_with_absent_lateness() {
        let snap = LinkProbe::default().to_snapshot(0);
        assert_eq!(snap.verdict, LinkVerdict::Healthy);
        assert_eq!(snap.dropped_buffers, 0);
        assert!(snap.lateness_ms.min.is_none());
        assert!(snap.lateness_ms.max.is_none());
        assert!(snap.lateness_ms.last.is_none());
    }

    #[test]
    fn to_snapshot_reflects_overflow_from_ground_truth_dropped() {
        let probe = LinkProbe::default();
        probe.record_need_data();
        // Buffers backing up (from the src-pad probe): late + a DISCONT at the
        // IDR resume, with the appsrc queue near its 500 ms bound.
        probe.record_buffer(true, true, 400, 480);
        probe.record_buffer(false, false, 380, 510);
        // The overflow count comes from ConsumptionLink::dropped(), threaded in
        // at snapshot time (the appsrc enough-data signal is suppressed).
        let snap = probe.to_snapshot(42);
        assert_eq!(snap.dropped_buffers, 42);
        assert_eq!(snap.need_data_events, 1);
        assert_eq!(snap.discont_buffers, 1);
        assert_eq!(snap.keyframe_buffers, 1);
        assert_eq!(snap.max_queue_level_ms, 510);
        assert_eq!(snap.lateness_ms.min, Some(380));
        assert_eq!(snap.lateness_ms.max, Some(400));
        assert_eq!(snap.lateness_ms.last, Some(380));
        assert_eq!(snap.verdict, LinkVerdict::QueueOverflow);
    }

    #[test]
    fn to_snapshot_keyframe_wait_when_dropped_is_zero() {
        // A pure keyframe wait: DISCONT buffers, but ConsumptionLink::dropped()
        // is still 0 (no overflow) → KeyframeWait, never QueueOverflow.
        let probe = LinkProbe::default();
        probe.record_buffer(true, true, 40, 80);
        let snap = probe.to_snapshot(0);
        assert_eq!(snap.dropped_buffers, 0);
        assert_eq!(snap.verdict, LinkVerdict::KeyframeWait);
    }

    #[test]
    fn snapshot_serializes_camelcase_with_verdict() {
        let probe = LinkProbe::default();
        probe.record_buffer(true, false, 450, 300);
        let json = serde_json::to_string(&probe.to_snapshot(7)).expect("serialize");
        assert!(json.contains("droppedBuffers"), "camelCase keys: {json}");
        assert!(json.contains("discontBuffers"), "camelCase keys: {json}");
        assert!(json.contains("maxQueueLevelMs"), "camelCase keys: {json}");
        assert!(
            json.contains("latenessMs"),
            "lateness object present: {json}"
        );
        assert!(json.contains("verdict"), "verdict present: {json}");
        assert!(
            json.contains("queueOverflow"),
            "verdict serializes camelCase: {json}"
        );
    }

    #[test]
    fn lateness_ms_realtime_buffer_is_near_zero() {
        // A realtime buffer: its running-time is ~one frame (33 ms @30 fps)
        // behind the consumer clock now → lateness ≈ +33 ms, the healthy signal.
        assert_eq!(lateness_ms(100_000_000, 67_000_000), 33);
        // Dead-on: running-time == now → 0.
        assert_eq!(lateness_ms(500_000_000, 500_000_000), 0);
        // A genuine future timestamp (buffer running-time ahead of the clock) is
        // negative — the shape a sync sink would hold on.
        assert_eq!(lateness_ms(40_000_000, 1_040_000_000), -1_000);
    }

    #[test]
    fn buffer_lateness_ms_removes_segment_base_1h_artifact() {
        // #768 lane 6: reproduces the observed anomaly and proves the fix.
        // The ndisrc/appsrc path presents a segment whose start is ~1 h, so a
        // realtime buffer's PTS is ~1 h AHEAD of its running-time. The OLD raw
        // `now − pts` therefore read ≈ −3 600 000 ms on every session even while
        // media flowed at 30 fps; the segment-aware value is ≈0.
        let _ = gstreamer::init();
        let probe = LinkProbe::default();
        let mut seg = gst::FormattedSegment::<gst::ClockTime>::new();
        seg.set_start(gst::ClockTime::from_seconds(3600));
        probe.record_segment(seg);

        // A buffer whose running-time is 40 ms, forwarded when the consumer
        // clock is also at 40 ms (realtime). Its PTS = start + 40 ms = 1 h + 40 ms.
        let now = gst::ClockTime::from_mseconds(40);
        let pts = gst::ClockTime::from_mseconds(3_600_040);

        let corrected = probe.buffer_lateness_ms(Some(now), Some(pts));
        assert!(
            corrected.abs() <= 1,
            "segment-aware lateness must be ≈0 for a realtime buffer, got {corrected} ms"
        );

        // The OLD raw computation (now − pts) is exactly the −3 600 000 ms
        // artifact this fix removes — asserted here so a regression that reverts
        // to raw PTS is caught.
        let raw_ms = (now.nseconds() as i64 - pts.nseconds() as i64) / 1_000_000;
        assert_eq!(
            raw_ms, -3_600_000,
            "raw pts−now is the −3600 s artifact the segment conversion fixes"
        );
    }

    #[test]
    fn buffer_lateness_ms_falls_back_to_raw_when_no_segment() {
        // Before any SEGMENT event is captured, fall back to the raw PTS (a
        // degenerate start=0/base=0 segment agrees with the raw value). now
        // 100 ms, pts 60 ms → 40 ms late.
        let _ = gstreamer::init();
        let probe = LinkProbe::default();
        let now = gst::ClockTime::from_mseconds(100);
        let pts = gst::ClockTime::from_mseconds(60);
        assert_eq!(probe.buffer_lateness_ms(Some(now), Some(pts)), 40);
    }

    #[test]
    fn buffer_lateness_ms_zero_when_clock_or_pts_unknown() {
        let _ = gstreamer::init();
        let probe = LinkProbe::default();
        assert_eq!(
            probe.buffer_lateness_ms(None, Some(gst::ClockTime::from_mseconds(5))),
            0
        );
        assert_eq!(
            probe.buffer_lateness_ms(Some(gst::ClockTime::from_mseconds(5)), None),
            0
        );
    }
}
