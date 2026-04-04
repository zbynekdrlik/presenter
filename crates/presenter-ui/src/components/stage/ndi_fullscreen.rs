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
    let img_ref = NodeRef::<leptos::html::Img>::new();

    // When ndi_active becomes true, connect to MJPEG WebSocket stream
    {
        let img_ref = img_ref.clone();
        Effect::new(move |_| {
            let active = ndi_active.get();
            if active {
                let img_ref = img_ref.clone();
                leptos::task::spawn_local(async move {
                    connect_mjpeg_ws(img_ref);
                });
            }
        });
    }

    view! {
        <div class="stage-ndi">
            <img
                node_ref=img_ref
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

/// Connect to the MJPEG WebSocket stream and render frames to an <img> element.
fn connect_mjpeg_ws(img_ref: NodeRef<leptos::html::Img>) {
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

    // Track previous blob URL for cleanup
    let prev_url = std::rc::Rc::new(std::cell::RefCell::new(String::new()));

    let img_ref_clone = img_ref.clone();
    let prev_url_clone = prev_url.clone();
    let onmessage =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
            let data = ev.data();
            // Binary message = JPEG frame
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
                    if let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) {
                        if let Some(img_el) = img_ref_clone.get() {
                            let html_img: &web_sys::HtmlImageElement = img_el.as_ref();
                            html_img.set_src(&url);
                        }
                        // Revoke previous blob URL to prevent memory leak
                        let mut prev = prev_url_clone.borrow_mut();
                        if !prev.is_empty() {
                            let _ = web_sys::Url::revoke_object_url(&prev);
                        }
                        *prev = url;
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

    let img_ref_reconnect = img_ref.clone();
    let onclose = Closure::<dyn FnMut()>::new(move || {
        web_sys::console::log_1(&"NDI: MJPEG WebSocket closed, reconnecting in 2s...".into());
        let img_ref = img_ref_reconnect.clone();
        let _ = gloo_timers::callback::Timeout::new(2000, move || {
            connect_mjpeg_ws(img_ref);
        });
    });
    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();
}
