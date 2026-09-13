//! Per-consumer-link drop DISCRIMINATOR probe (#768 D2b).
//!
//! Lane 3 refuted the base_time/timeline-offset root-cause hypothesis by code
//! and concluded the ONE missing measurement is a per-consumer-link
//! discriminator AT DROP TIME: is a drop an `enough_data` queue overflow
//! (downstream draining slower than realtime) or a `needs_keyframe` /
//! DISCONT wait (forwarding gated on the next IDR), and how late is the
//! forwarded buffer relative to the consumer pipeline's own running-time.
//! `gstreamer_utils::ConsumptionLink` exposes only `pushed()`/`dropped()`
//! totals — the discriminator is internal to `StreamProducer` — so this probe
//! reads it from OUR side of the bridge: the consumer `appsrc` that
//! `StreamProducer::add_consumer` feeds.
//!
//! The signal path is ADDITIVE: `emit-signals=true` plus a generic signal
//! connect never touches the appsrc's callback slot (which `StreamProducer`
//! may set), unlike `set_callbacks`. The consumer appsrc carries no need-data/
//! enough-data callback in normal operation (`configure_consumer` sets only
//! properties), so with emit-signals the signals fire. Everything is cheap:
//! plain atomics, no allocation per buffer, and a `debug!` rate-limited to at
//! most once per 5 s per session (`log-flood-backoff.md`).
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

/// Lateness (ms) beyond which an `enough_data` queue-full condition is read as
/// a genuine overflow rather than a startup transient — half the 500 ms appsrc
/// `max-time` bound. A buffer more than this far in the PAST relative to the
/// consumer clock means the queue is genuinely backing up.
const LATENESS_OVERFLOW_MS: i64 = 250;

/// How a consumer link's drops are best explained, from the counters below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkVerdict {
    /// The appsrc queue filled (`enough-data`) and buffers fell behind the
    /// consumer clock — downstream is draining slower than realtime.
    QueueOverflow,
    /// Forwarding was keyframe-gated: DISCONT-flagged buffers appeared on the
    /// src pad (the producer re-armed `needs_keyframe` and resumed at an IDR).
    KeyframeWait,
    /// Neither signature present — the link looks healthy.
    Healthy,
}

/// Classify a consumer link's drop signature (#768 D2b). Pure so it is
/// unit-testable on any CI host. `QueueOverflow` requires BOTH an `enough-data`
/// event AND buffers falling behind the consumer clock, so a brief startup
/// queue blip with near-zero lateness is not misread as an overflow.
pub fn classify(
    enough_data_events: u64,
    discont_buffers: u64,
    lateness_max_ms: i64,
) -> LinkVerdict {
    // Queue overflow is the root when the appsrc queue filled (enough-data)
    // AND buffers fell well behind the consumer clock — downstream draining
    // slower than realtime, the #768 incident signature. It takes precedence
    // over a keyframe wait: the enough-data thrash is what re-arms
    // needs_keyframe, producing the DISCONT buffers at the next IDR.
    if enough_data_events > 0 && lateness_max_ms > LATENESS_OVERFLOW_MS {
        LinkVerdict::QueueOverflow
    } else if discont_buffers > 0 {
        // Forwarding resumed at an IDR after a gap, with no sustained overflow.
        LinkVerdict::KeyframeWait
    } else {
        LinkVerdict::Healthy
    }
}

/// Cheap per-consumer-link diagnostic counters (#768 D2b). All fields are
/// atomics updated from GStreamer streaming threads (the appsrc signal
/// handlers and the src-pad buffer probe) and read losslessly by the snapshot
/// reader. Lateness is tracked in milliseconds; `lateness_min_ms`/`max` carry
/// sentinels (`i64::MAX`/`MIN`) until the first buffer so `min`/`max` are
/// reported as `None` while nothing has flowed.
#[derive(Debug)]
pub struct LinkProbe {
    enough_data_events: AtomicU64,
    need_data_events: AtomicU64,
    discont_buffers: AtomicU64,
    keyframe_buffers: AtomicU64,
    total_buffers: AtomicU64,
    max_queue_level_ms: AtomicU64,
    lateness_min_ms: AtomicI64,
    lateness_max_ms: AtomicI64,
    lateness_last_ms: AtomicI64,
}

