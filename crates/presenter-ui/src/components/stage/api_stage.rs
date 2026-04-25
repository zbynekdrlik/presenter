use leptos::prelude::*;

use crate::components::stage::worship_snv::WorshipSnv;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

/// Stage layout for API-driven slides with an optional live NDI video background.
///
/// Wraps `WorshipSnv` and adds a sibling `<img src="/ndi/mjpeg">` layer that
/// renders only when a video source is active (driven by
/// `StageContext::ndi_active`). Also surfaces the NDI connection status
/// overlay for the "connecting" / "disconnected" states.
#[component]
pub fn ApiStage(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_status = ctx.ndi_status;

    view! {
        <div class="stage-api">
            <Show when=move || ndi_active.get()>
                <img src="/ndi/mjpeg" class="stage-api__ndi" />
            </Show>

            <Show when=move || {
                let status = ndi_status.get();
                status == "disconnected" || status == "connecting"
            }>
                <div class="stage-api__overlay">
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

            <WorshipSnv ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}
