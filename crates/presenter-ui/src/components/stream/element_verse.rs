//! Bible-verse element for the stream output page (#710; block fade #863).
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
//! The four lines crossfade as ONE block (#863, `text_block.rs`): a change of
//! any shown line fades the whole stack, and nothing visible ever moves.
//!
//! Visibility (AC "BibleCleared ⇒ verse gone / no active content ⇒ nothing"): a
//! line is shown only when its source string is non-empty (and, for the
//! secondary lines, when `show_secondary`); a cleared slide (`None`) empties the
//! block key, so the element is DOM-empty and the transparent output stays
//! clean.

use leptos::prelude::*;
use presenter_core::{BibleSlideOutput, ContentTransition, Frame, TextStyle};

use super::style::frame_css;
use super::text_block::{block_key, block_render, line_style, BlockLine};
use super::transition::Crossfade;
use super::StreamContext;

/// The lines a block shows for `output`, in render order: text, reference,
/// secondary text, secondary reference. The secondary pair stays empty when the
/// element hides it, so a change there never fades the block.
fn verse_lines(output: &BibleSlideOutput, show_secondary: bool) -> [String; 4] {
    let secondary = |line: &str| {
        if show_secondary {
            line.to_string()
        } else {
            String::new()
        }
    };
    [
        output.main_text.clone(),
        output.main_reference.clone(),
        secondary(&output.secondary_text),
        secondary(&output.secondary_reference),
    ]
}

/// The look of the four lines, in [`verse_lines`] order. Both references share
/// `reference_style`.
fn verse_block_lines(
    text: &TextStyle,
    secondary: &TextStyle,
    reference: &TextStyle,
) -> Vec<BlockLine> {
    vec![
        BlockLine {
            role: "stream-verse-text",
            class: "stream-verse__text",
            style: line_style(text),
        },
        BlockLine {
            role: "stream-verse-reference",
            class: "stream-verse__reference",
            style: line_style(reference),
        },
        BlockLine {
            role: "stream-verse-secondary-text",
            class: "stream-verse__text stream-verse__secondary",
            style: line_style(secondary),
        },
        BlockLine {
            role: "stream-verse-secondary-reference",
            class: "stream-verse__reference stream-verse__secondary-reference",
            style: line_style(reference),
        },
    ]
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
    /// How a verse change animates the block (#716, #863): `Fade` crossfades
    /// the old block out while the new fades in, `FadeThrough` fades the old
    /// out first, `Cut` swaps instantly.
    content_transition: ContentTransition,
) -> impl IntoView {
    let ctx = use_context::<StreamContext>().expect("StreamContext not provided");
    // The Frame box only: the block wrapper fills it (`.stream-text-block`).
    let container = frame_css(&frame, z);
    let render = block_render(verse_block_lines(
        &text_style,
        &secondary_style,
        &reference_style,
    ));

    // A `Memo` over the (small) Bible output so the block crossfades only on a
    // genuine change of a SHOWN line. `show_secondary` is static element config
    // (the translation-vs-combined switch, arch §4). A cleared slide ⇒ an empty
    // key ⇒ the block fades out, then the element is DOM-empty (count 0).
    let block = Memo::new(move |_| {
        ctx.bible.with(|b| {
            b.as_ref()
                .map(|o| block_key(&verse_lines(o, show_secondary)))
                .unwrap_or_default()
        })
    });

    view! {
        <div
            class="stream-element stream-element--verse"
            data-role="stream-element-verse"
            data-element-id=id.to_string()
            style=container
        >
            <Crossfade
                content=block
                transition=content_transition
                role="stream-verse-block"
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

    fn output(
        text: &str,
        reference: &str,
        secondary: &str,
        secondary_ref: &str,
    ) -> BibleSlideOutput {
        BibleSlideOutput::new(
            text,
            reference,
            secondary,
            secondary_ref,
            chrono::Utc::now(),
        )
    }

    #[test]
    fn only_hidden_lines_is_nothing_shown() {
        let hidden_only = verse_lines(&output("", "", "Lebo tak", "Ján 3:16"), false);
        assert_eq!(
            block_key(&hidden_only),
            "",
            "only hidden lines ⇒ nothing shown"
        );
    }

    #[test]
    fn a_reference_only_change_changes_the_key() {
        let a = block_key(&verse_lines(
            &output("Ježiš zaplakal.", "Ján 11:35", "", ""),
            false,
        ));
        let b = block_key(&verse_lines(
            &output("Ježiš zaplakal.", "Ján 11:36", "", ""),
            false,
        ));
        assert_ne!(
            a, b,
            "the whole block fades when only the reference changes"
        );
    }

    #[test]
    fn a_text_change_with_the_same_reference_changes_the_key() {
        let long = block_key(&verse_lines(
            &output("Lebo tak Boh miloval svet", "Ján 3:16", "", ""),
            false,
        ));
        let short = block_key(&verse_lines(
            &output("Ježiš zaplakal.", "Ján 3:16", "", ""),
            false,
        ));
        assert_ne!(long, short);
    }

    #[test]
    fn hidden_secondary_lines_never_change_the_key() {
        let a = verse_lines(&output("Text", "Ref", "Secondary A", "Sec ref A"), false);
        let b = verse_lines(&output("Text", "Ref", "Secondary B", "Sec ref B"), false);
        assert_eq!(block_key(&a), block_key(&b));
        assert_eq!(a[2], "");
        assert_eq!(a[3], "");
    }

    #[test]
    fn shown_secondary_lines_are_part_of_the_block_in_role_order() {
        let lines = verse_lines(&output("Text", "Ref", "Secondary", "Sec ref"), true);
        assert_eq!(
            lines,
            ["Text", "Ref", "Secondary", "Sec ref"].map(String::from)
        );
        let roles: Vec<_> = verse_block_lines(&style(), &style(), &style())
            .iter()
            .map(|l| l.role)
            .collect();
        assert_eq!(
            roles,
            [
                "stream-verse-text",
                "stream-verse-reference",
                "stream-verse-secondary-text",
                "stream-verse-secondary-reference",
            ]
        );
        let other = verse_lines(&output("Text", "Ref", "Secondary 2", "Sec ref"), true);
        assert_ne!(block_key(&lines), block_key(&other));
    }

    fn style() -> TextStyle {
        serde_json::from_value(serde_json::json!({
            "fontFamily": "Inter",
            "sizePct": 6,
            "color": "#ffffff",
            "weight": 700,
            "align": "center",
            "lineHeight": 1.2
        }))
        .expect("a valid TextStyle")
    }
}
