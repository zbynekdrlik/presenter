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
//! cannot tell the two apart, and the SINCE-JOIN cumulative counters cannot even
//! tell a CURRENT fault from an aged-out join transient.
//!
//! #768 lane 7 therefore makes the verdict a TRAILING-30s WINDOW judgement (see
//! [`classify`] / [`VerdictInputs`]): `Healthy` whenever there are no drops in
//! the window (regardless of historical join drops); `KeyframeWait` when window
//! drops coincide with a pre-first-IDR state or a DISCONT within the window;
//! `QueueOverflow` only for window drops AFTER the first keyframe with the queue
//! near its `max-time` bound and buffers not in the future; `Unknown` when the
//! window is too young to judge. The earlier since-join gate
//! (`dropped()>0 && lateness>250ms`) mis-fired permanently on healthy sessions
//! once lane 6's segment-aware lateness made an ordinary join transient read
//! ~+250 ms — the classifier had inherited a threshold calibrated on the
//! pre-lane-6 wrong lateness (≈ −3600 s).
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

/// appsrc `current-level-time` (ms) at/above which the queue is read as
/// genuinely FULL — 90% of the 500 ms appsrc `max-time` bound. A window drop
/// with the queue this deep AND buffers not in the future is a real
/// downstream-too-slow overflow. (Both a queue OVERFLOW and a keyframe WAIT
/// fill the queue to ~500 ms, so this alone does not discriminate — it is a
/// gate reached only AFTER the keyframe-wait branch is ruled out.)
const QUEUE_FULL_MS: u64 = 450;

/// Lateness (ms) at/above which forwarded buffers are NOT in the future — the
/// shape of a real overflow (buffers at or behind the consumer clock,
/// downstream draining too slowly). A negative value means a buffer is AHEAD of
/// the clock, which a sync sink would hold on rather than drop, so it is not an
/// overflow (#768 lane 7 — replaces the old `> 250 ms` gate, which was
/// calibrated against the pre-lane-6 wrong lateness and mis-routed ordinary
/// join transients into `queueOverflow`).
const LATENESS_NOT_FUTURE_MS: i64 = 0;

/// How a consumer link's drops are best explained (#768). Computed over the
/// trailing 30s window (#768 lane 7), NOT since-join cumulative: an old
/// join-drop episode that has aged out of the window reads `Healthy`, not a
/// permanent fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkVerdict {
    /// The appsrc queue overflowed WITHIN the window (drops after the first
    /// keyframe, no recent DISCONT, queue near its `max-time` bound, buffers not
    /// in the future) — downstream is draining slower than realtime.
    QueueOverflow,
    /// Forwarding was keyframe-gated: drops in the window while still pre-first-
    /// IDR, OR a DISCONT was forwarded within the window (the producer re-armed
    /// `needs_keyframe` and resumed at an IDR) — the consumer is waiting for a
    /// keyframe, not being starved by a slow drain.
    KeyframeWait,
    /// No drops in the trailing window — the link looks healthy, regardless of
    /// any historical join-time drops.
    Healthy,
    /// The window has too few samples to judge recency yet (drops exist
    /// cumulatively but no trailing-window ratio), or drops in the window match
    /// no known signature.
    Unknown,
}

/// Window-scoped inputs to [`classify`] (#768 lane 7). Every field is evaluated
/// over the trailing 30s window (`HealthWindow` / the per-link discont window),
/// except the two cumulative gates that only disambiguate the `< 2 samples`
/// case — the whole point of the recalibration: an old join-drop episode that
/// has aged out of the window must read `Healthy`, not `QueueOverflow`.
#[derive(Debug, Clone, Copy)]
pub struct VerdictInputs {
    /// Dropped-buffer delta over the trailing window (from
    /// `HealthWindow::windowed_delta`). `None` when the window has < 2 samples
    /// (the 30s ratio is null).
    pub window_dropped: Option<u64>,
    /// Cumulative drops since join. Used ONLY when `window_dropped` is `None`,
    /// to tell "nothing ever dropped" (`Healthy`) from "drops happened but the
    /// window can't see them yet" (`Unknown`).
    pub cumulative_dropped: u64,
    /// Cumulative keyframe buffers forwarded. `<= 1` means the session is still
    /// pre-first-IDR (the join phase), where any drop is a keyframe wait.
    pub keyframe_buffers: u64,
    /// A DISCONT buffer was forwarded within the trailing window (forwarding
    /// resumed at an IDR after a gap — a keyframe wait).
    pub discont_in_window: bool,
    /// Max appsrc queue level (ms) seen. Near the 500 ms `max-time` bound = the
    /// queue-full evidence for an overflow.
    pub max_queue_level_ms: u64,
    /// Max forwarded-buffer lateness (ms). `>= 0` = buffers at/behind the
    /// consumer clock (a real overflow drains too slowly); `< 0` = buffers in
    /// the future (a sync sink holds them — not an overflow).
    pub lateness_max_ms: i64,
}

