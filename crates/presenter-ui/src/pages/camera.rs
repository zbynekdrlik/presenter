use leptos::prelude::*;
use presenter_core::LiveEvent;
use wasm_bindgen::prelude::*;

use crate::api;
use crate::components::stage::camera_crew::CameraCrew;
use crate::state::stage::StageContext;
use crate::ws::stage::{self, StageWsState};

const CAMERA_LAYOUT: &str = "camera-crew";

#[component]
pub fn CameraPage() -> impl IntoView {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "stage");
    }

    let ctx = StageContext::new(CAMERA_LAYOUT.to_string());
    provide_context(ctx.clone());

    set_global_string("__presenterStageClientId", &ctx.client_id);
    set_global_string("__presenterStageLayout", CAMERA_LAYOUT);

    // Connect stage WebSocket — same subscription as /stage clients use.
    // layout_code is pinned to "camera-crew" and never updated from events.
    let ws_handle = stage::use_stage_websocket(ctx.client_id.clone(), ctx.layout_code);

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

    // Handle WS events. CRITICAL: do NOT update layout_code from
    // LiveEvent::StageLayout — camera-crew is pinned.
    {
        let ctx = ctx.clone();
        let last_event = ws_handle.last_event;
        Effect::new(move |_| {
            let Some(event) = last_event.get() else {
                return;
            };
            match event {
                LiveEvent::Stage { snapshot } if snapshot.layout.code == CAMERA_LAYOUT => {
                    ctx.snapshot.set(Some(snapshot));
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
                _ => {}
            }
        });
    }

    // Initial data fetch — pinned to camera-crew.
    {
        let ctx = ctx.clone();
        leptos::task::spawn_local(async move {
            if let Ok(snapshot) = api::stage::get_snapshot_for(CAMERA_LAYOUT).await {
                ctx.snapshot.set(Some(snapshot));
            }
            if let Ok(broadcast) = api::stage::get_broadcast_live().await {
                ctx.broadcast_live.set(broadcast.enabled);
            }
        });
    }

    // Sync body attribute for E2E.
    Effect::new(move |_| {
        if let Some(body) = crate::utils::window::document_body() {
            let _ = body.set_attribute("data-layout-code", CAMERA_LAYOUT);
        }
    });

    view! { <CameraCrew /> }
}

fn set_global_string(name: &str, value: &str) {
    let _ = js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str(name),
        &JsValue::from_str(value),
    );
}
