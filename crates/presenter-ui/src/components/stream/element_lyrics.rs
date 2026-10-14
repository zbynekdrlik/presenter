//! Lyrics element for the stream output page (#710; block fade #863).
//!
//! Binds to the EXISTING worship-stage pipeline — `LiveEvent::Stage { snapshot }`
//! (the same event the stage page consumes) surfaced on `StreamContext.stage`,
//! plus a cold-load of `GET /stage/snapshot` on WS connect (see
//! `pages::stream_output`). Renders the current slide's main line (when
//! `show_main`) and translation line (when `show_translation`), each with its
//! own `TextStyle`, stacked inside the `Frame`.
//!
//! The two lines crossfade as ONE block (#863, `text_block.rs`): a slide change
//! fades both together, and nothing visible ever moves. Before #863 each line
//! crossfaded on its own, so an unchanged translation never faded and jumped
//! when a taller outgoing main line was removed.
//!
//! Visibility (AC "no active content ⇒ render nothing"): a line is shown only
//! when its toggle is on AND the current slide carries non-empty text — no
//! current slide, a cleared/broom-blanked stage, or an empty string ⇒ that line
//! is absent from the DOM, so the transparent output stays clean. A 4000-char
//! line is clipped to the `Frame` by the `.stream-element { overflow:hidden }`
//! rule (it never spills outside its box).

use leptos::prelude::*;
use presenter_core::{ContentTransition, Frame, StageDisplaySlide, TextStyle};

use super::style::frame_css;
use super::text_block::{block_key, block_render, line_style, BlockLine};
use super::transition::Crossfade;
use super::StreamContext;

/// The lines a block shows for `slide`, in render order: main, translation. A
/// line whose toggle is off stays empty, so it is never rendered and never fades
/// the block.
fn lyric_lines(slide: &StageDisplaySlide, show_main: bool, show_translation: bool) -> [String; 2] {
    let shown = |on: bool, line: &str| if on { line.to_string() } else { String::new() };
    [
        shown(show_main, &slide.main),
        shown(show_translation, &slide.translation),
    ]
}

/// The look of the two lines, in [`lyric_lines`] order.
fn lyric_block_lines(main: &TextStyle, translation: &TextStyle) -> Vec<BlockLine> {
    vec![
        BlockLine {
            role: "stream-lyrics-main",
            class: "stream-lyrics__line stream-lyrics__main",
            style: line_style(main),
        },
        BlockLine {
            role: "stream-lyrics-translation",
            class: "stream-lyrics__line stream-lyrics__translation",
            style: line_style(translation),
        },
    ]
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
    /// How a slide change animates the block (#716, #863): `Fade` crossfades
    /// the old lines out while the new fade in, `FadeThrough` fades the old out
    /// first, `Cut` swaps instantly.
    content_transition: ContentTransition,
) -> impl IntoView {
    let ctx = use_context::<StreamContext>().expect("StreamContext not provided");
    // The Frame box only: the block wrapper fills it (`.stream-text-block`).
    let container = frame_css(&frame, z);
    let render = block_render(lyric_block_lines(&main_style, &translation_style));

    // Read only the current slide out of the (large) `Stage` snapshot via
    // `.with()` — never clone the whole snapshot. A `Memo` so the block
    // crossfades only on a genuine change of a SHOWN line. No current slide or
    // only empty / toggled-off lines ⇒ an empty key ⇒ DOM-empty (the count-0
    // contract the #710 spec asserts). The toggles are STATIC element config.
    let block = Memo::new(move |_| {
        ctx.stage.with(|s| {
            s.as_ref()
                .and_then(|s| s.current.as_ref())
                .map(|c| block_key(&lyric_lines(c, show_main, show_translation)))
                .unwrap_or_default()
        })
    });

    view! {
        <div
            class="stream-element stream-element--lyrics"
            data-role="stream-element-lyrics"
            data-element-id=id.to_string()
            style=container
        >
            <Crossfade
                content=block
                transition=content_transition
                role="stream-lyrics-block"
                wrapper_class="stream-text-block"
                fill=true
                render=render
            />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slide(main: &str, translation: &str) -> StageDisplaySlide {
        StageDisplaySlide {
            main: main.to_string(),
            translation: translation.to_string(),
            stage: String::new(),
            group: None,
            group_color: None,
        }
    }

    #[test]
    fn toggled_off_lines_are_empty_and_never_change_the_key() {
        assert_eq!(
            lyric_lines(&slide("Main", "Preklad"), true, false),
            ["Main".to_string(), String::new()]
        );
        assert_eq!(
            lyric_lines(&slide("Main", "Preklad"), false, true),
            [String::new(), "Preklad".to_string()]
        );
        let a = block_key(&lyric_lines(&slide("Main", "Preklad A"), true, false));
        let b = block_key(&lyric_lines(&slide("Main", "Preklad B"), true, false));
        assert_eq!(a, b);
        assert_eq!(
            block_key(&lyric_lines(&slide("", "Preklad"), true, false)),
            ""
        );
    }

    #[test]
    fn a_main_change_with_the_same_translation_changes_the_key() {
        let long = block_key(&lyric_lines(
            &slide("Amazing grace, how sweet the sound", "Úžasná milosť"),
            true,
            true,
        ));
        let short = block_key(&lyric_lines(&slide("Amazing", "Úžasná milosť"), true, true));
        assert_ne!(long, short, "the whole block fades, translation included");
    }

    #[test]
    fn the_lines_keep_their_roles_in_order() {
        let style: TextStyle = serde_json::from_value(serde_json::json!({
            "fontFamily": "Inter",
            "sizePct": 6,
            "color": "#ffffff",
            "weight": 700,
            "align": "center",
            "lineHeight": 1.2
        }))
        .expect("a valid TextStyle");
        let roles: Vec<_> = lyric_block_lines(&style, &style)
            .iter()
            .map(|l| l.role)
            .collect();
        assert_eq!(roles, ["stream-lyrics-main", "stream-lyrics-translation"]);
    }
}
