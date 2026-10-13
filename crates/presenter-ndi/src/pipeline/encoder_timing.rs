//! Shared-encoder timestamp guard + lead meter (#854).
//!
//! Root cause this guards against: GStreamer 1.24.2 `vah264enc` (the encoder on
//! both prod sites) stamps its OUTPUT as `start_pts + frame_duration ×
//! frame_count` — a frame counter at the nominal caps rate — and throws away the
//! receive-time PTS that `ndisrc timestamp-mode=receive-time` put on the input.
//! When the NDI source runs even slightly faster than its declared rate by this
//! server's clock (PP: +48…+106 ppm, NTP off), that counter runs ahead of the
//! pipeline clock: ~173 ms per hour at +48 ppm. webrtcbin inserts a
//! `clocksync sync=true` on every input stream, so future-stamped buffers are
//! HELD there; once the lead passes the consumer appsrc's 500 ms `max-time`
//! every consumer queue overflows once per GOP and the TVs get broken GOPs.
//! A source framerate DROP rescales the same counter into a forward jump; a
//! framerate RISE rescales it backwards.
//!
//! The guard (a pad probe on the shared encoder's `src` pad) enforces ONE rule:
//! **no frame leaves the encoder stamped more than D = 50 ms
//! ([`TIMESTAMP_DEADBAND_MS`]) later than the pipeline clock.** It keeps the
//! encoder's own stamps and pulls its timeline back by a correction that only
//! grows when a frame would otherwise lead the clock by more than D, and then
//! only by the part above D ([`TimestampGuard::apply`]). The maths is done in
//! RUNNING TIME and converted back through the encoder's OUTPUT segment — never
//! by copying a raw PTS: the output segment carries GstVideoEncoder's
//! `time_adjustment` (vah264enc sets a 1000 h `min_pts`), the class of mistake
//! behind #768's constant −3600 s read.
//!
//! - x264enc (CI), NVENC (dev2), vah264enc ≥ 1.26: their stamps ARE the
//!   receive time, always earlier than the clock → exact identity.
//! - vah264enc 1.24 on SNV (prod, Intel N100): the counter is anchored at the
//!   first frame's arrival, so frames that arrive earlier than it lead the
//!   clock by the NDI arrival jitter (≤ ~40 ms minus the encode latency). That
//!   stays under D, so the guard is an identity there as well: SNV keeps exactly
//!   the pacing it had before the guard (clocksync still holds an early frame
//!   for its lead, as it always did). Only a source that ALSO drifts fast could
//!   push SNV's lead past D, and then the same cap as on PP applies.
//! - vah264enc 1.24 on PP (+48…+106 ppm): an identity until the drift brings
//!   the lead to D (~18 min of pipeline age at +48 ppm), then the lead is
//!   capped at D: a clocksync hold of ≤ D (~1.5 frames), i.e. ≈ +D of latency,
//!   never a queue overflow.
//! - A backward jump (a framerate rise) re-anchors the output one frame after
//!   the previous one; the correction then goes NEGATIVE (the encoder's stamps
//!   sit behind the output), which `encoderTimestampCorrectionMs` shows as < 0.
//!
//! The lead meter (a probe on the producer appsink's `sink` pad, after
//! h264parse, independent of the guard) records how far each shared buffer is
//! stamped ahead of the clock into a 30 s window: `/healthz`
//! `timestampLeadMs30s` and the supervisor's self-heal backstop read it.
//!
//! Untouched on purpose (refused or failed before, see #854): the system-clock
//! pin, `ntp-time-source=clock-time`, the 500 ms consumer queue, GOP, profile,
//! encoder count and bitrate; nothing per consumer (≠ the #373 per-consumer
//! `do-timestamp` attempt).

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;

use super::time_window::SecondBuckets;

const NS_PER_MS: i64 = 1_000_000;

/// The correction-growth INFO line fires each time the correction crosses
/// another multiple of this (ms): ~2 lines/hour on PP's +48 ppm source, none
/// where the encoder keeps the input PTS or the lead stays within D.
const CORRECTION_LOG_STEP_MS: i64 = 100;

/// The guard's deadband D (#854 ROZHODNUTÉ): a frame may leave the shared
/// encoder stamped at most this many ms LATER than the pipeline clock; only the
/// part of a lead above D is pulled back.
///
/// Why 50 ms: SNV's vah264enc 1.24 frame counter leads the clock by up to the
/// NDI arrival jitter (≤ ~40 ms, minus the encode latency), so with D above
/// that the guard never touches a frame on SNV, by construction. On PP it caps
/// the drift lead at D: a clocksync hold of ≤ D (~1.5 frames), far below the
/// 500 ms consumer queue that overflowed. The drift self-heal trigger (300 ms
/// lead) and the `futureTimestamps` verdict (≤ −100 ms lateness) both stay
/// above it, so neither can fire on a lead the guard allows.
pub(crate) const TIMESTAMP_DEADBAND_MS: i64 = 50;

/// [`TIMESTAMP_DEADBAND_MS`] in nanoseconds (the guard's running-time unit).
pub(crate) const TIMESTAMP_DEADBAND_NS: i64 = TIMESTAMP_DEADBAND_MS * NS_PER_MS;

/// What the guard did with one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuardKind {
    /// The encoder's own stamp, shifted back by the current correction (0 on a
    /// PTS-preserving encoder) — the encoder's cadence is kept.
    Kept,
    /// The stamp would have led the clock by more than D: clamped to clock + D
    /// and the correction grew by the excess.
    PulledBack,
    /// The encoder's timeline went backwards or stalled: re-anchored one frame
    /// after the previous output (never more than D ahead of the clock).
    Reanchored,
}

/// The guard's verdict for one frame: the output running time and how it was
/// reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuardDecision {
    pub(crate) out_ns: i64,
    pub(crate) kind: GuardKind,
}

