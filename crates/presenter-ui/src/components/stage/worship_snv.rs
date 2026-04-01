use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_text;
use crate::utils::color::{group_color, hex_to_rgba};
use crate::ws::stage::StageWsState;

const CURRENT_MAX_FONT: f64 = 120.0;
const NEXT_MAX_FONT: f64 = 80.0;

#[component]
pub fn WorshipSnv(
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

    // Auto-fit effect for current text
    {
        let r = current_text_ref;
        Effect::new(move |_| {
            let _text = current_text();
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

    // Auto-fit effect for next text
    {
        let r = next_text_ref;
        Effect::new(move |_| {
            let _text = next_text();
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
        <div class="stage-container" data-layout="worship-snv">
            <div class="stage__current-group">
                <span class="stage__debug-label">"current-group"</span>
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

            <div class="stage__current-slide">
                <span class="stage__debug-label">"current-slide"</span>
                <div node_ref=current_text_ref class="stage__slide-text">
                    {current_text}
                </div>
            </div>

            <div class="stage__next-group">
                <span class="stage__debug-label">"next-group"</span>
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

            <div class="stage__next-slide">
                <span class="stage__debug-label">"next-slide"</span>
                <div node_ref=next_text_ref class="stage__slide-text">
                    {next_text}
                </div>
            </div>

            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
        </div>
    }
}
