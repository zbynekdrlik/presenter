//! The on-air strip (spec §7.1 item 1): what is on the wall right now, from
//! the server — whoever put it there. `AppContext::active_bible_slide` is fed
//! by the operator WS (`BibleSlide` / `BibleCleared`), by this console's own
//! shows, and re-read from `GET /bible/active-slide` on mount and whenever the
//! page becomes visible again (a locked phone's socket may have missed it).

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

fn refresh(on_air: RwSignal<Option<BibleSlideOutput>>) {
    leptos::task::spawn_local(async move {
        if let Ok(output) = crate::api::bible::get_active_slide_output().await {
            on_air.set(output);
        }
    });
}

#[component]
pub(super) fn OnAirStrip() -> impl IntoView {
    let on_air = use_ctx!(AppContext).active_bible_slide;
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
                        <span>
                            "Na stene: "
                            <b data-role="console-on-air-reference">{reference}</b>
                        </span>
                        <span class="console-on-air__codes" data-role="console-on-air-codes">
                            {codes}
                        </span>
                    }
                        .into_any()
                }
                None => view! { <span>"Na stene: nič"</span> }.into_any(),
            }}
        </div>
    }
}