/// Pure guard state: the ratcheting correction and the last output running
/// time. Everything is in running-time nanoseconds; no GStreamer types, so the
/// policy is unit-tested with plain integers.
#[derive(Debug, Default)]
pub(crate) struct TimestampGuard {
    correction_ns: i64,
    last_out_ns: Option<i64>,
}

impl TimestampGuard {
    /// Decide the output running time for a frame the encoder stamped `enc_ns`,
    /// leaving the encoder when the clock reads `now_ns`. `duration_ns` (0 when
    /// unknown) spaces a re-anchored frame from its predecessor.
    ///
    /// Invariants: `out ≤ now + D` (except a 1 ns monotonic nudge when two
    /// frames leave at the same clock reading) and `out` strictly increases. A
    /// lead up to D is left alone; the correction only grows on `PulledBack`
    /// (by the part of the lead above D), so `Kept` frames keep the encoder's
    /// own spacing exactly.
    pub(crate) fn apply(&mut self, enc_ns: i64, now_ns: i64, duration_ns: i64) -> GuardDecision {
        let limit = now_ns + TIMESTAMP_DEADBAND_NS;
        let candidate = enc_ns - self.correction_ns;
        let (mut out_ns, kind) = if candidate > limit {
            (limit, GuardKind::PulledBack)
        } else {
            match self.last_out_ns {
                Some(last) if candidate <= last => (
                    (last + duration_ns.max(0)).min(limit),
                    GuardKind::Reanchored,
                ),
                _ => (candidate, GuardKind::Kept),
            }
        };
        if let Some(last) = self.last_out_ns {
            if out_ns <= last {
                out_ns = last + 1;
            }
        }
        self.correction_ns = enc_ns - out_ns;
        self.last_out_ns = Some(out_ns);
        GuardDecision { out_ns, kind }
    }

    /// How far (ns) the encoder's own timeline is pulled back. Positive = the
    /// encoder ran more than D ahead of the clock (the old code would have held
    /// it D longer than now); negative after a backward re-anchor.
    pub(crate) fn correction_ns(&self) -> i64 {
        self.correction_ns
    }

    /// Forget the correction and the last output (FLUSH_STOP: running time may
    /// restart). The first frame after a reset is re-clamped if needed.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Nanoseconds of a clock time as a signed value (running times stay far below
/// `i64::MAX`, ~292 years).
fn clock_ns(t: gst::ClockTime) -> i64 {
    t.nseconds() as i64
}

/// `dts` shifted by the same amount the PTS moved, so `pts − dts` is preserved
/// (0 for constrained-baseline; defensive for any reordering encoder).
fn shifted_dts(
    dts: Option<gst::ClockTime>,
    old_pts: gst::ClockTime,
    new_pts: gst::ClockTime,
) -> Option<gst::ClockTime> {
    let dts = dts?;
    let shifted = clock_ns(dts) - (clock_ns(old_pts) - clock_ns(new_pts));
    Some(gst::ClockTime::from_nseconds(shifted.max(0) as u64))
}

/// One buffer's outcome inside the guard probe.
#[derive(Debug, PartialEq, Eq)]
enum GuardStep {
    /// No segment yet, the PTS is outside the segment, or no clock yet: the
    /// buffer passes through untouched.
    Unmeasured,
    /// The guard ran. `restamp` is `None` when the stamp is unchanged.
    Decided {
        raw_lead_ns: i64,
        correction_ns: i64,
        kind: GuardKind,
        restamp: Option<(gst::ClockTime, Option<gst::ClockTime>)>,
    },
}

/// Guard probe state: the pure guard plus the encoder's OUTPUT segment (from
/// the downstream SEGMENT event; it carries the encoder's time adjustment).
#[derive(Debug, Default)]
struct GuardState {
    guard: TimestampGuard,
    segment: Option<gst::FormattedSegment<gst::ClockTime>>,
}

impl GuardState {
    /// Run the guard on one buffer's timestamps: PTS → running time through the
    /// output segment, decide, then running time → PTS through the SAME segment.
    fn process(
        &mut self,
        pts: gst::ClockTime,
        dts: Option<gst::ClockTime>,
        duration: Option<gst::ClockTime>,
        now: Option<gst::ClockTime>,
    ) -> GuardStep {
        let Some(segment) = self.segment.as_ref() else {
            return GuardStep::Unmeasured;
        };
        let (Some(enc_rt), Some(now)) = (segment.to_running_time(Some(pts)), now) else {
            return GuardStep::Unmeasured;
        };
        let enc_ns = clock_ns(enc_rt);
        let now_ns = clock_ns(now);
        let decision = self
            .guard
            .apply(enc_ns, now_ns, duration.map_or(0, clock_ns));
        let restamp = if decision.out_ns == enc_ns {
            None
        } else {
            let new_pts = u64::try_from(decision.out_ns).ok().and_then(|ns| {
                segment.position_from_running_time(Some(gst::ClockTime::from_nseconds(ns)))
            });
            let Some(new_pts) = new_pts else {
                return GuardStep::Unmeasured;
            };
            Some((new_pts, shifted_dts(dts, pts, new_pts)))
        };
        GuardStep::Decided {
            raw_lead_ns: enc_ns - now_ns,
            correction_ns: self.guard.correction_ns(),
            kind: decision.kind,
            restamp,
        }
    }
}

/// Lead meter state: the producer appsink's segment plus the 30 s window of
/// per-buffer leads (ms, positive = stamped in the future).
#[derive(Debug, Default)]
struct LeadState {
    segment: Option<gst::FormattedSegment<gst::ClockTime>>,
    window: SecondBuckets,
}

/// Serialized `/ndi/snapshot` `encoderTiming` block (#854).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderTimingSnapshot {
    /// Guard AND lead meter installed (both pads found).
    pub installed: bool,
    pub encoder_factory: String,
    /// Encoder output buffers with a valid PTS seen by the guard.
    pub frames_seen: u64,
    /// Frames clamped to clock + D (their stamp led the clock by more than D).
    pub frames_pulled_back: u64,
    /// Frames re-anchored after the encoder timeline went backwards/stalled.
    pub frames_reanchored: u64,
    /// Frames the guard could not measure (no segment / clock yet).
    pub frames_unmeasured: u64,
    /// How far the encoder's own timeline is pulled back (ms). On a
    /// fast-drifting vah264enc 1.24 source this is the lead the unguarded
    /// pipeline would have built, minus D; 0 on PTS-preserving encoders and
    /// while the lead stays within D (SNV); negative after a backward
    /// re-anchor (a framerate rise).
    pub correction_ms: i64,
    /// Encoder stamp − clock for the last measured frame (ms), BEFORE the guard.
    pub raw_lead_last_ms: Option<i64>,
    /// Maximum of the above since the pipeline was built.
    pub raw_lead_max_ms: Option<i64>,
    /// Trailing-30s lead at the producer appsink (after the guard). `null`
    /// without frames in the window. Healthy: max ≤ D (50); ≤ 0 on
    /// PTS-preserving encoders.
    pub lead_ms_30s: Option<LeadWindow>,
}

