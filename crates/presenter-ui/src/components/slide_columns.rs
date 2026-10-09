//! #832: the slides-per-row stepper of the operator slide toolbars (worship
//! and Bible) and the `body` sync that applies the choice to every
//! `.operator__slides` grid. The decision helpers live in the host-tested
//! `state::slide_columns`.

use leptos::prelude::*;

use crate::state::operator::OperatorState;
use crate::state::session;
use crate::state::slide_columns::{
    default_slide_columns, is_dense, step_slide_columns, SLIDE_COLUMNS_KEY,
};

/// Apply this browser's choice to `body`: the inherited
/// `--operator-slide-columns-choice` (removed without a choice, so the CSS
/// default — 3, or 2 on a phone — applies) and the dense-card flag.
pub fn apply_slide_columns(body: &web_sys::HtmlElement, choice: Option<u8>) {
    let style = body.style();
    match choice {
        Some(columns) => {
            let _ = style.set_property("--operator-slide-columns-choice", &columns.to_string());
        }
        None => {
            let _ = style.remove_property("--operator-slide-columns-choice");
        }
    }
    let dense = choice.is_some_and(is_dense);
    let _ = body.set_attribute(
        "data-slide-columns-dense",
        if dense { "true" } else { "false" },
    );
}

/// The viewport width in CSS px; unreadable counts as a desktop.
fn viewport_width() -> f64 {
    web_sys::window()
        .and_then(|window| window.inner_width().ok())
        .and_then(|width| width.as_f64())
        .unwrap_or(f64::MAX)
}

/// "− N +" — slides per row for every slide grid on the page, remembered in
/// this browser only. Without a choice it shows (and steps from) what the grid
/// shows: 2 on a phone, 3 otherwise. A click past 1 or 8 keeps the bound.
#[component]
pub fn SlideColumnsControl() -> impl IntoView {
    let op = use_ctx!(OperatorState);
    let slide_columns = op.slide_columns;
    let width = RwSignal::new(viewport_width());
    // Removed on unmount; `WindowListenerHandle` is `Send`, so `on_cleanup`
    // accepts it in the host build too.
    let resize = window_event_listener_untyped("resize", move |_| width.set(viewport_width()));
    on_cleanup(move || resize.remove());

    let shown = move || {
        slide_columns
            .get()
            .unwrap_or_else(|| default_slide_columns(width.get()))
    };
    let change = move |delta: i8| {
        let default = default_slide_columns(width.get_untracked());
        let next = step_slide_columns(slide_columns.get_untracked(), default, delta);
        slide_columns.set(Some(next));
        // Unavailable storage (private mode, blocked site data) only means
        // the choice is not remembered; it still applies to this page.
        session::set_persistent(SLIDE_COLUMNS_KEY, &next.to_string());
    };

    view! {
        <div
            class="operator__slide-columns"
            data-role="slide-columns-control"
            title="Slides per row (remembered in this browser)"
        >
            <button
                type="button"
                data-role="slide-columns-decrease"
                aria-label="Fewer slides per row"
                on:click=move |_| change(-1)
            >
                "\u{2212}"
            </button>
            <span class="operator__slide-columns-value" data-role="slide-columns-value">
                {shown}
            </span>
            <button
                type="button"
                data-role="slide-columns-increase"
                aria-label="More slides per row"
                on:click=move |_| change(1)
            >
                "+"
            </button>
        </div>
    }
}
