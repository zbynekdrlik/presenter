use leptos::prelude::*;
use presenter_core::LiveEvent;
use wasm_bindgen::prelude::*;

use crate::api;
use crate::components::stage::{
    bible_layout::BibleLayout, ndi_fullscreen::NdiFullscreen, preach_layout::PreachLayout,
    timer_layout::TimerLayout, worship_pp::WorshipPp, worship_snv::WorshipSnv,
};
use crate::state::stage::StageContext;
use crate::ws::stage::{self, StageWsState};

#[component]
pub fn StagePage() -> impl IntoView {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "stage");
    }

    let ctx = StageContext::new("worship-snv".to_string());
    provide_context(ctx.clone());

    // Expose test globals
    set_global_string("__presenterStageClientId", &ctx.client_id);
    set_global_string("__presenterStageLayout", &ctx.layout_code.get_untracked());

    // Connect stage WebSocket
    let ws_handle = stage::use_stage_websocket(ctx.client_id.clone(), ctx.layout_code);

    // Expose connection state for E2E tests
    {
        let ws_state = ws_handle.state;
        Effect::new(move |_| {
            let state_str = match ws_state.get() {
                StageWsState::Connecting => "connecting",
                StageWsState::Connected => "connected",
                StageWsState::Reconnecting => "reconnecting",
                StageWsState::Disconnected => "disconnected",
            };
            set_global_string("__presenterStageConnectionState", state_str);
        });
    }

    // Handle WebSocket events
    {
        let ctx = ctx.clone();
        let last_event = ws_handle.last_event;
        Effect::new(move |_| {
            let Some(event) = last_event.get() else {
                return;
            };
            match event {
                LiveEvent::Stage { snapshot } => {
                    ctx.snapshot.set(Some(snapshot));
                }
                LiveEvent::StageLayout { code } => {
                    ctx.layout_code.set(code.clone());
                    set_global_string("__presenterStageLayout", &code);
                }
                LiveEvent::BibleSlide { output } => {
                    ctx.bible_overlay.set(Some(output));
                }
                LiveEvent::BibleCleared => {
                    ctx.bible_overlay.set(None);
                }
                LiveEvent::BroadcastLive { enabled } => {
                    ctx.broadcast_live.set(enabled);
                }
                LiveEvent::Timers { overview } => {
                    ctx.snapshot.update(|snap| {
                        if let Some(s) = snap {
                            s.timers = overview;
                        }
                    });
                }
                LiveEvent::NdiSourceActivated { .. } => {
                    ctx.ndi_active.set(true);
                    ctx.ndi_status.set("connecting".to_string());
                }
                LiveEvent::NdiSourceDeactivated => {
                    ctx.ndi_active.set(false);
                    ctx.ndi_status.set(String::new());
                }
                LiveEvent::NdiConnectionStatus { status } => {
                    ctx.ndi_status.set(status);
                }
                _ => {}
            }
        });
    }

    // Fetch initial data
    {
        let ctx = ctx.clone();
        leptos::task::spawn_local(async move {
            if let Ok(layout_resp) = api::stage::get_layout().await {
                ctx.layout_code.set(layout_resp.code.clone());
                set_global_string("__presenterStageLayout", &layout_resp.code);
            }
            if let Ok(snapshot) = api::stage::get_snapshot().await {
                ctx.snapshot.set(Some(snapshot));
            }
            if let Ok(broadcast) = api::stage::get_broadcast_live().await {
                ctx.broadcast_live.set(broadcast.enabled);
            }
            if let Ok(Some(output)) = api::bible::get_active_slide_output().await {
                ctx.bible_overlay.set(Some(output));
            }
            // Check if an NDI source is already active
            if let Ok(sources) = api::ndi::list_video_sources().await {
                if sources.iter().any(|s| s.is_active) {
                    ctx.ndi_active.set(true);
                }
            }
        });
    }

    // Sync body attributes for E2E test compatibility
    {
        let bible_overlay = ctx.bible_overlay;
        Effect::new(move |_| {
            if let Some(body) = crate::utils::window::document_body() {
                let active = if bible_overlay.get().is_some() {
                    "true"
                } else {
                    "false"
                };
                let _ = body.set_attribute("data-bible-active", active);
            }
        });
    }
    {
        let layout_code = ctx.layout_code;
        Effect::new(move |_| {
            if let Some(body) = crate::utils::window::document_body() {
                let _ = body.set_attribute("data-layout-code", &layout_code.get());
            }
        });
    }

    let ws_state = ws_handle.state;
    let latency_ms = ws_handle.latency_ms;
    let layout_code = ctx.layout_code;

    view! {
        {move || {
            let code = layout_code.get();
            match code.as_str() {
                "worship-pp" => {
                    view! { <WorshipPp ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                "timer" => {
                    view! { <TimerLayout ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                "preach" => {
                    view! { <PreachLayout ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                "ndi-fullscreen" => {
                    view! { <NdiFullscreen ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                "bible" => {
                    view! { <BibleLayout ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                _ => {
                    view! { <WorshipSnv ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
            }
        }}
    }
}

fn set_global_string(name: &str, value: &str) {
    let _ = js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str(name),
        &JsValue::from_str(value),
    );
}