/// Min/max of a 30 s lead window (ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LeadWindow {
    pub min: i64,
    pub max: i64,
}

/// Per-pipeline timing state shared by the two probes and the readers
/// (`/healthz`, `/ndi/snapshot`, the supervisor's drift check). Counters are
/// atomics; the two small mutexes are taken by one streaming thread each
/// (uncontended) plus the occasional reader. A poisoned lock is recovered,
/// never propagated (a diagnostic must not poison a streaming thread).
#[derive(Debug)]
pub struct EncoderTiming {
    encoder_factory: String,
    label: String,
    installed: AtomicBool,
    origin: Instant,
    guard: Mutex<GuardState>,
    lead: Mutex<LeadState>,
    frames_seen: AtomicU64,
    frames_pulled_back: AtomicU64,
    frames_reanchored: AtomicU64,
    frames_unmeasured: AtomicU64,
    buffer_lists: AtomicU64,
    correction_ns: AtomicI64,
    raw_lead_last_ns: AtomicI64,
    /// `i64::MIN` until the first measured frame.
    raw_lead_max_ns: AtomicI64,
    /// Highest correction multiple of [`CORRECTION_LOG_STEP_MS`] already logged.
    logged_correction_steps: AtomicI64,
}

impl EncoderTiming {
    /// Detached state (no probes): what [`install`] starts from, and what the
    /// test-only pipeline constructors carry (`installed = false`).
    pub(crate) fn new(encoder_factory: String, label: String) -> Self {
        Self {
            encoder_factory,
            label,
            installed: AtomicBool::new(false),
            origin: Instant::now(),
            guard: Mutex::new(GuardState::default()),
            lead: Mutex::new(LeadState::default()),
            frames_seen: AtomicU64::new(0),
            frames_pulled_back: AtomicU64::new(0),
            frames_reanchored: AtomicU64::new(0),
            frames_unmeasured: AtomicU64::new(0),
            buffer_lists: AtomicU64::new(0),
            correction_ns: AtomicI64::new(0),
            raw_lead_last_ns: AtomicI64::new(0),
            raw_lead_max_ns: AtomicI64::new(i64::MIN),
            logged_correction_steps: AtomicI64::new(0),
        }
    }

    /// Both probes are installed.
    pub(crate) fn is_installed(&self) -> bool {
        self.installed.load(Ordering::Relaxed)
    }

    /// Current guard correction in ms (`/healthz` `encoderTimestampCorrectionMs`).
    pub(crate) fn correction_ms(&self) -> i64 {
        self.correction_ns.load(Ordering::Relaxed) / NS_PER_MS
    }

    /// `(min, max)` lead (ms) at the producer appsink over the trailing `span`;
    /// `None` when no frame arrived in it (a source stall, or not started).
    pub(crate) fn lead_window_ms(&self, span: Duration) -> Option<(i64, i64)> {
        let now_sec = self.origin.elapsed().as_secs();
        self.lead
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .window
            .extrema(now_sec, span.as_secs())
    }

    /// The serialized `/ndi/snapshot` block.
    pub fn snapshot(&self) -> EncoderTimingSnapshot {
        let raw_last = self.raw_lead_last_ns.load(Ordering::Relaxed);
        let raw_max = self.raw_lead_max_ns.load(Ordering::Relaxed);
        let measured = raw_max != i64::MIN;
        EncoderTimingSnapshot {
            installed: self.is_installed(),
            encoder_factory: self.encoder_factory.clone(),
            frames_seen: self.frames_seen.load(Ordering::Relaxed),
            frames_pulled_back: self.frames_pulled_back.load(Ordering::Relaxed),
            frames_reanchored: self.frames_reanchored.load(Ordering::Relaxed),
            frames_unmeasured: self.frames_unmeasured.load(Ordering::Relaxed),
            correction_ms: self.correction_ms(),
            raw_lead_last_ms: measured.then_some(raw_last / NS_PER_MS),
            raw_lead_max_ms: measured.then_some(raw_max / NS_PER_MS),
            lead_ms_30s: self
                .lead_window_ms(super::health_window::WINDOW)
                .map(|(min, max)| LeadWindow { min, max }),
        }
    }

    /// Guard probe, events: store the output segment; FLUSH_STOP resets the
    /// guard (running time may restart after a flush).
    fn on_guard_event(&self, event: &gst::Event) {
        match event.view() {
            gst::EventView::Segment(seg_ev) => {
                if let Some(seg) = seg_ev.segment().downcast_ref::<gst::ClockTime>() {
                    self.guard.lock().unwrap_or_else(|p| p.into_inner()).segment =
                        Some(seg.clone());
                }
            }
            gst::EventView::FlushStop(_) => {
                self.guard
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .guard
                    .reset();
                tracing::info!(
                    encoder = %self.encoder_factory,
                    source = %self.label,
                    "NDI encoder flushed — timestamp guard reset (#854)"
                );
            }
            _ => {}
        }
    }

