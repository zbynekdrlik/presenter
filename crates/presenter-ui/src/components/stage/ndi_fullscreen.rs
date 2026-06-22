use leptos::prelude::*;

use crate::components::stage::ndi_video::NdiVideo;
use crate::components::stage::status_bar::StatusBar;
use crate::components::stage::{ndi_overlay_kind, ndi_status_text, NdiOverlayKind};
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

    // De-duplicate signal writes: WS replays and initial-fetch both set the
    // same source_id during page load. Without Memo, every set() re-runs the
    // closure below and re-mounts <NdiVideo>, spawning a fresh RtcPeerConnection
    // + NVENC encoder session per write. ~7 writes per page load exhausted the
    // NVENC consumer-card session limit and surfaced as `CUDA_ERROR_NO_DEVICE`.
    let active_source = Memo::new(move |_| ndi_active_source_id.get());

    view! {
        <div class="stage-ndi">
            <Show when=move || ndi_active.get()>
                {move || {
                    active_source.get().map(|source_id| view! {
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

            // #448: a configured source that is OFF/silent (`no-signal`) or
            // still starting (`connecting`) is an EXPECTED state — render a calm
            // GRAY placeholder. The `--cover` modifier paints a solid black
            // background so the bare `<video>` (no srcObject yet) and its native
            // browser play-arrow are hidden behind it. Only a GENUINE failure
            // (`failed[: reason]` / `disconnected`) shows the alarming red
            // `.stage-ndi__overlay`.
            <Show when=move || {
                ndi_active.get() && ndi_overlay_kind(&ndi_status.get()) == NdiOverlayKind::Neutral
            }>
                <div class="stage-ndi__placeholder stage-ndi__placeholder--cover">
                    {move || ndi_status_text(&ndi_status.get())}
                </div>
            </Show>

            <Show when=move || ndi_overlay_kind(&ndi_status.get()) == NdiOverlayKind::Error>
                <div class="stage-ndi__overlay">
                    {move || ndi_status_text(&ndi_status.get())}
                </div>
            </Show>

            <StatusBar ws_state=ws_state latency_ms=latency_ms hide_live=true hide_song_number=true />
        </div>
    }
}
