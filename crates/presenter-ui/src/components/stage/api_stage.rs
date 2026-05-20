use leptos::prelude::*;

use crate::components::stage::ndi_status_text;
use crate::components::stage::ndi_video::NdiVideo;
use crate::components::stage::worship_snv::WorshipSnv;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

/// Stage layout for API-driven slides with an optional live NDI video background.
///
/// Wraps `WorshipSnv` and adds a sibling `<NdiVideo>` layer that renders only
/// when a video source is active (driven by `StageContext::ndi_active` plus
/// `ndi_active_source_id`). Also surfaces the NDI connection status overlay
/// for the "connecting" / "disconnected" states.
#[component]
pub fn ApiStage(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_active_source_id = ctx.ndi_active_source_id;
    let ndi_status = ctx.ndi_status;

    // De-duplicate via Memo: see ndi_fullscreen.rs for the full rationale.
    // Without this, WS replays + initial fetch each remount <NdiVideo>,
    // leaking NVENC encoder sessions per page load.
    let active_source = Memo::new(move |_| ndi_active_source_id.get());

    view! {
        <div class="stage-api">
            <Show when=move || ndi_active.get()>
                {move || {
                    active_source.get().map(|source_id| view! {
                        <NdiVideo
                            source_id=source_id
                            class="stage-api__ndi"
                        />
                    })
                }}
            </Show>

            <Show when=move || {
                let status = ndi_status.get();
                status == "disconnected"
                    || status == "connecting"
                    || status.starts_with("failed")
            }>
                <div class="stage-api__overlay">
                    {move || ndi_status_text(&ndi_status.get())}
                </div>
            </Show>

            <WorshipSnv ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}
