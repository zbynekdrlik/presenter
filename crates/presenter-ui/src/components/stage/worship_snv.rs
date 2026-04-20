use leptos::prelude::*;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_effect;
use crate::utils::color::group_pill_style;
use crate::ws::stage::StageWsState;

const CURRENT_MAX_FONT: f64 = 800.0;
const NEXT_MAX_FONT: f64 = 500.0;
const CURRENT_GROUP_MAX_FONT: f64 = 200.0;
const NEXT_GROUP_MAX_FONT: f64 = 200.0;
const CURRENT_SONG_MAX_FONT: f64 = 200.0;
const NEXT_SONG_MAX_FONT: f64 = 200.0;

#[component]
pub fn WorshipSnv(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let current_text_ref = NodeRef::<leptos::html::Div>::new();
    let next_text_ref = NodeRef::<leptos::html::Div>::new();
    let current_group_ref = NodeRef::<leptos::html::Div>::new();
    let next_group_ref = NodeRef::<leptos::html::Div>::new();
    let current_song_ref = NodeRef::<leptos::html::Div>::new();
    let next_song_ref = NodeRef::<leptos::html::Div>::new();

    let current_text = move || {
        ctx.snapshot
            .get()
            .and_then(|s| {
                s.current.map(|slide| {
                    if !slide.stage.is_empty() {
                        slide.stage
                    } else {
                        slide.main
                    }
                })
            })
            .unwrap_or_default()
    };

    let next_text = move || {
        ctx.snapshot
            .get()
            .and_then(|s| {
                s.next.map(|slide| {
                    if !slide.stage.is_empty() {
                        slide.stage
                    } else {
                        slide.main
                    }
                })
            })
            .unwrap_or_default()
    };

    let current_group = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.current.and_then(|sl| sl.group))
    };
    let next_group = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.next.and_then(|sl| sl.group))
    };

    let current_group_style = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.current.and_then(|sl| sl.group_color))
            .map(|color| group_pill_style(&color))
            .unwrap_or_default()
    };

    let next_group_style = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.next.and_then(|sl| sl.group_color))
            .map(|color| group_pill_style(&color))
            .unwrap_or_default()
    };

    let current_group_text = move || current_group().unwrap_or_default();
    let next_group_text = move || next_group().unwrap_or_default();

    let current_song_text = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.song_name)
            .unwrap_or_default()
    };

    let next_song_text = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.next_song_name)
            .unwrap_or_default()
    };

    autofit_effect(current_text_ref, CURRENT_MAX_FONT, current_text.clone());
    autofit_effect(next_text_ref, NEXT_MAX_FONT, next_text.clone());
    autofit_effect(
        current_group_ref,
        CURRENT_GROUP_MAX_FONT,
        current_group_text.clone(),
    );
    autofit_effect(next_group_ref, NEXT_GROUP_MAX_FONT, next_group_text.clone());
    autofit_effect(current_song_ref, CURRENT_SONG_MAX_FONT, current_song_text.clone());
    autofit_effect(next_song_ref, NEXT_SONG_MAX_FONT, next_song_text.clone());

    view! {
        <div class="stage-container" data-layout="worship-snv">
            <div class="stage__current-group">
                <span class="stage__debug-label">"current-group"</span>
                <div node_ref=current_group_ref class="stage__group-pill" style=current_group_style>
                    {current_group_text}
                </div>
            </div>

            <div class="stage__current-song">
                <span class="stage__debug-label">"current-song"</span>
                <div node_ref=current_song_ref class="stage__song-name-text">
                    {current_song_text}
                </div>
            </div>

            <div class="stage__current-slide">
                <span class="stage__debug-label">"current-slide"</span>
                <div node_ref=current_text_ref class="stage__slide-text">
                    {current_text}
                </div>
            </div>

            <div class="stage__next-group">
                <span class="stage__debug-label">"next-group"</span>
                <div node_ref=next_group_ref class="stage__group-pill" style=next_group_style>
                    {next_group_text}
                </div>
            </div>

            <div class="stage__next-song">
                <span class="stage__debug-label">"next-song"</span>
                <div node_ref=next_song_ref class="stage__song-name-text">
                    {next_song_text}
                </div>
            </div>

            <div class="stage__next-slide">
                <span class="stage__debug-label">"next-slide"</span>
                <div node_ref=next_text_ref class="stage__slide-text">
                    {next_text}
                </div>
            </div>

            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}
