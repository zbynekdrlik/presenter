//! Bible-verse element for the stream output page (#710).
//!
//! Binds to the EXISTING single-source-of-truth Bible pipeline:
//! `LiveEvent::BibleSlide { output: BibleSlideOutput }` and
//! `LiveEvent::BibleCleared` (surfaced on `StreamContext.bible` as
//! `Some(output)` / `None`), plus a cold-load of `GET /bible/active-slide` on WS
//! connect (see `pages::stream_output`). Renders the main verse text +
//! reference, and — when `show_secondary` — the secondary text + reference too.
//! `show_secondary` IS the "translation toggle vs combined SK+translation" the
//! epic-#718 architecture (§4) describes.
//!
//! Visibility (AC "BibleCleared ⇒ verse gone / no active content ⇒ nothing"): a
//! line is shown only when its source string is non-empty (and, for the
//! secondary lines, when `show_secondary`); a cleared slide (`None`) drops every
//! line, so the transparent output stays clean.

use leptos::prelude::*;
use presenter_core::{Frame, TextStyle};

use super::style::{frame_css, text_style_css};
use super::StreamContext;

/// Container CSS: the `Frame` box as a vertically-centered flex column so the
/// verse lines stack and center within the element.
fn container_style(frame: &Frame, z: i32) -> String {
    format!(
        "{}display:flex;flex-direction:column;justify-content:center;",
        frame_css(frame, z)
    )
}

/// A single verse line's CSS: its `TextStyle` plus `width:100%` so `text-align`
/// positions the text across the full frame width.
fn line_style(style: &TextStyle) -> String {
    format!("{}width:100%;", text_style_css(style))
}

#[component]
pub fn ElementVerse(
    /// `stream_elements.id` — for E2E targeting + a stable DOM identity.
    id: i64,
    show_secondary: bool,
    text_style: TextStyle,
    secondary_style: TextStyle,
    reference_style: TextStyle,
    frame: Frame,
    /// `z_order` mirrored to `z-index`.
    z: i32,
) -> impl IntoView {
    let ctx = use_context::<StreamContext>().expect("StreamContext not provided");
    let container = container_style(&frame, z);
    let text_css = line_style(&text_style);
    let secondary_css = line_style(&secondary_style);
    // Both references share `reference_style`, but each `<Show>` child closure
    // must OWN its style string (a non-`Copy` String can be moved into only one
    // closure), so build one per reference line.
    let main_reference_css = line_style(&reference_style);
    let secondary_reference_css = line_style(&reference_style);

    // Each field closure captures only `ctx.bible` (a `Copy` signal), so it is
    // itself `Copy` and can be reused in both its `<Show when=>` guard and the
    // rendered `{text}` child.
    let main_text = move || {
        ctx.bible
            .with(|b| b.as_ref().map(|o| o.main_text.clone()).unwrap_or_default())
    };
    let main_reference = move || {
        ctx.bible.with(|b| {
            b.as_ref()
                .map(|o| o.main_reference.clone())
                .unwrap_or_default()
        })
    };
    let secondary_text = move || {
        ctx.bible.with(|b| {
            b.as_ref()
                .map(|o| o.secondary_text.clone())
                .unwrap_or_default()
        })
    };
    let secondary_reference = move || {
        ctx.bible.with(|b| {
            b.as_ref()
                .map(|o| o.secondary_reference.clone())
                .unwrap_or_default()
        })
    };

    let show_main_text = move || !main_text().is_empty();
    let show_main_reference = move || !main_reference().is_empty();
    let show_secondary_text = move || show_secondary && !secondary_text().is_empty();
    let show_secondary_reference = move || show_secondary && !secondary_reference().is_empty();

    view! {
        <div
            class="stream-element stream-element--verse"
            data-role="stream-element-verse"
            data-element-id=id.to_string()
            style=container
        >
            <Show when=show_main_text>
                <div class="stream-verse__text" data-role="stream-verse-text" style=text_css.clone()>
                    {main_text}
                </div>
            </Show>
            <Show when=show_main_reference>
                <div
                    class="stream-verse__reference"
                    data-role="stream-verse-reference"
                    style=main_reference_css.clone()
                >
                    {main_reference}
                </div>
            </Show>
            <Show when=show_secondary_text>
                <div
                    class="stream-verse__text stream-verse__secondary"
                    data-role="stream-verse-secondary-text"
                    style=secondary_css.clone()
                >
                    {secondary_text}
                </div>
            </Show>
            <Show when=show_secondary_reference>
                <div
                    class="stream-verse__reference stream-verse__secondary-reference"
                    data-role="stream-verse-secondary-reference"
                    style=secondary_reference_css.clone()
                >
                    {secondary_reference}
                </div>
            </Show>
        </div>
    }
}