    /// Guard probe, buffers: run the guard and re-stamp the buffer in place when
    /// the stamp changes (a metadata copy; the memory stays shared). A buffer
    /// without a PTS (e.g. a header buffer) passes untouched.
    fn on_guard_buffer(&self, buffer: &mut gst::Buffer, now: Option<gst::ClockTime>) {
        let Some(pts) = buffer.pts() else {
            return;
        };
        self.frames_seen.fetch_add(1, Ordering::Relaxed);
        let step = self
            .guard
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .process(pts, buffer.dts(), buffer.duration(), now);
        match step {
            GuardStep::Unmeasured => self.note_unmeasured(),
            GuardStep::Decided {
                raw_lead_ns,
                correction_ns,
                kind,
                restamp,
            } => {
                self.record_decision(raw_lead_ns, correction_ns, kind);
                if let Some((new_pts, new_dts)) = restamp {
                    let buf = buffer.make_mut();
                    buf.set_pts(new_pts);
                    buf.set_dts(new_dts);
                }
            }
        }
    }

    /// Stats + logging for one decided frame. The correction-growth INFO is
    /// gated on 100 ms multiples; the re-anchor WARN on power-of-two counts
    /// (`log-flood-backoff.md`).
    fn record_decision(&self, raw_lead_ns: i64, correction_ns: i64, kind: GuardKind) {
        self.raw_lead_last_ns.store(raw_lead_ns, Ordering::Relaxed);
        self.raw_lead_max_ns
            .fetch_max(raw_lead_ns, Ordering::Relaxed);
        self.correction_ns.store(correction_ns, Ordering::Relaxed);
        match kind {
            GuardKind::Kept => {}
            GuardKind::PulledBack => {
                self.frames_pulled_back.fetch_add(1, Ordering::Relaxed);
                let steps = correction_ns / (CORRECTION_LOG_STEP_MS * NS_PER_MS);
                if steps > 0
                    && self
                        .logged_correction_steps
                        .fetch_max(steps, Ordering::Relaxed)
                        < steps
                {
                    // A pulled-back frame leaves at clock + D, so its lead
                    // was the correction + D.
                    tracing::info!(
                        encoder = %self.encoder_factory,
                        source = %self.label,
                        correction_ms = correction_ns / NS_PER_MS,
                        deadband_ms = TIMESTAMP_DEADBAND_MS,
                        "NDI encoder timeline would be +{} ms ahead of the clock — held back to \
                         +{} ms (#854)",
                        correction_ns / NS_PER_MS + TIMESTAMP_DEADBAND_MS,
                        TIMESTAMP_DEADBAND_MS
                    );
                }
            }
            GuardKind::Reanchored => {
                let n = self.frames_reanchored.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_power_of_two() {
                    tracing::warn!(
                        encoder = %self.encoder_factory,
                        source = %self.label,
                        reanchored = n,
                        correction_ms = correction_ns / NS_PER_MS,
                        "NDI encoder timeline went backwards or stalled — output re-anchored (#854)"
                    );
                }
            }
        }
    }

    fn note_unmeasured(&self) {
        let n = self.frames_unmeasured.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_power_of_two() {
            tracing::warn!(
                encoder = %self.encoder_factory,
                source = %self.label,
                unmeasured = n,
                "NDI encoder timestamp guard could not measure a frame (no segment, PTS \
                 outside the segment, or no clock yet) — passed through unguarded (#854)"
            );
        }
    }

    fn note_buffer_list(&self) {
        if self.buffer_lists.fetch_add(1, Ordering::Relaxed) == 0 {
            tracing::warn!(
                encoder = %self.encoder_factory,
                source = %self.label,
                "NDI encoder pushed a buffer list — passed through unguarded; the lead \
                 meter and the self-heal stay the backstop (#854)"
            );
        }
    }

    /// Lead meter, events: store the appsink's segment.
    fn on_lead_event(&self, event: &gst::Event) {
        if let gst::EventView::Segment(seg_ev) = event.view() {
            if let Some(seg) = seg_ev.segment().downcast_ref::<gst::ClockTime>() {
                self.lead.lock().unwrap_or_else(|p| p.into_inner()).segment = Some(seg.clone());
            }
        }
    }

    /// Lead meter, buffers: record `running_time(pts) − now` (ms) in the window.
    fn on_lead_buffer(&self, pts: Option<gst::ClockTime>, now: Option<gst::ClockTime>) {
        let mut state = self.lead.lock().unwrap_or_else(|p| p.into_inner());
        let running = state.segment.as_ref().and_then(|s| s.to_running_time(pts));
        let (Some(running), Some(now)) = (running, now) else {
            return;
        };
        let lead_ms = (clock_ns(running) - clock_ns(now)) / NS_PER_MS;
        state
            .window
            .record(self.origin.elapsed().as_secs(), lead_ms);
    }
}

/// Install the guard on `encoder`'s `src` pad and the lead meter on
/// `appsink`'s `sink` pad, BEFORE the pipeline leaves NULL (so both see the
/// sticky SEGMENT). `label` names the source in the logs. A missing pad is an
/// ERROR and leaves `installed = false` (visible on `/healthz`).
pub(crate) fn install(
    encoder: &gst::Element,
    appsink: &gst::Element,
    label: &str,
) -> Arc<EncoderTiming> {
    let factory = encoder
        .factory()
        .map(|f| f.name().to_string())
        .unwrap_or_default();
    let timing = Arc::new(EncoderTiming::new(factory, label.to_string()));
    let guard_ok = install_guard_probe(&timing, encoder);
    let meter_ok = install_lead_meter(&timing, appsink);
    if guard_ok && meter_ok {
        timing.installed.store(true, Ordering::Relaxed);
        tracing::info!(
            encoder = %timing.encoder_factory,
            source = %label,
            "NDI encoder timestamp guard installed on {} (#854)",
            timing.encoder_factory
        );
    } else {
        tracing::error!(
            encoder = %timing.encoder_factory,
            source = %label,
            guard = guard_ok,
            lead_meter = meter_ok,
            "NDI encoder timestamp guard NOT installed (pad missing) — shared encoder \
             timestamps are unguarded (#854)"
        );
    }
    timing
}

