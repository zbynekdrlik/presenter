//! Lyrics element for the stream output page (#710).
//!
//! Binds to the EXISTING worship-stage pipeline — `LiveEvent::Stage { snapshot }`
//! (the same event the stage page consumes) surfaced on `StreamContext.stage`,
//! plus a cold-load of `GET /stage/snapshot` on WS connect (see
//! `pages::stream_output`). Renders the current slide's main line (when
//! `show_main`) and translation line (when `show_translation`), each with its
//! own `TextStyle`, stacked inside the `Frame`.
//!
//! Visibility (AC "no active content ⇒ render nothing"): a line is shown only
//! when its toggle is on AND the current slide carries non-empty text — no
//! current slide, a cleared/broom-blanked stage, or an empty string ⇒ that line
//! is absent from the DOM, so the transparent output stays clean. A 4000-char
//! line is clipped to the `Frame` by the `.stream-element { overflow:hidden }`
//! rule (it never spills outside its box).

use leptos::prelude::*;
use presenter_core::{Frame, TextStyle};

use super::style::{frame_css, text_style_css};
use super::StreamContext;

/// Container CSS: the `Frame` box as a vertically-centered flex column so the
/// main + translation lines stack and center within the element.
fn container_style(frame: &Frame, z: i32) -> String {
    format!(
        "{}display:flex;flex-direction:column;justify-content:center;",
        frame_css(frame, z)
    )
}

/// A single lyrics line's CSS: its `TextStyle` plus `width:100%` so `text-align`
/// positions the text across the full frame width.
fn line_style(style: &TextStyle) -> String {
    format!("{}width:100%;", text_style_css(style))
}

#[component]
pub fn ElementLyrics(
    /// `stream_elements.id` — for E2E targeting + a stable DOM identity.
    id: i64,
    show_main: bool,
    show_translation: bool,
    main_style: TextStyle,
    translation_style: TextStyle,
    frame: Frame,
    /// `z_order` mirrored to `z-index`.
    z: i32,
) -> impl IntoView {
    let ctx = use_context::<StreamContext>().expect("StreamContext not provided");
    let container = container_style(&frame, z);
    let main_css = line_style(&main_style);
    let translation_css = line_style(&translation_style);

    // Read only the needed String out of the (large) `Stage` snapshot via
    // `.with()` — never clone the whole snapshot. Each closure captures only the
    // `Copy` `ctx.stage` signal, so it is itself `Copy` and reusable in both its
    // `<Show when=>` guard and the rendered `{text}` child.
    let main_text = move || {
        ctx.stage.with(|s| {
            s.as_ref()
                .and_then(|s| s.current.as_ref())
                .map(|c| c.main.clone())
                .unwrap_or_default()
        })
    };
    let translation_text = move || {
        ctx.stage.with(|s| {
            s.as_ref()
                .and_then(|s| s.current.as_ref())
                .map(|c| c.translation.clone())
                .unwrap_or_default()
        })
    };
    let show_main_line = move || show_main && !main_text().is_empty();
    let show_translation_line = move || show_translation && !translation_text().is_empty();

    view! {
        <div
            class="stream-element stream-element--lyrics"
            data-role="stream-element-lyrics"
            data-element-id=id.to_string()
            style=container
        >
            <Show when=show_main_line fallback=|| ()>
                <div
                    class="stream-lyrics__line stream-lyrics__main"
                    data-role="stream-lyrics-main"
                    style=main_css.clone()
                >
                    {main_text}
                </div>
            </Show>
            <Show when=show_translation_line fallback=|| ()>
                <div
                    class="stream-lyrics__line stream-lyrics__translation"
                    data-role="stream-lyrics-translation"
                    style=translation_css.clone()
                >
                    {translation_text}
                </div>
            </Show>
        </div>
    }
}
