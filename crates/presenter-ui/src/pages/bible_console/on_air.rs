//! The on-air strip (spec §7.1 item 1): what is on the wall right now, from
//! the server — whoever put it there. `AppContext::active_bible_slide` is fed
//! by the operator WS (`BibleSlide` / `BibleCleared`), by this console's own
//! shows, and re-read from `GET /bible/active-slide` on mount and whenever the
//! page becomes visible again or the socket reconnects (either may have missed
//! it). Zhasnúť sits here, the whole page away from ZOBRAZIŤ, so it appears
//! under nobody's finger. The strip holds two lines of text at its own height;
//! the codes run on with the reference to stay within them.

use leptos::prelude::*;
use presenter_core::BibleSlideOutput;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::state::bible_console::on_air_parts;
use crate::state::AppContext;

pub(super) fn setup_on_air_sync(on_air: RwSignal<Option<BibleSlideOutput>>) {
    refresh(on_air);
    let document = crate::utils::window::document();
    let watched = document.clone();
    let on_visible = Closure::<dyn Fn()>::new(move || {
        if watched.visibility_state() == web_sys::VisibilityState::Visible {
            refresh(on_air);
        }
    });
    let _ = document
        .add_event_listener_with_callback("visibilitychange", on_visible.as_ref().unchecked_ref());
    on_visible.forget();
}

/// Re-read the wall. An answer older than a change made while it was on its
/// way (a live event, a show, a clear) is dropped.
pub fn refresh(on_air: RwSignal<Option<BibleSlideOutput>>) {
    let asked_over = on_air.get_untracked();
    leptos::task::spawn_local(async move {
        if let Ok(output) = crate::api::bible::get_active_slide_output().await {
            if on_air.with_untracked(|now| *now == asked_over) {
                on_air.set(output);
            }
        }
    });
}

#[component]
pub(super) fn OnAirStrip() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let on_air = ctx.active_bible_slide;
    let clearing = RwSignal::new(false);
    let on_clear = move |_| {
        if clearing.get_untracked() {
            return;
        }
        clearing.set(true);
        let ctx = ctx.clone();
        leptos::task::spawn_local(async move {
            match crate::api::bible::clear_broadcast().await {
                Ok(()) => {
                    ctx.active_bible_slide.set(None);
                    ctx.active_bible_broadcast.set(None);
                }
                Err(e) => ctx.show_toast(&format!("Verš sa nezhasol: {e}"), "error"),
            }
            clearing.set(false);
        });
    };
    let parts = move || on_air.with(|output| output.as_ref().map(on_air_parts));
    view! {
        <div
            class="console-on-air"
            data-role="console-on-air"
            data-live=move || if on_air.with(Option::is_some) { "true" } else { "false" }
            role="status"
        >
            {move || match parts() {
                Some((reference, codes)) => {
                    view! {
                        <span class="console-on-air__text">
                            "Na stene: "
                            <b data-role="console-on-air-reference">{reference}</b>
                            <span class="console-on-air__codes" data-role="console-on-air-codes">
                                {codes}
                            </span>
                        </span>
                    }
                        .into_any()
                }
                None => view! { <span>"Na stene: nič"</span> }.into_any(),
            }}
            <Show when=move || on_air.with(Option::is_some)>
                <button
                    type="button"
                    class="console-clear-live"
                    data-role="console-clear-live"
                    disabled=move || clearing.get()
                    on:pointerdown=|ev: web_sys::PointerEvent| ev.prevent_default()
                    on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                    on:click=on_clear.clone()
                >
                    "Zhasnúť verš"
                </button>
            </Show>
        </div>
    }
}