fn install_guard_probe(timing: &Arc<EncoderTiming>, encoder: &gst::Element) -> bool {
    let Some(pad) = encoder.static_pad("src") else {
        return false;
    };
    let timing = Arc::clone(timing);
    let encoder_weak = encoder.downgrade();
    pad.add_probe(
        gst::PadProbeType::BUFFER
            | gst::PadProbeType::BUFFER_LIST
            | gst::PadProbeType::EVENT_DOWNSTREAM
            | gst::PadProbeType::EVENT_FLUSH,
        move |_pad, info| {
            if let Some(event) = info.event() {
                timing.on_guard_event(event);
                return gst::PadProbeReturn::Ok;
            }
            if info.buffer_list().is_some() {
                timing.note_buffer_list();
                return gst::PadProbeReturn::Ok;
            }
            let now = encoder_weak
                .upgrade()
                .and_then(|enc| enc.current_running_time());
            if let Some(buffer) = info.buffer_mut() {
                timing.on_guard_buffer(buffer, now);
            }
            gst::PadProbeReturn::Ok
        },
    )
    .is_some()
}

fn install_lead_meter(timing: &Arc<EncoderTiming>, appsink: &gst::Element) -> bool {
    let Some(pad) = appsink.static_pad("sink") else {
        return false;
    };
    let timing = Arc::clone(timing);
    let sink_weak = appsink.downgrade();
    pad.add_probe(
        gst::PadProbeType::BUFFER | gst::PadProbeType::EVENT_DOWNSTREAM,
        move |_pad, info| {
            if let Some(event) = info.event() {
                timing.on_lead_event(event);
                return gst::PadProbeReturn::Ok;
            }
            if let Some(buffer) = info.buffer() {
                let now = sink_weak
                    .upgrade()
                    .and_then(|sink| sink.current_running_time());
                timing.on_lead_buffer(buffer.pts(), now);
            }
            gst::PadProbeReturn::Ok
        },
    )
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME_NS: i64 = 33_333_333;
    const MS: i64 = NS_PER_MS;
    /// The deadband D: how far a frame may leave ahead of the clock.
    const D: i64 = TIMESTAMP_DEADBAND_NS;

    /// Deterministic xorshift for jitter (no RNG dependency).
    struct XorShift(u64);
    impl XorShift {
        fn next_u64(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        /// Uniform in `-max_abs_ns..=max_abs_ns`.
        fn next_jitter_ns(&mut self, max_abs_ns: i64) -> i64 {
            (self.next_u64() % (2 * max_abs_ns as u64 + 1)) as i64 - max_abs_ns
        }

        /// Uniform in `lo_ns..=hi_ns`.
        fn next_range_ns(&mut self, lo_ns: i64, hi_ns: i64) -> i64 {
            lo_ns + (self.next_u64() % ((hi_ns - lo_ns) as u64 + 1)) as i64
        }
    }

    #[test]
    fn fast_frame_counter_never_leads_the_clock_by_more_than_the_deadband() {
        // vah264enc 1.24 on a source delivering 40 fps against a 30 fps counter:
        // the counter gains 8.3 ms per frame on the clock.
        let mut g = TimestampGuard::default();
        let mut prev: Option<i64> = None;
        for n in 0..400i64 {
            let enc = n * FRAME_NS;
            let now = n * 25 * MS + 5 * MS;
            let d = g.apply(enc, now, FRAME_NS);
            assert!(
                d.out_ns <= now + D,
                "frame {n}: out {} more than D ahead of now {now}",
                d.out_ns
            );
            if let Some(p) = prev {
                assert!(d.out_ns > p, "frame {n}: output not strictly increasing");
            }
            prev = Some(d.out_ns);
            // Frames 0..=6 lead by at most 45 ms (n × 8.33 ms − 5 ms): inside
            // D, left exactly as the encoder stamped them.
            let expected = if n < 7 {
                GuardKind::Kept
            } else {
                GuardKind::PulledBack
            };
            assert_eq!(d.kind, expected, "frame {n}");
        }
        // The correction is the lead the unguarded pipeline would have built,
        // minus D: 399 frames × 8.33 ms − 5 ms encode latency − 50 ms ≈ 3.27 s.
        let corr_ms = g.correction_ns() / MS;
        assert!((3250..=3290).contains(&corr_ms), "correction {corr_ms} ms");
    }

    #[test]
    fn snv_arrival_jitter_inside_the_deadband_is_an_exact_identity() {
        // SNV (prod, Intel N100) runs vah264enc 1.24 too. Its counter is
        // anchored at the FIRST frame's arrival, so when that frame arrived
        // late, every frame that arrives earlier than it leads the clock by up
        // to the arrival jitter (0–40 ms) minus the encode latency (3–8 ms).
        // Source on the server clock (0 ppm), one hour at 30 fps. With D the
        // guard must leave every frame exactly as the encoder stamped it: SNV
        // stays bit-identical to the pipeline it had before the guard.
        let mut g = TimestampGuard::default();
        let mut rng = XorShift(0x854_5A1);
        // The worst case: the first frame arrived as late as the jitter gets.
        let first_arrival = 40 * MS;
        let mut max_lead = i64::MIN;
        for n in 0..108_000i64 {
            let arrival = if n == 0 {
                first_arrival
            } else {
                n * FRAME_NS + rng.next_range_ns(0, 40 * MS)
            };
            let enc = first_arrival + n * FRAME_NS;
            let now = arrival + rng.next_range_ns(3 * MS, 8 * MS);
            max_lead = max_lead.max(enc - now);
            let d = g.apply(enc, now, FRAME_NS);
            assert_eq!(d.out_ns, enc, "frame {n} re-stamped");
            assert_eq!(d.kind, GuardKind::Kept, "frame {n}");
        }
        assert_eq!(g.correction_ns(), 0);
        // The input really carried SNV's early-frame lead (≈ 40 − 3 ms), so
        // the identity above is not vacuous: without the deadband those
        // frames would have been pulled back.
        assert!(
            (35 * MS..D).contains(&max_lead),
            "max lead {} µs",
            max_lead / 1_000
        );
    }

    #[test]
    fn pts_preserving_encoder_is_an_exact_identity() {
        // x264enc / NVENC / vah264enc ≥ 1.26: the stamp IS the receive time,
        // always earlier than the clock — the guard must change nothing.
        let mut g = TimestampGuard::default();
        let mut rng = XorShift(0x854);
        for n in 0..10_000i64 {
            let receive = n * FRAME_NS + rng.next_jitter_ns(15 * MS) + 20 * MS;
            let now = receive + 4 * MS + rng.next_jitter_ns(2 * MS).abs();
            let d = g.apply(receive, now, FRAME_NS);
            assert_eq!(d.out_ns, receive, "frame {n} re-stamped");
            assert_eq!(d.kind, GuardKind::Kept, "frame {n}");
        }
        assert_eq!(g.correction_ns(), 0);
    }

    #[test]
    fn slow_source_counter_is_left_alone() {
        // PP's Sunday −61 ppm: the counter falls BEHIND the clock (late
        // stamps, harmless) — identity, exactly today's behaviour.
        let mut g = TimestampGuard::default();
        let real_period = FRAME_NS + 2_033; // −61 ppm source
        for n in 0..10_000i64 {
            let enc = n * FRAME_NS;
            let now = n * real_period + 6 * MS;
            let d = g.apply(enc, now, FRAME_NS);
            assert_eq!(d.out_ns, enc, "frame {n}");
        }
        assert_eq!(g.correction_ns(), 0);
    }

    #[test]
    fn pp_drift_lead_is_capped_at_the_deadband() {
        // PP: +48 ppm (1.6 µs/frame) with ±5 ms arrival jitter over 100 000
        // frames (~56 min). Every frame is left alone until the drift brings
        // the lead to D (~15 min in); from then on the lead is capped at D:
        // never more than D ahead of the clock, every Kept frame exactly one
        // counter period after its predecessor, and each pulled-back frame
        // (a new earliest arrival) shortened by well under 1 ms of its 33.3 ms.
        let mut g = TimestampGuard::default();
        let mut rng = XorShift(0x2026_1013);
        let real_period = FRAME_NS - 1_600;
        let first_arrival = rng.next_jitter_ns(5 * MS);
        let mut prev: Option<i64> = None;
        let mut pulled_back = 0u32;
        let mut first_pulled_back: Option<i64> = None;
        let mut max_out_lead = i64::MIN;
        for n in 0..100_000i64 {
            let enc = first_arrival + n * FRAME_NS;
            let now = n * real_period + rng.next_jitter_ns(5 * MS) + 3 * MS;
            let d = g.apply(enc, now, FRAME_NS);
            max_out_lead = max_out_lead.max(d.out_ns - now);
            if let Some(p) = prev {
                let interval = d.out_ns - p;
                match d.kind {
                    GuardKind::Kept => {
                        assert_eq!(interval, FRAME_NS, "frame {n}: cadence changed")
                    }
                    GuardKind::PulledBack => {
                        pulled_back += 1;
                        first_pulled_back = first_pulled_back.or(Some(n));
                        assert!(
                            interval >= FRAME_NS - MS,
                            "frame {n}: pulled back by {} µs",
                            (FRAME_NS - interval) / 1_000
                        );
                    }
                    GuardKind::Reanchored => panic!("frame {n}: unexpected re-anchor"),
                }
            }
            prev = Some(d.out_ns);
        }
        // Capped AT D (the drift really reached it), never above it.
        assert_eq!(max_out_lead, D, "max output lead {max_out_lead} ns");
        // Untouched while the lead (4 ms first-frame jitter + n × 1.6 µs +
        // ≤ 5 ms jitter − 3 ms encode latency) is still under D.
        assert!(
            first_pulled_back.is_some_and(|n| n > 27_000),
            "first pulled-back frame {first_pulled_back:?}"
        );
        // drift (100 000 × 1.6 µs = 160 ms) + 4 ms first-frame jitter + the
        // ≤ 5 ms early-jitter envelope − 3 ms encode latency − D ≈ 116 ms.
        let corr_ms = g.correction_ns() / MS;
        assert!((108..=122).contains(&corr_ms), "correction {corr_ms} ms");
        assert!(
            pulled_back < 1_500,
            "{pulled_back} of 100000 frames pulled back — cadence not preserved"
        );
    }

    #[test]
    fn forward_jump_is_absorbed_into_the_correction() {
        // The 1.24 framerate-rescale bug after a framerate DROP (a longer
        // frame duration × the whole frame count): the counter jumps +10 s.
        let mut g = TimestampGuard::default();
        for n in 0..30i64 {
            g.apply(n * FRAME_NS, n * FRAME_NS + 5 * MS, FRAME_NS);
        }
        let jump = 10_000 * MS;
        let mut prev = 29 * FRAME_NS;
        for n in 30..90i64 {
            let enc = n * FRAME_NS + jump;
            let now = n * FRAME_NS + 5 * MS;
            let d = g.apply(enc, now, FRAME_NS);
            assert!(d.out_ns <= now + D, "frame {n} more than D ahead");
            assert!(d.out_ns > prev, "frame {n} not monotonic");
            if n > 30 {
                assert_eq!(
                    d.out_ns - prev,
                    FRAME_NS,
                    "frame {n}: output not continuous"
                );
            }
            prev = d.out_ns;
        }
        // The jump minus the 5 ms the frame spent in the encoder, minus D.
        assert_eq!(g.correction_ns(), jump - 5 * MS - D);
    }

    #[test]
    fn backward_jump_reanchors_and_stays_monotonic() {
        // A framerate RISE rescales the counter backwards: the output is
        // re-anchored and the correction goes NEGATIVE.
        let mut g = TimestampGuard::default();
        for n in 0..30i64 {
            g.apply(n * FRAME_NS, n * FRAME_NS + 5 * MS, FRAME_NS);
        }
        let back = 10_000 * MS;
        let enc = 30 * FRAME_NS - back;
        let now = 30 * FRAME_NS + 5 * MS;
        let d = g.apply(enc, now, FRAME_NS);
        assert_eq!(d.kind, GuardKind::Reanchored);
        assert_eq!(
            d.out_ns,
            30 * FRAME_NS,
            "one frame after the previous output"
        );
        assert!(d.out_ns <= now);
        // The next frame continues on the encoder's cadence from the anchor.
        let d2 = g.apply(31 * FRAME_NS - back, 31 * FRAME_NS + 5 * MS, FRAME_NS);
        assert_eq!(d2.kind, GuardKind::Kept);
        assert_eq!(d2.out_ns, 31 * FRAME_NS);
        assert_eq!(g.correction_ns(), -back);
    }

    #[test]
    fn equal_clock_readings_still_increase() {
        let mut g = TimestampGuard::default();
        let a = g.apply(200 * MS, 50 * MS, FRAME_NS);
        let b = g.apply(300 * MS, 50 * MS, FRAME_NS);
        assert_eq!(a.out_ns, 50 * MS + D);
        assert_eq!(b.out_ns, 50 * MS + D + 1, "monotonic nudge");
    }

    #[test]
    fn reset_forgets_the_correction() {
        let mut g = TimestampGuard::default();
        g.apply(500 * MS, 100 * MS, FRAME_NS);
        assert_eq!(g.correction_ns(), 400 * MS - D);
        g.reset();
        assert_eq!(g.correction_ns(), 0);
        let d = g.apply(10 * MS, 20 * MS, FRAME_NS);
        assert_eq!((d.out_ns, d.kind), (10 * MS, GuardKind::Kept));
    }

    /// The encoder OUTPUT segment as GstVideoEncoder builds it for vah264enc
    /// 1.24: input start (5 s) + the 1000 h `time_adjustment`, base 0.
    fn vah264enc_output_segment() -> gst::FormattedSegment<gst::ClockTime> {
        let mut seg = gst::FormattedSegment::<gst::ClockTime>::new();
        seg.set_start(gst::ClockTime::from_seconds(3_600_000 + 5));
        seg
    }

    #[test]
    fn restamp_converts_through_the_output_segment_never_raw_pts() {
        gst::init().expect("gst init");
        let seg = vah264enc_output_segment();
        let mut state = GuardState {
            guard: TimestampGuard::default(),
            segment: Some(seg.clone()),
        };
        // A frame whose running time is 12 s, leaving when the clock reads 10 s
        // (2 s in the future). Its PTS carries the segment start.
        let pts = gst::ClockTime::from_seconds(3_600_000 + 5 + 12);
        let now = gst::ClockTime::from_seconds(10);
        let step = state.process(
            pts,
            Some(pts),
            Some(gst::ClockTime::from_mseconds(33)),
            Some(now),
        );
        let GuardStep::Decided {
            raw_lead_ns,
            correction_ns,
            kind,
            restamp: Some((new_pts, new_dts)),
        } = step
        else {
            panic!("expected a re-stamp, got {step:?}");
        };
        assert_eq!(kind, GuardKind::PulledBack);
        assert_eq!(raw_lead_ns, 2_000 * MS);
        assert_eq!(correction_ns, 2_000 * MS - D);
        // Running time 10 s + D mapped back through the SAME segment.
        let out_rt = now + gst::ClockTime::from_nseconds(D as u64);
        assert_eq!(
            new_pts,
            gst::ClockTime::from_seconds(3_600_000 + 5) + out_rt
        );
        assert_eq!(seg.to_running_time(Some(new_pts)), Some(out_rt));
        assert_eq!(new_dts, Some(new_pts), "pts − dts preserved (0)");
        // The #768 trap: a raw copy of a running-time/receive-time value would
        // land exactly the 1000 h time_adjustment away from the right PTS.
        let raw_copy = gst::ClockTime::from_seconds(5) + out_rt;
        assert_eq!(
            new_pts.nseconds() - raw_copy.nseconds(),
            gst::ClockTime::from_seconds(3_600_000).nseconds()
        );
    }

    #[test]
    fn unchanged_stamp_is_not_rewritten() {
        gst::init().expect("gst init");
        let mut state = GuardState {
            guard: TimestampGuard::default(),
            segment: Some(vah264enc_output_segment()),
        };
        let pts = gst::ClockTime::from_seconds(3_600_000 + 5 + 9);
        let step = state.process(pts, Some(pts), None, Some(gst::ClockTime::from_seconds(10)));
        assert!(
            matches!(
                step,
                GuardStep::Decided {
                    kind: GuardKind::Kept,
                    restamp: None,
                    ..
                }
            ),
            "a stamp behind the clock passes untouched: {step:?}"
        );
    }

    #[test]
    fn missing_segment_or_clock_is_unmeasured() {
        gst::init().expect("gst init");
        let pts = gst::ClockTime::from_seconds(1);
        let mut no_segment = GuardState::default();
        assert_eq!(
            no_segment.process(pts, None, None, Some(pts)),
            GuardStep::Unmeasured
        );
        let mut no_clock = GuardState {
            guard: TimestampGuard::default(),
            segment: Some(gst::FormattedSegment::<gst::ClockTime>::new()),
        };
        assert_eq!(
            no_clock.process(pts, None, None, None),
            GuardStep::Unmeasured
        );
        // A PTS before the segment start has no running time.
        let mut before_start = GuardState {
            guard: TimestampGuard::default(),
            segment: Some(vah264enc_output_segment()),
        };
        assert_eq!(
            before_start.process(pts, None, None, Some(pts)),
            GuardStep::Unmeasured
        );
    }

    #[test]
    fn a_failed_conversion_leaves_the_guard_state_unchanged() {
        gst::init().expect("gst init");
        // An output segment with a 10 s base: a running time below 10 s maps
        // to a position before the segment start, i.e. no position at all.
        let start = 3_600_000 + 5;
        let mut seg = vah264enc_output_segment();
        seg.set_base(gst::ClockTime::from_seconds(10));
        let mut state = GuardState {
            guard: TimestampGuard::default(),
            segment: Some(seg),
        };
        // Running time 14 s while the clock reads 5 s: the guard would pull
        // the frame back to 5 s + D, which cannot be converted back to a PTS.
        let pts = gst::ClockTime::from_seconds(start + 4);
        let step = state.process(pts, Some(pts), None, Some(gst::ClockTime::from_seconds(5)));
        assert_eq!(step, GuardStep::Unmeasured);
        // The buffer left untouched, so the guard must not remember having
        // re-stamped it: no correction, no last output.
        assert_eq!(state.guard.correction_ns(), 0, "correction committed");
        assert_eq!(state.guard.last_out_ns, None, "last output committed");
        // The next frame (running time 11 s, clock 12 s) is judged as if the
        // failed one never happened: kept, untouched.
        let next = gst::ClockTime::from_seconds(start + 1);
        let step = state.process(
            next,
            Some(next),
            None,
            Some(gst::ClockTime::from_seconds(12)),
        );
        assert_eq!(
            step,
            GuardStep::Decided {
                raw_lead_ns: -1_000 * MS,
                correction_ns: 0,
                kind: GuardKind::Kept,
                restamp: None,
            }
        );
    }

    #[test]
    fn guard_probe_glue_restamps_buffers_and_counts() {
        // The probe callbacks without pads: SEGMENT event → buffers → stats.
        gst::init().expect("gst init");
        let timing = EncoderTiming::new("vah264enc".into(), "test".into());
        let seg = vah264enc_output_segment();
        timing.on_guard_event(&gst::event::Segment::new(&seg));

        let mut buffer = gst::Buffer::new();
        {
            let b = buffer.get_mut().expect("fresh buffer is writable");
            let pts = gst::ClockTime::from_seconds(3_600_000 + 5 + 12);
            b.set_pts(pts);
            b.set_dts(pts);
            b.set_duration(gst::ClockTime::from_mseconds(33));
        }
        timing.on_guard_buffer(&mut buffer, Some(gst::ClockTime::from_seconds(11)));
        // 1 s ahead of the clock: pulled back to clock + D.
        assert_eq!(
            buffer.pts(),
            Some(
                gst::ClockTime::from_seconds(3_600_000 + 5 + 11)
                    + gst::ClockTime::from_nseconds(D as u64)
            )
        );
        assert_eq!(buffer.dts(), buffer.pts());

        let snap = timing.snapshot();
        assert!(!snap.installed, "no pads → not installed");
        assert_eq!(snap.frames_seen, 1);
        assert_eq!(snap.frames_pulled_back, 1);
        assert_eq!(snap.correction_ms, 1_000 - TIMESTAMP_DEADBAND_MS);
        assert_eq!(snap.raw_lead_max_ms, Some(1_000));
        assert_eq!(snap.raw_lead_last_ms, Some(1_000));

        // FLUSH_STOP resets the guard: the next on-time frame is untouched.
        timing.on_guard_event(&gst::event::FlushStop::new(true));
        let mut next = gst::Buffer::new();
        let on_time = gst::ClockTime::from_seconds(3_600_000 + 5 + 20);
        next.get_mut().expect("writable").set_pts(on_time);
        timing.on_guard_buffer(&mut next, Some(gst::ClockTime::from_seconds(21)));
        assert_eq!(next.pts(), Some(on_time));

        // A buffer without a PTS passes untouched and is not counted.
        let mut headers = gst::Buffer::new();
        timing.on_guard_buffer(&mut headers, Some(gst::ClockTime::from_seconds(22)));
        assert_eq!(headers.pts(), None);
        assert_eq!(timing.snapshot().frames_seen, 2);
    }

    #[test]
    fn lead_meter_records_the_window() {
        gst::init().expect("gst init");
        let timing = EncoderTiming::new("x264enc".into(), "test".into());
        assert_eq!(timing.lead_window_ms(Duration::from_secs(30)), None);
        timing.on_lead_event(&gst::event::Segment::new(&gst::FormattedSegment::<
            gst::ClockTime,
        >::new()));
        // 40 ms behind the clock, then 250 ms ahead of it.
        timing.on_lead_buffer(
            Some(gst::ClockTime::from_mseconds(960)),
            Some(gst::ClockTime::from_seconds(1)),
        );
        timing.on_lead_buffer(
            Some(gst::ClockTime::from_mseconds(2_250)),
            Some(gst::ClockTime::from_seconds(2)),
        );
        assert_eq!(
            timing.lead_window_ms(Duration::from_secs(30)),
            Some((-40, 250))
        );
        let snap = timing.snapshot();
        assert_eq!(snap.lead_ms_30s, Some(LeadWindow { min: -40, max: 250 }));
        assert_eq!(snap.raw_lead_max_ms, None, "the guard measured nothing");
        let json = serde_json::to_value(&snap).expect("serialize");
        assert!(json.get("leadMs30s").is_some(), "camelCase: {json}");
        assert!(json.get("framesPulledBack").is_some(), "camelCase: {json}");
    }
}
