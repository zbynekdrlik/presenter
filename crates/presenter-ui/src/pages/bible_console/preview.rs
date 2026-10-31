//! The preview card (spec §7.1 item 4): the slide ZOBRAZIŤ would send, in the
//! current translations. `data-ready="true"` only while it shows the current
//! target (like the console's `data-preview-ready`). While the field has focus
//! the card carries its own ZOBRAZIŤ: on an iPhone the keyboard may cover the
//! sticky bar (spec §12), and this button sits right under the field.

use leptos::prelude::*;

use super::{show, Console, Preview};
use crate::state::AppContext;

#[component]
pub(super) fn ConsolePreview(console: Console) -> impl IntoView {
    let c = console;
    let ctx = use_ctx!(AppContext);
    let state = move || c.preview.with(Preview::state);
    let reference = move || {
        c.preview
            .with(|p| p.slide().map(|s| s.bible_main_reference.clone()))
            .unwrap_or_default()
    };
    let position = move || {
        c.preview.with(|p| match p {
            Preview::Ready { slides, index, .. } => (index + 1, slides.len()),
            _ => (0, 0),
        })
    };
    let body = move || {
        c.preview.with(|p| match p {
            Preview::Ready {
                slides,
                index,
                warning,
                ..
            } => slides.get(*index).map(|slide| {
                let secondary = (!slide.bible_translation.is_empty()).then(|| {
                    view! {
                        <p class="console-preview__secondary" data-role="console-preview-secondary">
                            {slide.bible_translation.clone()}
                        </p>
                    }
                });
                let warning = warning.clone().map(|text| {
                    view! { <p class="console-preview__warning" role="status">{text}</p> }
                });
                view! {
                    <p class="console-preview__reference">{slide.bible_main_reference.clone()}</p>
                    <p class="console-preview__main" data-role="console-preview-main">
                        {slide.bible_main.clone()}
                    </p>
                    {secondary}
                    {warning}
                }
                .into_any()
            }),
            Preview::Loading { .. } => Some(
                view! { <p class="console-preview__status">"Načítavam\u{2026}"</p> }.into_any(),
            ),
            Preview::Failed { message, .. } => Some(
                view! {
                    <p class="console-preview__error" data-role="console-preview-error" role="alert">
                        {message.clone()}
                    </p>
                }
                .into_any(),
            ),
            Preview::Empty => None,
        })
    };

    view! {
        <section
            class="console-preview"
            data-role="console-preview"
            data-state=state
            data-ready=move || if c.ready.get() { "true" } else { "false" }
            data-preview-for=move || c.preview.with(|p| p.key().unwrap_or_default().to_string())
            data-reference=reference
            data-slide-index=move || position().0.to_string()
            data-slide-count=move || position().1.to_string()
            hidden=move || matches!(state(), "empty")
        >
            {body}
            <Show when=move || { position().1 > 1 }>
                <span class="console-preview__position" data-role="console-preview-position">
                    {move || format!("{}/{}", position().0, position().1)}
                </span>
            </Show>
            <Show when=move || c.field_focused.get() && c.can_show.get()>
                <button
                    type="button"
                    class="console-preview__show"
                    data-role="console-preview-show"
                    on:pointerdown=|ev: web_sys::PointerEvent| ev.prevent_default()
                    on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                    on:click={
                        let ctx = ctx.clone();
                        move |_| show(c, ctx.clone())
                    }
                >
                    "ZOBRAZIŤ"
                </button>
            </Show>
        </section>
    }
}
