use leptos::prelude::*;
use presenter_core::{BibleSlideOutput, StageDisplaySnapshot};
use uuid::Uuid;

use super::session;

const CLIENT_ID_KEY: &str = "stageClientId";

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
