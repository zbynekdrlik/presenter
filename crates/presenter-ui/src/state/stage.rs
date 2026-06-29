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
