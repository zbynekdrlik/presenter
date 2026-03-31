use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_text;
use crate::utils::color::{group_color, hex_to_rgba};
use crate::ws::stage::StageWsState;

const CURRENT_MAX_FONT: f64 = 100.0;
const NEXT_MAX_FONT: f64 = 60.0;

#[component]
pub fn WorshipPp(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let current_text_ref = NodeRef::<leptos::html::Div>::new();
    let next_text_ref = NodeRef::<leptos::html::Div>::new();

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

    let current_group = move || ctx.snapshot.get().and_then(|s| s.current.and_then(|sl| sl.group));
    let next_group = move || ctx.snapshot.get().and_then(|s| s.next.and_then(|sl| sl.group));
    let playlist_entries = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.playlist_entries)
            .unwrap_or_default()
    };

    // Auto-fit effects
    {
        let r = current_text_ref;
        Effect::new(move |_| {
            let _t = current_text();
            if let Some(el) = r.get() {
                let html_el: &HtmlElement = &el;
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, CURRENT_MAX_FONT);
                });
                let _ = web_sys::window()
                    .expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }
    {
        let r = next_text_ref;
        Effect::new(move |_| {
            let _t = next_text();
            if let Some(el) = r.get() {
                let html_el: &HtmlElement = &el;
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, NEXT_MAX_FONT);
                });
                let _ = web_sys::window()
                    .expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }

    view! {
        <div class="stage-container" data-layout="worship-pp">
            <div class="stage-pp__slides-area">
                <div class="stage__current-group" style="left:14%;width:72%;">
                    {move || {
                        current_group()
                            .map(|name| {
                                let color = group_color(&name);
                                let bg = hex_to_rgba(color, 0.25);
                                view! {
                                    <span
                                        class="stage__group-pill"
                                        style=format!("color:{color};background:{bg};")
                                    >
                                        {name}
                                    </span>
                                }
                            })
                    }}
                </div>
                <div class="stage__current-slide" style="width:66%;left:2%;">
                    <div node_ref=current_text_ref class="stage__slide-text">
                        {current_text}
                    </div>
                </div>
                <div class="stage__next-group" style="left:14%;width:72%;">
                    {move || {
                        next_group()
                            .map(|name| {
                                let color = group_color(&name);
                                let bg = hex_to_rgba(color, 0.25);
                                view! {
                                    <span
                                        class="stage__group-pill"
                                        style=format!("color:{color};background:{bg};")
                                    >
                                        {name}
                                    </span>
                                }
                            })
                    }}
                </div>
                <div class="stage__next-slide" style="width:66%;left:2%;">
                    <div node_ref=next_text_ref class="stage__slide-text">
                        {next_text}
                    </div>
                </div>
            </div>

            <div class="stage-pp__playlist-sidebar">
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
