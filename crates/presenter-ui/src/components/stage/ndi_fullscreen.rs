use leptos::prelude::*;

use crate::components::stage::ndi_video::NdiVideo;
use crate::components::stage::status_bar::StatusBar;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

/// NDI fullscreen stage layout.
///
/// Mounts an `<NdiVideo>` Leptos component that connects to the per-source
/// WHEP endpoint and streams HW-decoded H264 via WebRTC into a `<video>`
/// element. Composition is browser-native; presenter only proxies signalling.
#[component]
pub fn NdiFullscreen(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_active_source_id = ctx.ndi_active_source_id;
    let ndi_status = ctx.ndi_status;

    view! {
        <div class="stage-ndi">
            <Show when=move || ndi_active.get()>
                {move || {
                    ndi_active_source_id.get().map(|source_id| view! {
                        <NdiVideo
                            source_id=source_id
                            class="stage-ndi__video"
                        />
                    })
                }}
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
