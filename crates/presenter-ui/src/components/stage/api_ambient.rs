use leptos::prelude::*;

use crate::components::stage::api_text::{slide_api_lines, ApiLines};
use crate::components::stage::ndi_video::NdiVideo;
use crate::components::stage::status_bar::StatusBar;
use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_effect;
use crate::ws::stage::StageWsState;

/// Largest font the ambient overlay's lines may auto-fit to (px). The boxes
/// are viewport-relative (see `stage_ambient.css`), so autofit shrinks from
/// here to whatever fits.
const PRIMARY_MAX_FONT: f64 = 160.0;
const SECONDARY_MAX_FONT: f64 = 90.0;

/// Ambient API layout (`api-ambient`, #799): the stage display as an
/// atmosphere surface. The active NDI source (CG song videos) always fills
/// the screen; the current API lyric line(s) — per the operator's text mode —
/// fade in as a clean lower-third overlay only while there is text, and fade
/// out to pure video when the text is cleared. The bottom status bar (clock +
/// connection readout) stays exactly as on `ndi-fullscreen` (owner ruling on
/// #799) and the overlay sits above it; no header chrome and no NDI status
/// overlays — with no text the screen is the video (or black while no source
/// is live) plus the status bar.
#[component]
pub fn ApiAmbient(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_active_source_id = ctx.ndi_active_source_id;
    let snapshot = ctx.snapshot;

    // De-duplicate via Memo: see ndi_fullscreen.rs for the full rationale —
    // without it every snapshot/WS replay remounts <NdiVideo> and leaks an
    // NVENC encoder session per write.
    let active_source = Memo::new(move |_| ndi_active_source_id.get());

    // The line(s) the current text mode selects for the CURRENT slide only
    // (an ambient display never previews the next line). Memo so a snapshot
    // re-publish with the same text (timers, a same-mode republish) neither
    // re-fits the font nor restarts the fade.
    let lines = Memo::new(move |_| {
        snapshot.with(|snap| {
            snap.as_ref()
                .map(|s| slide_api_lines(s.current.as_ref(), s.text_mode))
                .unwrap_or_default()
        })
    });
    let visible = Memo::new(move |_| !lines.get().is_empty());

    // The overlay keeps showing the LAST non-empty lines while it fades out,
    // so a clear fades the text away instead of blanking it instantly.
    let shown = RwSignal::new(ApiLines::default());
    Effect::new(move |_| {
        let next = lines.get();
        if !next.is_empty() {
            shown.set(next);
        }
    });

    let primary_text = move || shown.with(|l| l.primary.clone());
    let secondary_text = move || shown.with(|l| l.secondary.clone().unwrap_or_default());
    let has_secondary = move || shown.with(|l| l.secondary.is_some());

    let primary_ref = NodeRef::<leptos::html::Div>::new();
    let secondary_ref = NodeRef::<leptos::html::Div>::new();
    // The primary box grows/shrinks when the secondary line hides/shows (a
    // both <-> original switch keeps the same primary text), so re-fit on
    // either change.
    let primary_fit_trigger = move || shown.with(|l| (l.primary.clone(), l.secondary.is_some()));
    autofit_effect(primary_ref, PRIMARY_MAX_FONT, primary_fit_trigger);
    autofit_effect(secondary_ref, SECONDARY_MAX_FONT, secondary_text);

    view! {
        <div class="stage-api-ambient" data-layout="api-ambient">
            <Show when=move || ndi_active.get()>
                {move || {
                    active_source.get().map(|source_id| view! {
                        <NdiVideo
                            source_id=source_id
                            class="stage-api-ambient__video"
                        />
                    })
                }}
            </Show>

            <div
                class="stage-api-ambient__lyrics"
                data-role="ambient-lyrics"
                data-visible=move || if visible.get() { "true" } else { "false" }
                data-both=move || if has_secondary() { "true" } else { "false" }
            >
                <div
                    node_ref=primary_ref
                    class="stage-api-ambient__primary"
                    data-role="ambient-primary"
                >
                    {primary_text}
                </div>
                <div
                    node_ref=secondary_ref
                    class="stage-api-ambient__secondary"
                    data-role="ambient-secondary"
                >
                    {secondary_text}
                </div>
            </div>

            // Same status bar + flags as ndi-fullscreen (owner ruling, #799).
            <StatusBar ws_state=ws_state latency_ms=latency_ms hide_live=true hide_song_number=true />
        </div>
    }
}
