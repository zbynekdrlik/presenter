use gloo_timers::callback::Interval;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;

use crate::components::stage::status_bar::StatusBar;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

/// NDI fullscreen stage layout.
///
/// Uses native browser MJPEG rendering via `<img src="/ndi/mjpeg">`.
/// The server sends `multipart/x-mixed-replace` which the browser
/// decodes natively with zero JavaScript overhead.
///
/// A parallel WebSocket connection counts frames for the FPS display.
#[component]
pub fn NdiFullscreen(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_status = ctx.ndi_status;

    // FPS counter: counts frames via a parallel WebSocket (lightweight)
    let (ndi_fps, set_ndi_fps) = signal(0u32);

    // Start FPS counter when NDI is active
    {
        Effect::new(move |_| {
            if ndi_active.get() {
                leptos::task::spawn_local(async move {
                    start_fps_counter(set_ndi_fps);
                });
            }
        });
    }

    // Build the MJPEG URL from the current page location
    let mjpeg_url = move || {
        if ndi_active.get() {
            "/ndi/mjpeg".to_string()
        } else {
            String::new()
        }
    };

    view! {
        <div class="stage-ndi">
            <Show when=move || ndi_active.get()>
                <img
                    src=mjpeg_url
                    class="stage-ndi__video"
                />
            </Show>

            <Show when=move || !ndi_active.get()>
                <div class="stage-ndi__placeholder">
                    "No video source configured"
                </div>
            </Show>

            <Show when=move || {
                let status = ndi_status.get();
                status == "disconnected" || status == "connecting"
            }>
                <div class="stage-ndi__overlay">
                    {move || {
                        let status = ndi_status.get();
                        if status == "disconnected" {
                            "Signal Lost — Reconnecting..."
                        } else if status == "connecting" {
                            "Connecting..."
                        } else {
                            ""
                        }
                    }}
                </div>
            </Show>

            // NDI layout: show connection box with FPS, but NO live pill
            <StatusBar ws_state=ws_state latency_ms=latency_ms hide_live=true ndi_fps=ndi_fps />
        </div>
    }
}

/// Start a lightweight WebSocket that counts frames for FPS display.
///
/// Does NOT render frames — only counts binary messages per second.
/// The actual rendering is done by the native MJPEG `<img>`.
fn start_fps_counter(set_fps: WriteSignal<u32>) {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return,
    };
    let location = window.location();
    let protocol = location.protocol().unwrap_or_default();
    let host = location.host().unwrap_or_default();
    let ws_protocol = if protocol == "https:" { "wss:" } else { "ws:" };
    let ws_url = format!("{ws_protocol}//{host}/ndi/stream");

    let ws = match web_sys::WebSocket::new(&ws_url) {
        Ok(ws) => ws,
        Err(_) => return,
    };
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

    // Count frames in a rolling 1-second window
    let frame_count = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let frame_count_msg = frame_count.clone();

    let onmessage =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |_ev: web_sys::MessageEvent| {
            frame_count_msg.set(frame_count_msg.get() + 1);
        });
    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    // Every second, read the count and reset
    let interval = Interval::new(1_000, move || {
        let count = frame_count.get();
        frame_count.set(0);
        set_fps.set(count);
    });
    interval.forget();
}