impl Default for LinkProbe {
    fn default() -> Self {
        Self {
            enough_data_events: AtomicU64::new(0),
            need_data_events: AtomicU64::new(0),
            discont_buffers: AtomicU64::new(0),
            keyframe_buffers: AtomicU64::new(0),
            total_buffers: AtomicU64::new(0),
            max_queue_level_ms: AtomicU64::new(0),
            lateness_min_ms: AtomicI64::new(i64::MAX),
            lateness_max_ms: AtomicI64::new(i64::MIN),
            lateness_last_ms: AtomicI64::new(0),
        }
    }
}

impl LinkProbe {
    /// Record one appsrc `need-data` emission (the queue drained below its low
    /// watermark and is asking for more — the healthy steady state).
    pub(crate) fn record_need_data(&self) {
        self.need_data_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one appsrc `enough-data` emission (the queue reached its bound),
    /// keeping the max observed queue depth (ms) at that moment.
    pub(crate) fn record_enough_data(&self, queue_level_ms: u64) {
        self.enough_data_events.fetch_add(1, Ordering::Relaxed);
        self.max_queue_level_ms
            .fetch_max(queue_level_ms, Ordering::Relaxed);
    }

    /// Record one buffer forwarded out of the appsrc src pad: whether it is
    /// DISCONT-flagged, whether it is a keyframe (`!DELTA_UNIT`), and its
    /// lateness (ms) relative to the consumer pipeline's current running-time
    /// (positive = in the past; negative = in the future = a sync sink holds).
    pub(crate) fn record_buffer(&self, discont: bool, keyframe: bool, lateness_ms: i64) {
        self.total_buffers.fetch_add(1, Ordering::Relaxed);
        if discont {
            self.discont_buffers.fetch_add(1, Ordering::Relaxed);
        }
        if keyframe {
            self.keyframe_buffers.fetch_add(1, Ordering::Relaxed);
        }
        self.lateness_min_ms
            .fetch_min(lateness_ms, Ordering::Relaxed);
        self.lateness_max_ms
            .fetch_max(lateness_ms, Ordering::Relaxed);
        self.lateness_last_ms.store(lateness_ms, Ordering::Relaxed);
    }

    /// Render the diagnostic snapshot (a cheap set of atomic loads plus the
    /// classifier verdict). Lateness min/max/last are `None` until the first
    /// buffer has flowed.
    pub(crate) fn to_snapshot(&self) -> LinkProbeSnapshot {
        let total = self.total_buffers.load(Ordering::Relaxed);
        let enough_data_events = self.enough_data_events.load(Ordering::Relaxed);
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
            enough_data_events,
            need_data_events: self.need_data_events.load(Ordering::Relaxed),
            discont_buffers,
            keyframe_buffers: self.keyframe_buffers.load(Ordering::Relaxed),
            max_queue_level_ms: self.max_queue_level_ms.load(Ordering::Relaxed),
            lateness_ms,
            verdict: classify(enough_data_events, discont_buffers, lateness_for_verdict),
        }
    }
}

/// Lateness (ms) of forwarded buffers relative to the consumer clock, as
/// min/max/last over the session. All `None` until the first buffer.
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
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkProbeSnapshot {
    pub enough_data_events: u64,
    pub need_data_events: u64,
    pub discont_buffers: u64,
    pub keyframe_buffers: u64,
    pub max_queue_level_ms: u64,
    pub lateness_ms: LatenessSnapshot,
    pub verdict: LinkVerdict,
}

