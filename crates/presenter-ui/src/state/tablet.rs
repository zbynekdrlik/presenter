use leptos::prelude::*;
use presenter_core::BibleBroadcast;
use std::collections::HashMap;

use crate::api::bible::{BiblePresentationSummary, BibleSlideDto};
use crate::state::session;

const DEFAULT_SCALE: u32 = 100;
const SCALE_KEY: &str = "tablet-scale";

#[derive(Clone)]
pub struct TabletContext {
    pub presentations: RwSignal<Vec<BiblePresentationSummary>>,
    pub current_presentation_id: RwSignal<Option<String>>,
    pub current_presentation_name: RwSignal<String>,
    pub slides: RwSignal<Vec<BibleSlideDto>>,
    pub slides_cache: RwSignal<HashMap<String, Vec<BibleSlideDto>>>,
    pub active_broadcast: RwSignal<Option<BibleBroadcast>>,
    pub sidebar_open: RwSignal<bool>,
    pub text_scale: RwSignal<u32>,
    pub toast_message: RwSignal<Option<String>>,
    pub toast_variant: RwSignal<String>,
    pub ws_connected: RwSignal<bool>,
}

impl TabletContext {
    pub fn new() -> Self {
        let saved_scale = session::get_persistent(SCALE_KEY)
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|&v| (50..=200).contains(&v))
            .unwrap_or(DEFAULT_SCALE);

        Self {
            presentations: RwSignal::new(Vec::new()),
            current_presentation_id: RwSignal::new(None),
            current_presentation_name: RwSignal::new("Select a presentation".to_string()),
            slides: RwSignal::new(Vec::new()),
            slides_cache: RwSignal::new(HashMap::new()),
            active_broadcast: RwSignal::new(None),
            sidebar_open: RwSignal::new(true),
            text_scale: RwSignal::new(saved_scale),
            toast_message: RwSignal::new(None),
            toast_variant: RwSignal::new("info".to_string()),
            ws_connected: RwSignal::new(false),
        }
    }

    pub fn show_toast(&self, msg: &str, variant: &str) {
        let toast = self.toast_message;
        let toast_variant = self.toast_variant;
        toast_variant.set(variant.to_string());
        toast.set(Some(msg.to_string()));
        gloo_timers::callback::Timeout::new(2_500, move || {
            toast.set(None);
        })
        .forget();
    }

    pub fn persist_scale(&self) {
        let scale = self.text_scale.get_untracked();
        session::set_persistent(SCALE_KEY, &scale.to_string());
    }
}
