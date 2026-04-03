use leptos::prelude::*;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_effect;
use crate::utils::color::{group_color, hex_to_rgba};
use crate::ws::stage::StageWsState;

const CURRENT_MAX_FONT: f64 = 800.0;
const NEXT_MAX_FONT: f64 = 500.0;
const CURRENT_GROUP_MAX_FONT: f64 = 200.0;
const NEXT_GROUP_MAX_FONT: f64 = 200.0;

#[component]
pub fn WorshipPp(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let current_text_ref = NodeRef::<leptos::html::Div>::new();
    let next_text_ref = NodeRef::<leptos::html::Div>::new();
    let current_group_ref = NodeRef::<leptos::html::Div>::new();
    let next_group_ref = NodeRef::<leptos::html::Div>::new();

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
    let playlist_entries = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.playlist_entries)
            .unwrap_or_default()
    };

    let current_group_style = move || {
        current_group().map_or(String::new(), |name| {
            let color = group_color(&name);
            let bg = hex_to_rgba(color, 0.25);
            format!("color:{color};background:{bg};")
        })
    };
    let next_group_style = move || {
        next_group().map_or(String::new(), |name| {
            let color = group_color(&name);
            let bg = hex_to_rgba(color, 0.25);
            format!("color:{color};background:{bg};")
        })
    };
    let current_group_text = move || current_group().unwrap_or_default();
    let next_group_text = move || next_group().unwrap_or_default();

    autofit_effect(current_text_ref, CURRENT_MAX_FONT, current_text.clone());
    autofit_effect(next_text_ref, NEXT_MAX_FONT, next_text.clone());
    autofit_effect(
        current_group_ref,
        CURRENT_GROUP_MAX_FONT,
        current_group_text.clone(),
    );
    autofit_effect(next_group_ref, NEXT_GROUP_MAX_FONT, next_group_text.clone());

    view! {
        <div class="stage-container" data-layout="worship-pp">
            <div class="stage-pp__slides-area">
                <span class="stage__debug-label">"slides-area"</span>
                <div class="stage__current-group" style="left:14%;width:72%;">
                    <span class="stage__debug-label">"current-group"</span>
                    <div node_ref=current_group_ref class="stage__group-pill" style=current_group_style>
                        {current_group_text}
                    </div>
                </div>
                <div class="stage__current-slide" style="width:66%;left:2%;">
                    <span class="stage__debug-label">"current-slide"</span>
                    <div node_ref=current_text_ref class="stage__slide-text">
                        {current_text}
                    </div>
                </div>
                <div class="stage__next-group" style="left:14%;width:72%;">
                    <span class="stage__debug-label">"next-group"</span>
                    <div node_ref=next_group_ref class="stage__group-pill" style=next_group_style>
                        {next_group_text}
                    </div>
                </div>
                <div class="stage__next-slide" style="width:66%;left:2%;">
                    <span class="stage__debug-label">"next-slide"</span>
                    <div node_ref=next_text_ref class="stage__slide-text">
                        {next_text}
                    </div>
                </div>
            </div>

            <div class="stage-pp__playlist-sidebar">
                <span class="stage__debug-label">"playlist-sidebar"</span>
                <For
                    each=playlist_entries
                    key=|entry| entry.name.clone()
                    children=move |entry| {
                        let class = if entry.is_active {
                            "stage-pp__playlist-entry stage-pp__playlist-entry--active"
                        } else {
                            "stage-pp__playlist-entry"
                        };
                        view! { <div class=class>{entry.name.clone()}</div> }
                    }
                />
            </div>

            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
        </div>
    }
}
