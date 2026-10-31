//! The sticky action bar (spec §7.1 item 7): ◀ · ZOBRAZIŤ / ĎALŠÍ · ▶. Only
//! the primary button sends to the wall; ◀ / ▶ move the preview; Zhasnúť lives
//! in the on-air strip, never near it. On an iPhone the bar rides above the
//! keyboard (`--kb-inset`, `state::keyboard_inset`). A press keeps the focus
//! in the field: a blur removes the preview's own button, this one moves up under the
//! finger and the click lands on nothing. A show in flight only marks it busy
//! (`show` refuses a second one): `disabled` would drop the keyboard focus, and
//! Space would then go to the global search instead of the next part; the
//! same goes for the wait while the next verse loads after a show. A held
//! Enter or Space sends nothing: Enter in the field brought the focus here, and its
//! auto-repeat would show the verse.

use leptos::prelude::*;
use presenter_core::bible::resolver::Step;

use super::stepping::{apply_move, neighbour, Move};
use super::{show, Console, Preview};
use crate::state::bible_console_step::short_label;
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

    let split = move || {
        c.preview.with(|p| match p {
            Preview::Ready { slides, index, .. } if slides.len() > 1 => {
                Some((index + 1, slides.len()))
            }
            _ => None,
        })
    };
    let label = move || {
        if c.showing.get() {
            return "Zobrazujem\u{2026}".to_string();
        }
        let part = split()
            .map(|(i, n)| format!(" {i}/{n}"))
            .unwrap_or_default();
        match (c.after_show.get(), c.target.get()) {
            (true, Some(target)) => format!("ĎALŠÍ: {}{part}", short_label(&target)),
            _ => format!("ZOBRAZIŤ{part}"),
        }
    };
    let prev = Memo::new(move |_| neighbour(c, Step::Prev));
    let next = Memo::new(move |_| neighbour(c, Step::Next));
    let step_text = move |step: Option<Move>| match step {
        Some(Move::Slide(index)) => {
            let count = split().map(|(_, n)| n).unwrap_or(0);
            format!("{}/{count}", index + 1)
        }
        Some(Move::Target(target)) => short_label(&target),
        None => String::new(),
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
                class="console-bar__step"
                data-role="console-prev"
                aria-label="Predošlý"
                disabled=move || prev.with(Option::is_none)
                on:pointerdown=|ev: web_sys::PointerEvent| ev.prevent_default()
                on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                on:click=move |_| apply_move(c, Step::Prev)
            >
                {move || format!("\u{25C0} {}", step_text(prev.get()))}
            </button>
            <button
                type="button"
                class="console-bar__show"
                data-role="console-show"
                node_ref=show_button
                disabled=move || !c.ready.get() && !c.after_show.get()
                aria-disabled=move || if c.can_show.get() { "false" } else { "true" }
                on:pointerdown=|ev: web_sys::PointerEvent| ev.prevent_default()
                on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                on:keydown=|ev: web_sys::KeyboardEvent| {
                    if ev.repeat() && matches!(ev.key().as_str(), "Enter" | " ") {
                        ev.prevent_default();
                    }
                }
                on:click={
                    let ctx = ctx.clone();
                    move |_| show(c, ctx.clone())
                }
            >
                {label}
            </button>
            <button
                type="button"
                class="console-bar__step"
                data-role="console-next"
                aria-label="Ďalší"
                disabled=move || next.with(Option::is_none)
                on:pointerdown=|ev: web_sys::PointerEvent| ev.prevent_default()
                on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                on:click=move |_| apply_move(c, Step::Next)
            >
                {move || format!("{} \u{25B6}", step_text(next.get()))}
            </button>
        </div>
    }
}
