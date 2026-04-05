use leptos::prelude::*;

use crate::components::stage::status_bar::StatusBar;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

/// NDI fullscreen stage layout.
///
/// Uses native browser MJPEG rendering via `<img src="/ndi/mjpeg">`.
/// The server sends `multipart/x-mixed-replace` which the browser
/// decodes natively with zero JavaScript overhead.
#[component]
pub fn NdiFullscreen(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_status = ctx.ndi_status;

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

            <StatusBar ws_state=ws_state latency_ms=latency_ms hide_live=true />
        </div>
    }
}
