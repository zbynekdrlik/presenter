use leptos::prelude::*;

use crate::components::stage::api_text::{slide_api_lines, ApiLines};
use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_effect;
use crate::utils::color::group_pill_style;
use crate::utils::text::break_if_long;
use crate::ws::stage::StageWsState;

const CURRENT_MAX_FONT: f64 = 800.0;
const NEXT_MAX_FONT: f64 = 500.0;
const CURRENT_GROUP_MAX_FONT: f64 = 200.0;
const NEXT_GROUP_MAX_FONT: f64 = 200.0;
const CURRENT_SONG_MAX_FONT: f64 = 200.0;
const NEXT_SONG_MAX_FONT: f64 = 200.0;
const STAGE_SLIDE_BREAK_THRESHOLD: usize = 26;

/// #799: the API box text — each line tail-broken on its own (a joined
/// `both` text already contains a newline, which `break_if_long` skips),
/// then joined.
fn api_box_text(lines: ApiLines) -> String {
    lines
        .map_lines(|line| break_if_long(line, STAGE_SLIDE_BREAK_THRESHOLD))
        .joined()
}

/// `api_text_mode` (#799): the `api` layout sets it so the current/next
/// boxes show the API slide per the operator's text mode (original /
/// translation / both joined on a new line) — same boxes, same sizes. Every
/// other layout leaves it off and keeps the stage-text-else-main behaviour.
#[component]
pub fn WorshipSnv(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
    #[prop(optional)] api_text_mode: bool,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let current_text_ref = NodeRef::<leptos::html::Div>::new();
    let next_text_ref = NodeRef::<leptos::html::Div>::new();
    let current_group_ref = NodeRef::<leptos::html::Div>::new();
    let next_group_ref = NodeRef::<leptos::html::Div>::new();
    let current_song_ref = NodeRef::<leptos::html::Div>::new();
    let next_song_ref = NodeRef::<leptos::html::Div>::new();

    let current_text = move || {
        if api_text_mode {
            return ctx.snapshot.with(|snap| {
                snap.as_ref()
                    .map(|s| api_box_text(slide_api_lines(s.current.as_ref(), s.text_mode)))
                    .unwrap_or_default()
            });
        }
        let raw = ctx
            .snapshot
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
            .unwrap_or_default();
        break_if_long(raw, STAGE_SLIDE_BREAK_THRESHOLD)
    };

    let next_text = move || {
        if api_text_mode {
            return ctx.snapshot.with(|snap| {
                snap.as_ref()
                    .map(|s| api_box_text(slide_api_lines(s.next.as_ref(), s.text_mode)))
                    .unwrap_or_default()
            });
        }
        let raw = ctx
            .snapshot
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
            .unwrap_or_default();
        break_if_long(raw, STAGE_SLIDE_BREAK_THRESHOLD)
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

    autofit_effect(current_text_ref, CURRENT_MAX_FONT, current_text);
    autofit_effect(next_text_ref, NEXT_MAX_FONT, next_text);
    autofit_effect(
        current_group_ref,
        CURRENT_GROUP_MAX_FONT,
        current_group_text,
    );
    autofit_effect(next_group_ref, NEXT_GROUP_MAX_FONT, next_group_text);
    autofit_effect(current_song_ref, CURRENT_SONG_MAX_FONT, current_song_text);
    autofit_effect(next_song_ref, NEXT_SONG_MAX_FONT, next_song_text);

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
