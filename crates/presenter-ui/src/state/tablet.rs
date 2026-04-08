use leptos::prelude::*;
use presenter_core::{BibleBroadcast, TimersOverview};
use std::collections::HashMap;
use wasm_bindgen::{JsCast, UnwrapThrowExt};

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
    pub toast_timer_handle: RwSignal<Option<i32>>,
    pub active_slide_id: RwSignal<Option<String>>,
    pub ws_connected: RwSignal<bool>,
    pub timers: RwSignal<Option<TimersOverview>>,
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
            active_slide_id: RwSignal::new(None),
            sidebar_open: RwSignal::new(true),
            text_scale: RwSignal::new(saved_scale),
            toast_message: RwSignal::new(None),
            toast_variant: RwSignal::new("info".to_string()),
            toast_timer_handle: RwSignal::new(None),
            ws_connected: RwSignal::new(false),
            timers: RwSignal::new(None),
        }
    }

    pub fn show_toast(&self, msg: &str, variant: &str) {
        // Cancel previous timer
        if let Some(handle) = self.toast_timer_handle.get_untracked() {
            let window = web_sys::window().expect_throw("no window");
            window.clear_timeout_with_handle(handle);
        }

        self.toast_variant.set(variant.to_string());
        self.toast_message.set(Some(msg.to_string()));

        let toast = self.toast_message;
        let timer_handle_sig = self.toast_timer_handle;
        let closure = wasm_bindgen::closure::Closure::once_into_js(move || {
            toast.set(None);
            timer_handle_sig.set(None);
        });
        let window = web_sys::window().expect_throw("no window");
        if let Ok(handle) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            2_500,
        ) {
            self.toast_timer_handle.set(Some(handle));
        }
    }

    pub fn persist_scale(&self) {
        let scale = self.text_scale.get_untracked();
        session::set_persistent(SCALE_KEY, &scale.to_string());
    }
}
