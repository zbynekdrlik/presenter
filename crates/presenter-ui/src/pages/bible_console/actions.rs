//! The sticky action bar (spec §7.1 item 7). ZOBRAZIŤ is the only control that
//! sends to the wall; Zhasnúť lives in the on-air strip, never near it.
//! Stepping (◀ / ĎALŠÍ / ▶) comes with B2b. A press keeps the focus in the
//! field: a blur removes the preview's own button, this one moves up under the
//! finger and the click lands on nothing.

use leptos::prelude::*;

use super::{show, Console, Preview};
use crate::state::AppContext;

#[component]
pub(super) fn ConsoleActions(console: Console) -> impl IntoView {
    let c = console;
    let ctx = use_ctx!(AppContext);
    let show_button = NodeRef::<leptos::html::Button>::new();

    // Desktop Enter asked for ZOBRAZIŤ's focus: give it once it is enabled,
    // deferred behind the render that enables it (#819).
    Effect::new(move |_| {
        if c.focus_show.get() && c.can_show.get() {
            c.focus_show.set(false);
            leptos::task::spawn_local(async move {
                if let Some(button) = show_button.get_untracked() {
                    let _ = button.focus();
                }
            });
        }
    });

    let label = move || {
        if c.showing.get() {
            return "Zobrazujem\u{2026}".to_string();
        }
        c.preview.with(|p| match p {
            Preview::Ready { slides, index, .. } if slides.len() > 1 => {
                format!("ZOBRAZIŤ {}/{}", index + 1, slides.len())
            }
            _ => "ZOBRAZIŤ".to_string(),
        })
    };

    view! {
        {move || {
            c.show_error
                .get()
                .map(|text| {
                    view! {
                        <p class="console-error" data-role="console-show-error" role="alert">
                            {text}
                        </p>
                    }
                })
        }}
        <div class="console-bar" data-role="console-bar">
            <button
                type="button"
                class="console-bar__show"
                data-role="console-show"
                node_ref=show_button
                disabled=move || !c.can_show.get()
                on:pointerdown=|ev: web_sys::PointerEvent| ev.prevent_default()
                on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                on:click={
                    let ctx = ctx.clone();
                    move |_| show(c, ctx.clone())
                }
            >
                {label}
            </button>
        </div>
    }
}
