use leptos::prelude::*;
use wasm_bindgen::prelude::*;

use crate::components::stage::status_bar::StatusBar;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

#[component]
pub fn NdiFullscreen(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_status = ctx.ndi_status;
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

    // When ndi_active becomes true, connect to MJPEG WebSocket stream
    {
        let canvas_ref = canvas_ref.clone();
        Effect::new(move |_| {
            let active = ndi_active.get();
            if active {
                let canvas_ref = canvas_ref.clone();
                leptos::task::spawn_local(async move {
                    connect_mjpeg_ws(canvas_ref);
                });
            }
        });
    }

    view! {
        <div class="stage-ndi">
            <canvas
                node_ref=canvas_ref
                class="stage-ndi__video"
            />

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

            <StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}

/// Connect to the MJPEG WebSocket stream and render frames to a `<canvas>`.
///
/// Uses `createImageBitmap()` for off-main-thread JPEG decoding, then
/// draws to canvas with `drawImage()`. This avoids the Blob URL overhead
/// of the previous `<img>`-based approach.
fn connect_mjpeg_ws(canvas_ref: NodeRef<leptos::html::Canvas>) {
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
        Err(e) => {
            web_sys::console::error_1(&format!("NDI WS connect failed: {e:?}").into());
            return;
        }
    };
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let canvas_ref_msg = canvas_ref.clone();
    let onmessage =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
            let data = ev.data();
            if let Ok(buf) = data.dyn_into::<js_sys::ArrayBuffer>() {
                let array = js_sys::Uint8Array::new(&buf);
                let blob_parts = js_sys::Array::new();
                blob_parts.push(&array);

                let mut options = web_sys::BlobPropertyBag::new();
                options.type_("image/jpeg");

                if let Ok(blob) = web_sys::Blob::new_with_buffer_source_sequence_and_options(
                    &blob_parts,
                    &options,
                ) {
                    let canvas_ref = canvas_ref_msg.clone();
                    // createImageBitmap decodes JPEG off the main thread
                    if let Some(window) = web_sys::window() {
                        if let Ok(promise) = window.create_image_bitmap_with_blob(&blob) {
                            let future = wasm_bindgen_futures::JsFuture::from(promise);
                            wasm_bindgen_futures::spawn_local(async move {
                                if let Ok(bitmap_js) = future.await {
                                    let bitmap: web_sys::ImageBitmap =
                                        bitmap_js.unchecked_into();
                                    if let Some(canvas_el) = canvas_ref.get() {
                                        let html_canvas: &web_sys::HtmlCanvasElement =
                                            canvas_el.as_ref();
                                        let bw = bitmap.width();
                                        let bh = bitmap.height();

                                        // Match canvas internal resolution to source
                                        if html_canvas.width() != bw
                                            || html_canvas.height() != bh
                                        {
                                            html_canvas.set_width(bw);
                                            html_canvas.set_height(bh);
                                        }

                                        if let Ok(Some(ctx)) = html_canvas.get_context("2d") {
                                            let ctx: web_sys::CanvasRenderingContext2d =
                                                ctx.unchecked_into();
                                            let _ = ctx
                                                .draw_image_with_image_bitmap(&bitmap, 0.0, 0.0);
                                        }
                                        bitmap.close();
                                    }
                                }
                            });
                        }
                    }
                }
            }
        });
    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    let onopen = Closure::<dyn FnMut()>::new(move || {
        web_sys::console::log_1(&"NDI: MJPEG WebSocket connected".into());
    });
    ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    let onerror = Closure::<dyn FnMut()>::new(move || {
        web_sys::console::error_1(&"NDI: MJPEG WebSocket error".into());
    });
    ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    onerror.forget();

    let canvas_ref_reconnect = canvas_ref.clone();
    let onclose = Closure::<dyn FnMut()>::new(move || {
        web_sys::console::log_1(&"NDI: MJPEG WebSocket closed, reconnecting in 2s...".into());
        let canvas_ref = canvas_ref_reconnect.clone();
        let _ = gloo_timers::callback::Timeout::new(2000, move || {
            connect_mjpeg_ws(canvas_ref);
        });
    });
    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();
}
