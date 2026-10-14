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
//! The four lines crossfade as ONE block (#863). A change of any shown line
//! fades the whole stack, and each crossfade layer renders the full stack,
//! vertically centred on its own in the same grid cell. The old block fades
//! out where it stands and the new one fades in where it will stay, so nothing
//! visible ever moves. Before #863 each line crossfaded on its own: an
//! unchanged reference never faded, and it jumped once the taller outgoing
//! text layer was removed.
//!
//! Visibility (AC "BibleCleared ⇒ verse gone / no active content ⇒ nothing"): a
//! line is shown only when its source string is non-empty (and, for the
//! secondary lines, when `show_secondary`); a cleared slide (`None`) empties the
//! block key, so the element is DOM-empty and the transparent output stays
//! clean.

use std::sync::Arc;

use leptos::prelude::*;
use presenter_core::{BibleSlideOutput, ContentTransition, Frame, TextStyle};

use super::style::{frame_css, text_style_css};
use super::transition::{Crossfade, LayerRender};
use super::StreamContext;

/// The four lines of a verse block, in render order (#863).
type VerseLines = [String; 4];
const TEXT: usize = 0;
const REFERENCE: usize = 1;
const SECONDARY_TEXT: usize = 2;
const SECONDARY_REFERENCE: usize = 3;

/// The lines a block shows for `output`: the secondary pair stays empty when
/// the element hides it, so a change there never fades the block.
fn verse_lines(output: &BibleSlideOutput, show_secondary: bool) -> VerseLines {
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

/// The block's crossfade content key: empty when no line is shown (the element
/// is then DOM-empty), else the four lines as a JSON array. JSON escapes quotes
/// and newlines, so no verse text can fake a line boundary.
fn block_key(lines: &VerseLines) -> String {
    if lines.iter().all(String::is_empty) {
        return String::new();
    }
    serde_json::to_string(lines).unwrap_or_default()
}

/// The lines encoded in a [`block_key`]; an empty or unreadable key shows
/// nothing.
fn lines_from_key(key: &str) -> VerseLines {
    serde_json::from_str(key).unwrap_or_default()
}

/// Container CSS: just the `Frame` box. The crossfade wrapper fills it and each
/// layer centres the stacked lines vertically (`.stream-verse__block` CSS).
fn container_style(frame: &Frame, z: i32) -> String {
    frame_css(frame, z)
}

/// A single verse line's CSS: its `TextStyle` plus `width:100%` so `text-align`
/// positions the text across the full frame width.
fn line_style(style: &TextStyle) -> String {
    format!("{}width:100%;", text_style_css(style))
}

/// The inline CSS of the four lines, in [`VerseLines`] order. Both references
/// share `reference_style`.
#[derive(Clone)]
struct LineStyles([String; 4]);

impl LineStyles {
    fn new(text: &TextStyle, secondary: &TextStyle, reference: &TextStyle) -> Self {
        Self([
            line_style(text),
            line_style(reference),
            line_style(secondary),
            line_style(reference),
        ])
    }
}

/// One line of a block layer, absent while its text is empty. The text is read
/// reactively: the `Cut` path updates a layer's lines in place.
fn verse_line(
    lines: Memo<VerseLines>,
    index: usize,
    role: &'static str,
    class: &'static str,
    style: String,
) -> impl IntoView {
    move || {
        let text = lines.with(|l| l[index].clone());
        (!text.is_empty()).then(|| {
            view! {
                <div class=class data-role=role style=style.clone()>
                    {text}
                </div>
            }
        })
    }
}

/// One crossfade layer: the whole stack decoded from the layer's block key.
fn verse_layer(key: Signal<String>, styles: &LineStyles) -> AnyView {
    let lines = Memo::new(move |_| lines_from_key(&key.get()));
    let [text_css, reference_css, secondary_css, secondary_reference_css] = styles.0.clone();
    view! {
        {verse_line(lines, TEXT, "stream-verse-text", "stream-verse__text", text_css)}
        {verse_line(
            lines,
            REFERENCE,
            "stream-verse-reference",
            "stream-verse__reference",
            reference_css,
        )}
        {verse_line(
            lines,
            SECONDARY_TEXT,
            "stream-verse-secondary-text",
            "stream-verse__text stream-verse__secondary",
            secondary_css,
        )}
        {verse_line(
            lines,
            SECONDARY_REFERENCE,
            "stream-verse-secondary-reference",
            "stream-verse__reference stream-verse__secondary-reference",
            secondary_reference_css,
        )}
    }
    .into_any()
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
    let container = container_style(&frame, z);
    let styles = LineStyles::new(&text_style, &secondary_style, &reference_style);

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
    let render: LayerRender = Arc::new(move |key: Signal<String>| verse_layer(key, &styles));

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
                wrapper_class="stream-verse__block"
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
    fn the_key_round_trips_every_line_even_with_separator_like_text() {
        let lines: VerseLines = [
            "Riekol: \"Ja som\",\na \"\\\" | ; \u{1f} ]".to_string(),
            "Ján 3:16-17 (SEB)".to_string(),
            "\",\"".to_string(),
            "John 3:16 (KJV)".to_string(),
        ];
        assert_eq!(lines_from_key(&block_key(&lines)), lines);
    }

    #[test]
    fn nothing_shown_is_the_empty_key_and_back() {
        let empty = VerseLines::default();
        assert_eq!(block_key(&empty), "");
        assert_eq!(lines_from_key(""), empty);
        assert_eq!(
            lines_from_key("not json"),
            empty,
            "an unreadable key shows nothing"
        );
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
        assert_eq!(a[SECONDARY_TEXT], "");
        assert_eq!(a[SECONDARY_REFERENCE], "");
    }

    #[test]
    fn shown_secondary_lines_are_part_of_the_block() {
        let lines = verse_lines(&output("Text", "Ref", "Secondary", "Sec ref"), true);
        assert_eq!(lines[TEXT], "Text");
        assert_eq!(lines[REFERENCE], "Ref");
        assert_eq!(lines[SECONDARY_TEXT], "Secondary");
        assert_eq!(lines[SECONDARY_REFERENCE], "Sec ref");
        let other = verse_lines(&output("Text", "Ref", "Secondary 2", "Sec ref"), true);
        assert_ne!(block_key(&lines), block_key(&other));
    }
}
