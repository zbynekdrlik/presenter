//! #832: the slides-per-row stepper of the operator slide toolbars (worship
//! and Bible) and the `body` sync that applies the choice to every
//! `.operator__slides` grid. The decision helpers live in the host-tested
//! `state::slide_columns`.

use leptos::prelude::*;

use crate::state::operator::OperatorState;
use crate::state::session;
use crate::state::slide_columns::{
    is_dense, step_slide_columns, DEFAULT_SLIDE_COLUMNS, SLIDE_COLUMNS_KEY,
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

/// "− N +" — slides per row for every slide grid on the page, remembered in
/// this browser only. A click past 1 or 8 keeps the bound (`step_slide_columns`).
#[component]
pub fn SlideColumnsControl() -> impl IntoView {
    let op = use_ctx!(OperatorState);
    let slide_columns = op.slide_columns;
    let shown = move || slide_columns.get().unwrap_or(DEFAULT_SLIDE_COLUMNS);
    let change = move |delta: i8| {
        let next = step_slide_columns(slide_columns.get_untracked(), delta);
        slide_columns.set(Some(next));
        // Unavailable storage (private mode, blocked site data) only means
        // the choice is not remembered; it still applies to this page.
        let _ = session::try_set_local(SLIDE_COLUMNS_KEY, &next.to_string());
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
