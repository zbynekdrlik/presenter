//! Zhasnúť and the sticky action bar (spec §7.1 items 6–7). ZOBRAZIŤ is the
//! only control that sends to the wall; Zhasnúť is outlined red, shown only
//! while something is on air, and never next to it. Stepping (◀ / ĎALŠÍ / ▶)
//! comes with B2b.

use leptos::prelude::*;

use super::{show, Console, Preview};
use crate::state::AppContext;

#[component]
pub(super) fn ConsoleActions(console: Console) -> impl IntoView {
    let c = console;
    let ctx = use_ctx!(AppContext);
    let on_air = ctx.active_bible_slide;
    let clearing = RwSignal::new(false);
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

    let on_clear = {
        let ctx = ctx.clone();
        move |_| {
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
        }
    };
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
        <Show when=move || on_air.with(Option::is_some)>
            <button
                type="button"
                class="console-clear-live"
                data-role="console-clear-live"
                disabled=move || clearing.get()
                on:click=on_clear.clone()
            >
                "Zhasnúť verš"
            </button>
        </Show>
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