/// Classify a consumer link's drop signature over the trailing 30s window
/// (#768 lane 7). Pure so it is unit-testable on any CI host. The verdict keys
/// off DROPS IN THE WINDOW, never since-join cumulative counters — a healthy
/// session whose only drops were join-time (before its first IDR) reads
/// `Healthy` once those samples age out of the window, instead of the permanent
/// `QueueOverflow` the old cumulative gate produced (see the module + the #768
/// lane 7 design comment).
pub fn classify(inputs: &VerdictInputs) -> LinkVerdict {
    let Some(window_dropped) = inputs.window_dropped else {
        // Window has < 2 samples: it cannot judge recency. If nothing has EVER
        // dropped, the link is plainly healthy; otherwise we do not yet know.
        return if inputs.cumulative_dropped == 0 {
            LinkVerdict::Healthy
        } else {
            LinkVerdict::Unknown
        };
    };
    if window_dropped == 0 {
        // No drops in the trailing window — healthy REGARDLESS of historical
        // join drops (the whole recalibration: an aged-out join episode is not
        // a fault).
        return LinkVerdict::Healthy;
    }
    // Drops IN the window. A keyframe wait is the benign explanation and takes
    // precedence: forwarding is gated on the next IDR, either because the
    // session is still pre-first-IDR (join) or a DISCONT re-armed the wait
    // within the window (a mid-life source glitch resuming at an IDR).
    if inputs.keyframe_buffers <= 1 || inputs.discont_in_window {
        return LinkVerdict::KeyframeWait;
    }
    // Drops after the first keyframe with no recent DISCONT: a genuine overflow
    // iff the queue was near its `max-time` bound AND buffers were not in the
    // future (downstream draining slower than realtime — the #768 incident).
    if inputs.max_queue_level_ms >= QUEUE_FULL_MS
        && inputs.lateness_max_ms >= LATENESS_NOT_FUTURE_MS
    {
        return LinkVerdict::QueueOverflow;
    }
    // Drops after the first keyframe that match no known signature (queue not
    // full, or buffers in the future) — an anomaly, not a named fault.
    LinkVerdict::Unknown
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

/// Belt on the discont-window ring (mirrors `HealthWindow::MAX_SAMPLES`):
/// eviction-by-age already bounds it to `WINDOW / MIN_SAMPLE_SPACING` (~16), so
/// this only guards a pathological clock.
const MAX_DISCONT_SAMPLES: usize = 32;

/// Read-driven trailing-window of the cumulative DISCONT count (#768 lane 7),
/// so the verdict can tell a RECENT keyframe-wait (a DISCONT within the last
/// [`super::health_window::WINDOW`]) from an old join-time one. Sampled at the
/// SAME read-driven cadence and min-spacing as
/// [`super::health_window::HealthWindow`] — recorded on the `/ndi/snapshot`
/// read path (`LinkProbe::to_snapshot`), which is exactly where the verdict is
/// consumed. Pure and side-effect-free (synthetic `Instant`s), the Tier-0 seam
/// discipline of `ndi-manager-locking.md`.
#[derive(Debug, Default)]
struct DiscontWindow {
    samples: Vec<(Instant, u64)>,
}

impl DiscontWindow {
    /// Record the cumulative discont count at `now`, IF at least
    /// [`super::health_window::MIN_SAMPLE_SPACING`] has elapsed since the last
    /// sample (a too-soon call is a no-op — read-driven sampling dedups many
    /// pollers), then evict samples older than
    /// [`super::health_window::WINDOW`]. Mirrors `HealthWindow::record`.
    fn record(&mut self, now: Instant, cumulative_discont: u64) {
        if let Some(&(last_at, _)) = self.samples.last() {
            if now.duration_since(last_at) < super::health_window::MIN_SAMPLE_SPACING {
                return;
            }
        }
        self.samples.push((now, cumulative_discont));
        self.samples
            .retain(|&(at, _)| now.duration_since(at) <= super::health_window::WINDOW);
        if self.samples.len() > MAX_DISCONT_SAMPLES {
            let excess = self.samples.len() - MAX_DISCONT_SAMPLES;
            self.samples.drain(0..excess);
        }
    }

    /// Whether a DISCONT was forwarded within the trailing window: the newest
    /// cumulative count exceeds the oldest in-window one. `None` with < 2
    /// in-window samples — the same null gate as the drop window, so the verdict
    /// treats "can't tell yet" distinctly from "no discont".
    fn saw_discont(&self) -> Option<bool> {
        if self.samples.len() < 2 {
            return None;
        }
        let newest = self.samples.last()?.1;
        let oldest = self.samples.first()?.1;
        Some(newest > oldest)
    }
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
    /// Trailing-window of the cumulative discont count (#768 lane 7), sampled on
    /// the snapshot read path so the verdict can tell a RECENT keyframe wait
    /// (discont within the window) from an old join-time one.
    discont_window: std::sync::Mutex<DiscontWindow>,
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
            discont_window: std::sync::Mutex::new(DiscontWindow::default()),
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
    /// trailing-window classifier verdict). `dropped_buffers` is the
    /// ground-truth drop count from `ConsumptionLink::dropped()`, supplied by
    /// the caller because the appsrc `enough-data` signal is suppressed by
    /// `StreamProducer`'s callback (module doc). `window_dropped` is the
    /// trailing-30s dropped delta from the session's `HealthWindow` (`None`
    /// until >= 2 samples), and `now` is the read instant — used to sample this
    /// link's discont window at the SAME cadence as the drop window so the
    /// verdict's "DISCONT in window" and "drops in window" are aligned
    /// (#768 lane 7). Lateness min/max/last are `None` until the first buffer.
    pub(crate) fn to_snapshot(
        &self,
        dropped_buffers: u64,
        window_dropped: Option<u64>,
        now: Instant,
    ) -> LinkProbeSnapshot {
        let total = self.total_buffers.load(Ordering::Relaxed);
        let discont_buffers = self.discont_buffers.load(Ordering::Relaxed);
        let keyframe_buffers = self.keyframe_buffers.load(Ordering::Relaxed);
        let max_queue_level_ms = self.max_queue_level_ms.load(Ordering::Relaxed);
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
        // Sample the discont window at this read instant (same cadence as the
        // drop window) and read whether a DISCONT landed within the last 30s.
        let discont_in_window = {
            let mut w = self
                .discont_window
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            w.record(now, discont_buffers);
            // `None` (< 2 samples) = no window evidence yet → not a keyframe wait.
            w.saw_discont().unwrap_or(false)
        };
        // The classifier only trusts lateness once a buffer has flowed.
        let lateness_for_verdict = if total == 0 { 0 } else { lateness_max };
        let verdict = classify(&VerdictInputs {
            window_dropped,
            cumulative_dropped: dropped_buffers,
            keyframe_buffers,
            discont_in_window,
            max_queue_level_ms,
            lateness_max_ms: lateness_for_verdict,
        });
        LinkProbeSnapshot {
            dropped_buffers,
            need_data_events: self.need_data_events.load(Ordering::Relaxed),
            discont_buffers,
            keyframe_buffers,
            max_queue_level_ms,
            lateness_ms,
            verdict,
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
                    // Read counters directly for the log line — NOT via
                    // `to_snapshot`, so the discont window (a verdict input) is
                    // only ever sampled on the snapshot READ path, keeping it in
                    // lock-step with the drop window (#768 lane 7). The verdict
                    // itself is computed at snapshot time with dropped().
                    let total = p_buf.total_buffers.load(Ordering::Relaxed);
                    let lateness_max =
                        (total != 0).then(|| p_buf.lateness_max_ms.load(Ordering::Relaxed));
                    tracing::debug!(
                        session_id = %sid,
                        need_data = p_buf.need_data_events.load(Ordering::Relaxed),
                        discont = p_buf.discont_buffers.load(Ordering::Relaxed),
                        keyframes = p_buf.keyframe_buffers.load(Ordering::Relaxed),
                        max_queue_ms = p_buf.max_queue_level_ms.load(Ordering::Relaxed),
                        lateness_max_ms = ?lateness_max,
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

    use std::time::Duration;

    /// A healthy baseline (window present, no drops in it) — each `classify`
    /// test overrides only the fields it exercises (#768 lane 7).
    fn healthy_inputs() -> VerdictInputs {
        VerdictInputs {
            window_dropped: Some(0),
            cumulative_dropped: 0,
            keyframe_buffers: 100,
            discont_in_window: false,
            max_queue_level_ms: 0,
            lateness_max_ms: 0,
        }
    }

    #[test]
    fn classify_healthy_when_no_drops_in_window() {
        assert_eq!(classify(&healthy_inputs()), LinkVerdict::Healthy);
    }

    #[test]
    fn classify_healthy_after_join_ignores_historical_drops() {
        // The #768 lane 7 fix: a long-established session whose ONLY drops were
        // join-time (aged out of the window) — cumulative drops present, queue
        // once full, lateness once ~+256 ms — reads Healthy because the WINDOW
        // is clean. This is the live SNV v0.4.279 incident, recalibrated.
        let inputs = VerdictInputs {
            window_dropped: Some(0),
            cumulative_dropped: 87,
            keyframe_buffers: 179,
            discont_in_window: false,
            max_queue_level_ms: 499,
            lateness_max_ms: 256,
        };
        assert_eq!(classify(&inputs), LinkVerdict::Healthy);
    }

    #[test]
    fn classify_join_drops_in_window_with_discont_is_keyframe_wait() {
        // A fresh session whose join drops are STILL inside the trailing window,
        // with the join DISCONT also in the window (forwarding resumed at the
        // first IDR): keyframe wait, not overflow — even though keyframeBuffers
        // has already climbed past 1 and the join queue was full.
        let inputs = VerdictInputs {
            window_dropped: Some(120),
            cumulative_dropped: 120,
            keyframe_buffers: 5,
            discont_in_window: true,
            max_queue_level_ms: 499,
            lateness_max_ms: 300,
        };
        assert_eq!(classify(&inputs), LinkVerdict::KeyframeWait);
    }

    #[test]
    fn classify_pre_first_idr_drops_is_keyframe_wait() {
        // Still before the first IDR (keyframeBuffers <= 1): any drop is a
        // keyframe wait by definition, even with no DISCONT sampled yet.
        let inputs = VerdictInputs {
            window_dropped: Some(60),
            cumulative_dropped: 60,
            keyframe_buffers: 1,
            discont_in_window: false,
            max_queue_level_ms: 499,
            lateness_max_ms: 300,
        };
        assert_eq!(classify(&inputs), LinkVerdict::KeyframeWait);
    }

    #[test]
    fn classify_mid_life_overflow_burst() {
        // Drops in the window AFTER the first keyframe, no recent DISCONT, queue
        // near its max-time bound, buffers not in the future = the #768 incident
        // signature (downstream draining slower than realtime).
        let inputs = VerdictInputs {
            window_dropped: Some(400),
            cumulative_dropped: 5000,
            keyframe_buffers: 5000,
            discont_in_window: false,
            max_queue_level_ms: 499,
            lateness_max_ms: 300,
        };
        assert_eq!(classify(&inputs), LinkVerdict::QueueOverflow);
    }

    #[test]
    fn classify_discont_burst_is_keyframe_wait() {
        // A mid-life source glitch: window drops AND a DISCONT within the window
        // (forwarding re-gated on the next IDR). The keyframe-wait branch wins
        // over the overflow gate even with the queue full.
        let inputs = VerdictInputs {
            window_dropped: Some(60),
            cumulative_dropped: 60,
            keyframe_buffers: 5000,
            discont_in_window: true,
            max_queue_level_ms: 499,
            lateness_max_ms: 300,
        };
        assert_eq!(classify(&inputs), LinkVerdict::KeyframeWait);
    }

    #[test]
    fn classify_overflow_not_when_buffers_in_future() {
        // Window drops after the first keyframe, queue full, but buffers are
        // AHEAD of the clock (negative lateness) — a sync sink would hold them,
        // not a slow-drain overflow. Unknown, not QueueOverflow.
        let inputs = VerdictInputs {
            window_dropped: Some(400),
            cumulative_dropped: 5000,
            keyframe_buffers: 5000,
            discont_in_window: false,
            max_queue_level_ms: 499,
            lateness_max_ms: -50,
        };
        assert_eq!(classify(&inputs), LinkVerdict::Unknown);
    }

    #[test]
    fn classify_overflow_not_when_queue_not_full() {
        // Window drops after the first keyframe, buffers not in the future, but
        // the queue never approached its max-time bound — the queue was not the
        // bottleneck. An anomaly, not a named overflow.
        let inputs = VerdictInputs {
            window_dropped: Some(400),
            cumulative_dropped: 5000,
            keyframe_buffers: 5000,
            discont_in_window: false,
            max_queue_level_ms: 200,
            lateness_max_ms: 300,
        };
        assert_eq!(classify(&inputs), LinkVerdict::Unknown);
    }

    #[test]
    fn classify_unknown_when_window_too_few_samples_with_cumulative_drops() {
        // A freshly-joined session (< 2 window samples) that already dropped at
        // join: the window can't judge recency yet → Unknown (self-heals to
        // Healthy once its window fills with clean samples).
        let inputs = VerdictInputs {
            window_dropped: None,
            cumulative_dropped: 57,
            keyframe_buffers: 5,
            discont_in_window: false,
            max_queue_level_ms: 499,
            lateness_max_ms: 2152,
        };
        assert_eq!(classify(&inputs), LinkVerdict::Unknown);
    }

    #[test]
    fn classify_healthy_when_window_too_few_samples_no_drops() {
        // < 2 window samples but nothing has ever dropped → plainly Healthy.
        let inputs = VerdictInputs {
            window_dropped: None,
            cumulative_dropped: 0,
            ..healthy_inputs()
        };
        assert_eq!(classify(&inputs), LinkVerdict::Healthy);
    }

    #[test]
    fn verdict_variants_serialize_camelcase() {
        // The shell self-heal gate and the airuleset watchdog read these exact
        // strings off `/ndi/snapshot`; the new `unknown` must round-trip too.
        assert_eq!(
            serde_json::to_string(&LinkVerdict::Unknown).expect("serialize"),
            "\"unknown\""
        );
        assert_eq!(
            serde_json::to_string(&LinkVerdict::QueueOverflow).expect("serialize"),
            "\"queueOverflow\""
        );
        assert_eq!(
            serde_json::to_string(&LinkVerdict::KeyframeWait).expect("serialize"),
            "\"keyframeWait\""
        );
        assert_eq!(
            serde_json::to_string(&LinkVerdict::Healthy).expect("serialize"),
            "\"healthy\""
        );
    }

    #[test]
    fn discont_window_needs_two_samples() {
        let mut w = DiscontWindow::default();
        w.record(Instant::now(), 0);
        assert_eq!(w.saw_discont(), None);
    }

    #[test]
    fn discont_window_true_when_count_climbs() {
        let mut w = DiscontWindow::default();
        let t = Instant::now();
        w.record(t, 0);
        w.record(t + Duration::from_secs(3), 2);
        assert_eq!(w.saw_discont(), Some(true));
    }

    #[test]
    fn discont_window_false_when_count_flat() {
        let mut w = DiscontWindow::default();
        let t = Instant::now();
        w.record(t, 5);
        w.record(t + Duration::from_secs(3), 5);
        assert_eq!(w.saw_discont(), Some(false));
    }

    #[test]
    fn discont_window_min_spacing_suppresses_too_soon() {
        let mut w = DiscontWindow::default();
        let t = Instant::now();
        w.record(t, 0);
        w.record(t + Duration::from_millis(1500), 2); // < 2s → suppressed
        assert_eq!(w.saw_discont(), None, "too-soon sample suppressed");
        w.record(t + Duration::from_secs(3), 2);
        assert_eq!(w.saw_discont(), Some(true));
    }

    #[test]
    fn discont_window_evicts_older_than_window() {
        // An old DISCONT climb ages out; a recent flat window reads no discont.
        let mut w = DiscontWindow::default();
        let t = Instant::now();
        w.record(t, 0);
        w.record(t + Duration::from_secs(3), 5); // the climb (later evicted)
        for k in 2..=13u64 {
            w.record(t + Duration::from_secs(3 * k), 5); // flat, spans past 30s
        }
        assert_eq!(
            w.saw_discont(),
            Some(false),
            "old climb evicted, recent window is flat"
        );
    }

    #[test]
    fn to_snapshot_empty_probe_is_healthy_with_absent_lateness() {
        let snap = LinkProbe::default().to_snapshot(0, None, Instant::now());
        assert_eq!(snap.verdict, LinkVerdict::Healthy);
        assert_eq!(snap.dropped_buffers, 0);
        assert!(snap.lateness_ms.min.is_none());
        assert!(snap.lateness_ms.max.is_none());
        assert!(snap.lateness_ms.last.is_none());
    }

    #[test]
    fn to_snapshot_window_clean_is_healthy_despite_join_stats() {
        // Wiring proof of the recalibration: join stats present on the probe
        // (a DISCONT, high lateness, queue full) but the trailing window is
        // clean (window_dropped == 0) → Healthy, never QueueOverflow.
        let probe = LinkProbe::default();
        probe.record_need_data();
        probe.record_buffer(true, true, 400, 499);
        let snap = probe.to_snapshot(87, Some(0), Instant::now());
        assert_eq!(snap.dropped_buffers, 87);
        assert_eq!(snap.discont_buffers, 1);
        assert_eq!(snap.max_queue_level_ms, 499);
        assert_eq!(snap.lateness_ms.max, Some(400));
        assert_eq!(snap.verdict, LinkVerdict::Healthy);
    }

    #[test]
    fn to_snapshot_overflow_when_window_drops_after_keyframe() {
        // Two keyframe buffers (keyframeBuffers > 1), no DISCONT, queue full,
        // buffers behind the clock; two spaced reads give a 2-sample discont
        // window that saw no DISCONT → QueueOverflow.
        let probe = LinkProbe::default();
        let t0 = Instant::now();
        probe.record_buffer(false, true, 300, 499);
        probe.record_buffer(false, true, 300, 499);
        let _ = probe.to_snapshot(500, Some(400), t0);
        let snap = probe.to_snapshot(500, Some(400), t0 + Duration::from_secs(3));
        assert_eq!(snap.keyframe_buffers, 2);
        assert_eq!(snap.verdict, LinkVerdict::QueueOverflow);
    }

    #[test]
    fn to_snapshot_discont_in_window_drives_keyframe_wait() {
        // keyframeBuffers > 1 and drops in the window, but a DISCONT lands
        // between the two reads → discont-in-window true → KeyframeWait (the
        // discont path, distinct from the pre-first-IDR path).
        let probe = LinkProbe::default();
        let t0 = Instant::now();
        probe.record_buffer(false, true, 300, 499);
        probe.record_buffer(false, true, 300, 499); // keyframeBuffers = 2, discont = 0
        let _ = probe.to_snapshot(100, Some(60), t0);
        probe.record_buffer(true, false, 300, 499); // a mid-life DISCONT
        let snap = probe.to_snapshot(100, Some(60), t0 + Duration::from_secs(3));
        assert_eq!(snap.keyframe_buffers, 2);
        assert_eq!(snap.verdict, LinkVerdict::KeyframeWait);
    }

    #[test]
    fn snapshot_serializes_camelcase_with_verdict() {
        let probe = LinkProbe::default();
        let t0 = Instant::now();
        probe.record_buffer(false, true, 450, 499);
        probe.record_buffer(false, true, 450, 499);
        let _ = probe.to_snapshot(500, Some(400), t0);
        let json =
            serde_json::to_string(&probe.to_snapshot(500, Some(400), t0 + Duration::from_secs(3)))
                .expect("serialize");
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
