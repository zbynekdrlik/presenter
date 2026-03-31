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
}

impl StageContext {
    pub fn new(initial_layout: String) -> Self {
        Self {
            client_id: load_or_create_client_id(),
            layout_code: RwSignal::new(initial_layout),
            snapshot: RwSignal::new(None),
            broadcast_live: RwSignal::new(false),
            bible_overlay: RwSignal::new(None),
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