/// Attach the #768 D2b diagnostic probe to a consumer `appsrc`: the
/// `need-data`/`enough-data` signals (counters + max queue depth) and a
/// src-pad buffer probe (DISCONT / keyframe / lateness). Creates the
/// [`LinkProbe`] and returns it for storage on the `WhepSession`. The buffer
/// probe holds a WEAK appsrc ref, so it never forms a reference cycle keeping
/// the appsrc alive after the consumer pipeline is torn down.
pub(crate) fn attach(appsrc: &gst_app::AppSrc, session_id: &str) -> Arc<LinkProbe> {
    let probe = Arc::new(LinkProbe::default());
    // Additive: enable signal emission without touching the callback slot.
    appsrc.set_property("emit-signals", true);

    let p_need = probe.clone();
    appsrc.connect("need-data", false, move |_args| {
        p_need.record_need_data();
        None
    });

    let p_enough = probe.clone();
    appsrc.connect("enough-data", false, move |args| {
        let level_ms = args
            .first()
            .and_then(|v| v.get::<gst_app::AppSrc>().ok())
            .map(|src| src.current_level_time().mseconds())
            .unwrap_or(0);
        p_enough.record_enough_data(level_ms);
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
    src_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
        if let Some(buffer) = info.buffer() {
            let flags = buffer.flags();
            let discont = flags.contains(gst::BufferFlags::DISCONT);
            let keyframe = !flags.contains(gst::BufferFlags::DELTA_UNIT);
            let lateness_ms = appsrc_weak
                .upgrade()
                .and_then(|src| {
                    let now = src.upcast_ref::<gst::Element>().current_running_time()?;
                    let pts = buffer.pts()?;
                    Some((now.nseconds() as i64 - pts.nseconds() as i64) / 1_000_000)
                })
                .unwrap_or(0);
            p_buf.record_buffer(discont, keyframe, lateness_ms);

            // Rate-limited (≤ 1 per 5 s per session) so a per-buffer signal
            // never floods the log (`log-flood-backoff.md`).
            let elapsed = base.elapsed().as_millis() as i64;
            let prev = last_log_ms.load(Ordering::Relaxed);
            if elapsed - prev >= 5_000
                && last_log_ms
                    .compare_exchange(prev, elapsed, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
            {
                let snap = p_buf.to_snapshot();
                tracing::debug!(
                    session_id = %sid,
                    enough_data = snap.enough_data_events,
                    need_data = snap.need_data_events,
                    discont = snap.discont_buffers,
                    keyframes = snap.keyframe_buffers,
                    max_queue_ms = snap.max_queue_level_ms,
                    lateness_max_ms = ?snap.lateness_ms.max,
                    verdict = ?snap.verdict,
                    "#768 D2b link probe"
                );
            }
        }
        gst::PadProbeReturn::Ok
    });
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
    fn classify_queue_overflow_on_enough_data_with_lateness() {
        // enough-data fired AND buffers are well behind the consumer clock =
        // the #768 incident signature (downstream drains slower than realtime).
        assert_eq!(classify(50, 3, 400), LinkVerdict::QueueOverflow);
    }

    #[test]
    fn classify_keyframe_wait_on_discont_without_backed_up_queue() {
        // DISCONT-flagged buffers but no sustained overflow (low/negative
        // lateness) = forwarding gated on the next IDR, not a slow consumer.
        assert_eq!(classify(0, 5, -10), LinkVerdict::KeyframeWait);
    }

    #[test]
    fn classify_startup_blip_is_not_overflow() {
        // A brief enough-data blip with near-zero lateness must NOT be read as
        // an overflow; with no DISCONT it is Healthy.
        assert_eq!(classify(1, 0, 20), LinkVerdict::Healthy);
    }

    #[test]
    fn classify_overflow_takes_precedence_over_keyframe_wait() {
        // Both signatures present (enough-data thrash re-arming needs_keyframe →
        // DISCONT at the next IDR): the queue overflow is the root, so it wins.
        assert_eq!(classify(30, 4, 450), LinkVerdict::QueueOverflow);
    }

    #[test]
    fn to_snapshot_empty_probe_is_healthy_with_absent_lateness() {
        let snap = LinkProbe::default().to_snapshot();
        assert_eq!(snap.verdict, LinkVerdict::Healthy);
        assert_eq!(snap.enough_data_events, 0);
        assert!(snap.lateness_ms.min.is_none());
        assert!(snap.lateness_ms.max.is_none());
        assert!(snap.lateness_ms.last.is_none());
    }

    #[test]
    fn to_snapshot_reflects_recorded_overflow() {
        let probe = LinkProbe::default();
        probe.record_need_data();
        probe.record_enough_data(510);
        // A buffer 400 ms in the past = queue backing up.
        probe.record_buffer(true, true, 400);
        probe.record_buffer(false, false, 380);
        let snap = probe.to_snapshot();
        assert_eq!(snap.enough_data_events, 1);
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
    fn snapshot_serializes_camelcase_with_verdict() {
        let probe = LinkProbe::default();
        probe.record_enough_data(300);
        probe.record_buffer(true, false, 450);
        let json = serde_json::to_string(&probe.to_snapshot()).expect("serialize");
        assert!(json.contains("enoughDataEvents"), "camelCase keys: {json}");
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
}
