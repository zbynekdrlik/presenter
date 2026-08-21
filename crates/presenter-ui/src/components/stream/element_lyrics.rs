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
use presenter_core::{ContentTransition, Frame, TextStyle};

use super::style::{frame_css, text_style_css};
use super::transition::CrossfadeText;
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
    /// How a slide change animates the lines (#716): `Fade` crossfades the old
    /// line out while the new fades in; `Cut` swaps instantly. Applies to both
    /// lines together, so a slide switch shows the same two-layer overlap.
    content_transition: ContentTransition,
) -> impl IntoView {
    let ctx = use_context::<StreamContext>().expect("StreamContext not provided");
    let container = container_style(&frame, z);
    let main_css = line_style(&main_style);
    let translation_css = line_style(&translation_style);

    // Read only the needed String out of the (large) `Stage` snapshot via
    // `.with()` — never clone the whole snapshot. `Memo`s so `CrossfadeText`
    // crossfades only on a genuine slide-text change. Empty text ⇒ the line
    // renders nothing (the wrapper is unmounted), preserving the count-0-on-clear
    // contract the #710 spec asserts.
    let main_text = Memo::new(move |_| {
        ctx.stage.with(|s| {
            s.as_ref()
                .and_then(|s| s.current.as_ref())
                .map(|c| c.main.clone())
                .unwrap_or_default()
        })
    });
    let translation_text = Memo::new(move |_| {
        ctx.stage.with(|s| {
            s.as_ref()
                .and_then(|s| s.current.as_ref())
                .map(|c| c.translation.clone())
                .unwrap_or_default()
        })
    });

    // The `show_main` / `show_translation` toggles are STATIC element config, so
    // a disabled line is never rendered at all (count 0) — content visibility
    // (empty ⇒ absent) is handled inside `CrossfadeText`.
    let ct_main = content_transition.clone();
    let ct_translation = content_transition;
    let main_view = show_main.then(move || {
        view! {
            <CrossfadeText
                text=main_text
                transition=ct_main
                role="stream-lyrics-main"
                wrapper_class="stream-lyrics__line stream-lyrics__main"
                wrapper_style=main_css
                fill=true
            />
        }
    });
    let translation_view = show_translation.then(move || {
        view! {
            <CrossfadeText
                text=translation_text
                transition=ct_translation
                role="stream-lyrics-translation"
                wrapper_class="stream-lyrics__line stream-lyrics__translation"
                wrapper_style=translation_css
                fill=true
            />
        }
    });

    view! {
        <div
            class="stream-element stream-element--lyrics"
            data-role="stream-element-lyrics"
            data-element-id=id.to_string()
            style=container
        >
            {main_view}
            {translation_view}
        </div>
    }
}
