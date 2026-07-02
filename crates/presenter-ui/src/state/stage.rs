use leptos::prelude::*;
use presenter_core::{BibleSlideOutput, StageDisplaySnapshot};
use uuid::Uuid;

use super::session;

const CLIENT_ID_KEY: &str = "stageClientId";

/// Recent-window stage-TV health verdict (#532): "is this TV usable for the
/// band right now?" — computed CLIENT-SIDE from the render-side per-interval
/// accumulators in `FrameStats` (presented fps + presentation-gap stats),
/// which reset every beacon and are therefore inherently RECENT, unlike
/// getStats' cumulative freeze/drop counters (`dropped_frames` below answers
/// "how many since connect"; this answers "how is it doing RIGHT NOW", which
/// is what an operator glancing at the stage actually needs). See
/// `components::stage::ndi_frame_stats::stage_health` for the pure
/// classifier and its named threshold constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageHealth {
    /// 🟢 plynulé — smooth, fully usable.
    Good,
    /// 🟡 mierne seká — minor stutter, still usable.
    Degraded,
    /// 🔴 výpadky — freezing / not usable, needs attention.
    Bad,
}

impl StageHealth {
    /// The colour glyph shown on the stage readout.
    pub fn emoji(self) -> &'static str {
        match self {
            StageHealth::Good => "\u{1f7e2}",
            StageHealth::Degraded => "\u{1f7e1}",
            StageHealth::Bad => "\u{1f534}",
        }
    }

    /// The short Slovak label shown next to the glyph.
    pub fn label(self) -> &'static str {
        match self {
            StageHealth::Good => "plynul\u{e9}",
            StageHealth::Degraded => "mierne sek\u{e1}",
            StageHealth::Bad => "v\u{fd}padky",
        }
    }
}

/// One classified health reading (#532): the verdict plus the recent-window
/// presented-fps figure it was derived from, shown together as the small
/// secondary detail beside the colour (e.g. "🟢 plynulé · 28 fps").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageHealthReading {
    pub state: StageHealth,
    pub fps: f64,
}

#[derive(Clone)]
pub struct StageContext {
    pub client_id: String,
    pub layout_code: RwSignal<String>,
    pub snapshot: RwSignal<Option<StageDisplaySnapshot>>,
    pub broadcast_live: RwSignal<bool>,
    pub bible_overlay: RwSignal<Option<BibleSlideOutput>>,
    pub ndi_active: RwSignal<bool>,
    pub ndi_active_source_id: RwSignal<Option<String>>,
    pub ndi_status: RwSignal<String>,
    /// Stage-side VIDEO latency in ms (#479): the received→displayed decode+
    /// present lag of the NDI/WHEP video, derived per-frame from rVFC metadata
    /// by `NdiVideo`'s frame observer and shown in the stage's separate
    /// "video · N ms" readout. `None` when no video is flowing (no `NdiVideo`
    /// mounted, or no frames yet) — the readout is then hidden. Distinct from
    /// the WS connection round-trip shown in the "CONNECTED · N ms" readout.
    pub video_latency_ms: RwSignal<Option<f64>>,
    /// Whether NDI video frames are ACTUALLY presenting on screen right now
    /// (#500). Set `true` per presented frame by `NdiVideo`'s rVFC observer (or
    /// the currentTime proxy on rVFC-less browsers), and flipped back to `false`
    /// by the 1s health ticker once frames go stale (`FRAMES_LIVE_STALENESS_MS`),
    /// on `NdiVideo` cleanup, and when NDI goes inactive. Gates the neutral
    /// covering placeholder (`should_show_neutral_cover`) so a late-joining stage
    /// client whose `ndi_status` is still a stale `connecting` does not hide a
    /// video that is already decoding. `false` whenever no frames are flowing.
    pub ndi_frames_live: RwSignal<bool>,
    /// Browser↔server pipeline-clock offset estimate (#510, T3):
    /// `Some((offset_ms, rtt_ms))` once a fresh, low-RTT NTP-style round trip
    /// against `/ndi/time` has landed, `None` before the first sample or once
    /// the freshest one ages out (design's honest `n/a` trust predicate — see
    /// `ndi_clock_offset`). A later ticket (#512, T4) reads this to convert a
    /// `report.timestamp` reading into the server pipeline-clock domain.
    pub clock_offset: RwSignal<Option<(f64, f64)>>,
    /// Per-display dropped-frame + freeze counts (#523): `Some((frames_dropped,
    /// freeze_count))` from the SAME getStats inbound-rtp sample the health
    /// beacon already reads (`ndi_beacon::extract_inbound_video`), pushed to
    /// this signal each time a beacon is posted (~every
    /// `Watchdog::RVFC_BEACON_FRAME_PERIOD` frames or every 15th health tick —
    /// getStats is async, so this updates on the BEACON cadence, not the 1s
    /// video-latency cadence). **#532: this plumbing is kept (still populated,
    /// still test-hookable) but is NO LONGER shown on the stage readout** —
    /// the cumulative count never recovered from one old blip, which read as
    /// meaningless. `stage_health` below is what the readout displays now.
    pub dropped_frames: RwSignal<Option<(u32, u32)>>,
    /// Recent-window (~15-20s) stage-TV health verdict (#532): replaces the
    /// cumulative ⬇N/❄N suffix `dropped_frames` used to drive on the
    /// "server→displej · N ms" readout. `None` until the first beacon
    /// classifies a window (or right after a reconnect resets it); `Some` is
    /// refreshed on the SAME beacon cadence as `dropped_frames` above.
    pub stage_health: RwSignal<Option<StageHealthReading>>,
}

impl StageContext {
    pub fn new(initial_layout: String) -> Self {
        Self {
            client_id: load_or_create_client_id(),
            layout_code: RwSignal::new(initial_layout),
            snapshot: RwSignal::new(None),
            broadcast_live: RwSignal::new(false),
            bible_overlay: RwSignal::new(None),
            ndi_active: RwSignal::new(false),
            ndi_active_source_id: RwSignal::new(None),
            ndi_status: RwSignal::new(String::new()),
            video_latency_ms: RwSignal::new(None),
            ndi_frames_live: RwSignal::new(false),
            clock_offset: RwSignal::new(None),
            dropped_frames: RwSignal::new(None),
            stage_health: RwSignal::new(None),
        }
    }
}

fn load_or_create_client_id() -> String {
    if let Some(id) = session::get_persistent(CLIENT_ID_KEY) {
        if !id.is_empty() {
            return id;
        }
    }
    let id = Uuid::new_v4().to_string();
    session::set_persistent(CLIENT_ID_KEY, &id);
    id
}
