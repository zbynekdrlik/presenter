use leptos::prelude::*;
use presenter_core::LiveEvent;
use wasm_bindgen::prelude::*;

use crate::api;
use crate::components::stage::{
    api_stage::ApiStage, bible_layout::BibleLayout, ndi_fullscreen::NdiFullscreen,
    preach_layout::PreachLayout, timer_layout::TimerLayout, worship_pp::WorshipPp,
    worship_snv::WorshipSnv,
};
use crate::state::stage::StageContext;
use crate::ws::stage::{self, StageWsState};

#[component]
pub fn StagePage() -> impl IntoView {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "stage");
    }

    // #460: a preview mirror (`/stage?preview=1`, embedded in the operator
    // header) is a small passive snapshot of the stage, NOT a real display —
    // it must not grab a screen wake lock (that's for the operator's own
    // screen, and the redundant request just logs a permissions warning in
    // the iframe). Detect preview mode once and skip the wake lock for it.
    let preview = crate::utils::window::url_flag_enabled("preview");

    // Keep the TV screen awake for the whole service (issue #402). Acquires a
    // screen wake lock now and re-acquires it on every visibilitychange back
    // to visible (the browser auto-releases the lock when the page hides).
    if !preview {
        crate::components::stage::wake_lock::start_wake_lock_guard();
    }

    let ctx = StageContext::new("worship-snv".to_string());
    provide_context(ctx.clone());

    // Expose test globals
    set_global_string("__presenterStageClientId", &ctx.client_id);
    set_global_string("__presenterStageLayout", &ctx.layout_code.get_untracked());

    // Test hook (#479): drive the stage-side video-latency readout
    // deterministically from the E2E without a live NDI pipeline (the
    // GitHub-hosted e2e lane has no NDI source/GPU). Accepts a number (ms) to
    // show "video · N ms", or null/undefined to clear it. In production this
    // global is simply never called — the real value is written per-frame by
    // `NdiVideo`'s rVFC observer.
    {
        let video_latency = ctx.video_latency_ms;
        let setter = Closure::wrap(Box::new(move |v: JsValue| match v.as_f64() {
            Some(ms) => video_latency.set(Some(ms)),
            None => video_latency.set(None),
        }) as Box<dyn Fn(JsValue)>);
        let _ = js_sys::Reflect::set(
            &js_sys::global(),
            &JsValue::from_str("__presenterStageSetVideoLatency"),
            setter.as_ref(),
        );
        setter.forget();
    }

    // Test hook (#500): drive the "frames are presenting" flag deterministically
    // from the E2E without a live NDI/GPU pipeline (the GitHub-hosted e2e lane
    // has neither). Accepts a boolean — the SAME signal the rVFC observer /
    // proxy write per frame — so a spec can simulate frames flowing while the
    // status is still a stale `connecting` and assert the neutral cover drops.
    // In production this global is simply never called.
    {
        let frames_live = ctx.ndi_frames_live;
        let setter = Closure::wrap(Box::new(move |v: JsValue| {
            frames_live.set(v.as_bool().unwrap_or(false));
        }) as Box<dyn Fn(JsValue)>);
        let _ = js_sys::Reflect::set(
            &js_sys::global(),
            &JsValue::from_str("__presenterStageSetNdiFramesLive"),
            setter.as_ref(),
        );
        setter.forget();
    }

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
                // Only accept Stage snapshots matching our layout to keep
                // API stage and normal stage independent.
                LiveEvent::Stage { snapshot }
                    if snapshot.layout.code == ctx.layout_code.get_untracked() =>
                {
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
                LiveEvent::NdiSourceActivated { source_id, .. } => {
                    ctx.ndi_active.set(true);
                    ctx.ndi_active_source_id.set(Some(source_id));
                    ctx.ndi_status.set("connecting".to_string());
                    // #500: a freshly-activated source has no frames yet — the
                    // neutral cover must show until the WHEP video decodes.
                    ctx.ndi_frames_live.set(false);
                }
                LiveEvent::NdiSourceDeactivated => {
                    ctx.ndi_active.set(false);
                    ctx.ndi_active_source_id.set(None);
                    ctx.ndi_status.set(String::new());
                    // #500: no source → no frames; clear the live-frames flag.
                    ctx.ndi_frames_live.set(false);
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
        });
    }

    // Sync NDI source state on page load AND on every WS (re)connect. The
    // live hub does not replay events, so an `ndi_source_activated`
    // published while this client's socket was down or zombie is LOST —
    // without the reconnect resync the stage stays white (no <NdiVideo>,
    // zero WHEP attempts) until a manual page reload (prod TV incident).
    {
        let ctx = ctx.clone();
        let ws_state = ws_handle.state;
        // Memo dedups the per-heartbeat Connected re-sets so the fetch only
        // runs on actual state TRANSITIONS (first connect + reconnects).
        let connected = Memo::new(move |_| ws_state.get() == StageWsState::Connected);
        sync_ndi_source_state(ctx.clone());
        Effect::new(move |_| {
            if connected.get() {
                sync_ndi_source_state(ctx.clone());
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
                "api" => {
                    view! { <ApiStage ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                _ => {
                    view! { <WorshipSnv ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
            }
        }}
    }
}

/// Fetch the currently-active video source and sync the NDI signals to it.
///
/// Safe to call repeatedly: `NdiFullscreen`'s `Memo` + `Show` dedup
/// identical values, so a no-change resync never remounts `<NdiVideo>`
/// (no reconnect churn). Only an actual server-side change (deactivate,
/// reactivate, different source) propagates.
fn sync_ndi_source_state(ctx: StageContext) {
    leptos::task::spawn_local(async move {
        let Ok(sources) = api::ndi::list_video_sources().await else {
            return;
        };
        match sources.iter().find(|s| s.is_active) {
            Some(active) => {
                let id = Some(active.id.clone());
                if ctx.ndi_active_source_id.get_untracked() != id {
                    // New/changed source (incl. every FRESH page load / stage
                    // relaunch, where the prior id is None): start in the
                    // neutral "connecting" state, NOT "" — the empty status maps
                    // to NdiOverlayKind::None, which renders NEITHER the covering
                    // placeholder NOR the overlay, leaving the bare <video>'s
                    // native play-arrow exposed until the server's next ~30s
                    // status tick (#448 regression on the #447-frequent relaunch
                    // path). "connecting" is a neutral covering state, so the
                    // gray placeholder hides the bare video immediately; the
                    // real status (no-signal / connected / failed) replaces it
                    // when the pipeline resolves. It also clears any stale
                    // "disconnected"/"failed" overlay from before the gap.
                    ctx.ndi_status.set("connecting".to_string());
                    // #500: a new/changed source (incl. every fresh page load /
                    // relaunch) has no frames yet — show the neutral cover until
                    // the WHEP video decodes, never carry a stale `true`.
                    ctx.ndi_frames_live.set(false);
                }
                ctx.ndi_active.set(true);
                ctx.ndi_active_source_id.set(id);
            }
            None => {
                ctx.ndi_active.set(false);
                ctx.ndi_active_source_id.set(None);
                ctx.ndi_status.set(String::new());
                // #500: no active source → no frames; clear the live-frames flag.
                ctx.ndi_frames_live.set(false);
            }
        }
    });
}

fn set_global_string(name: &str, value: &str) {
    let _ = js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str(name),
        &JsValue::from_str(value),
    );
}
