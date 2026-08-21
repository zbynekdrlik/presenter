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
use presenter_core::{ContentTransition, Frame, TextStyle};

use super::style::{frame_css, text_style_css};
use super::transition::CrossfadeText;
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
    /// How a verse change animates all four lines (#716): `Fade` crossfades the
    /// old lines out while the new fade in; `Cut` swaps instantly.
    content_transition: ContentTransition,
) -> impl IntoView {
    let ctx = use_context::<StreamContext>().expect("StreamContext not provided");
    let container = container_style(&frame, z);
    let text_css = line_style(&text_style);
    let secondary_css = line_style(&secondary_style);
    // Both references share `reference_style`, but each style string is moved into
    // its own `CrossfadeText`, so build one per reference line.
    let main_reference_css = line_style(&reference_style);
    let secondary_reference_css = line_style(&reference_style);

    // `Memo`s over the (small) Bible output so `CrossfadeText` crossfades only on
    // a genuine verse change. Empty text ⇒ the line renders nothing (a
    // `BibleCleared` fades every line out then removes it — count 0).
    let main_text = Memo::new(move |_| {
        ctx.bible
            .with(|b| b.as_ref().map(|o| o.main_text.clone()).unwrap_or_default())
    });
    let main_reference = Memo::new(move |_| {
        ctx.bible.with(|b| {
            b.as_ref()
                .map(|o| o.main_reference.clone())
                .unwrap_or_default()
        })
    });
    let secondary_text = Memo::new(move |_| {
        ctx.bible.with(|b| {
            b.as_ref()
                .map(|o| o.secondary_text.clone())
                .unwrap_or_default()
        })
    });
    let secondary_reference = Memo::new(move |_| {
        ctx.bible.with(|b| {
            b.as_ref()
                .map(|o| o.secondary_reference.clone())
                .unwrap_or_default()
        })
    });

    // The main lines are always rendered (visibility follows content); the
    // secondary lines follow the STATIC `show_secondary` toggle (`show_secondary`
    // IS the translation-vs-combined switch, arch §4).
    let ct_secondary_text = content_transition.clone();
    let ct_secondary_reference = content_transition.clone();
    let secondary_text_view = show_secondary.then(move || {
        view! {
            <CrossfadeText
                text=secondary_text
                transition=ct_secondary_text
                role="stream-verse-secondary-text"
                wrapper_class="stream-verse__text stream-verse__secondary"
                wrapper_style=secondary_css
                fill=true
            />
        }
    });
    let secondary_reference_view = show_secondary.then(move || {
        view! {
            <CrossfadeText
                text=secondary_reference
                transition=ct_secondary_reference
                role="stream-verse-secondary-reference"
                wrapper_class="stream-verse__reference stream-verse__secondary-reference"
                wrapper_style=secondary_reference_css
                fill=true
            />
        }
    });

    view! {
        <div
            class="stream-element stream-element--verse"
            data-role="stream-element-verse"
            data-element-id=id.to_string()
            style=container
        >
            <CrossfadeText
                text=main_text
                transition=content_transition.clone()
                role="stream-verse-text"
                wrapper_class="stream-verse__text"
                wrapper_style=text_css
                fill=true
            />
            <CrossfadeText
                text=main_reference
                transition=content_transition
                role="stream-verse-reference"
                wrapper_class="stream-verse__reference"
                wrapper_style=main_reference_css
                fill=true
            />
            {secondary_text_view}
            {secondary_reference_view}
        </div>
    }
}
