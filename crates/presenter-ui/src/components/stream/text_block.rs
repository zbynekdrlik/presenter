//! A stack of text lines that crossfades as ONE block (#863): the verse and the
//! lyrics elements.
//!
//! Each element encodes the lines it shows into ONE content key
//! ([`block_key`]) and hands [`block_render`] the look of every line. The
//! generic [`Crossfade`](super::transition::Crossfade) then fades the whole
//! stack on any line change. Every layer renders the full stack, and the
//! `.stream-text-block` CSS makes each layer the same box (the Frame), vertically
//! centring its own lines. The old block fades out where it stands and the new
//! one fades in where it stays, so nothing visible ever moves.
//!
//! Before #863 every line was its own crossfade in a centred flex column: an
//! unchanged line (the verse reference, the lyrics translation) never faded,
//! and it jumped once the taller outgoing layer of the line above was removed.

use std::sync::Arc;

use leptos::prelude::*;
use presenter_core::TextStyle;

use super::style::text_style_css;
use super::transition::{CrossfadeLayer, LayerRender};

/// A block line's CSS: its `TextStyle` plus `width:100%` so `text-align`
/// positions the text across the full frame width.
pub(super) fn line_style(style: &TextStyle) -> String {
    format!("{}width:100%;", text_style_css(style))
}

/// The look of one line of a block, in key order.
pub(super) struct BlockLine {
    /// `data-role` of the line, set on the copy that stays (see [`block_line`]).
    pub(super) role: &'static str,
    pub(super) class: &'static str,
    /// Inline CSS (the line's `TextStyle` + `width:100%`).
    pub(super) style: String,
}

/// The block's crossfade content key: empty when no line is shown (the element
/// is then DOM-empty), else the lines as a JSON array. JSON escapes quotes and
/// newlines, so no text can fake a line boundary. A hidden line is passed as
/// an empty string, so it never fades the block.
pub(super) fn block_key(lines: &[String]) -> String {
    if lines.iter().all(String::is_empty) {
        return String::new();
    }
    serde_json::to_string(lines).unwrap_or_default()
}

/// The lines encoded in a [`block_key`]; an empty or unreadable key shows
/// nothing.
pub(super) fn lines_from_key(key: &str) -> Vec<String> {
    serde_json::from_str(key).unwrap_or_default()
}

/// One line of a block layer, absent while its text is empty. The text is read
/// reactively (the `Cut` path updates a layer's lines in place). The
/// `data-role` is withdrawn while the layer leaves, so a role always names the
/// copy that stays and a selector never matches two copies mid-fade.
fn block_line(
    texts: Memo<Vec<String>>,
    leaving: Signal<bool>,
    index: usize,
    role: &'static str,
    class: &'static str,
    style: String,
) -> impl IntoView {
    move || {
        let text = texts.with(|t| t.get(index).cloned().unwrap_or_default());
        (!text.is_empty()).then(|| {
            view! {
                <div
                    class=class
                    data-role=move || (!leaving.get()).then_some(role)
                    style=style.clone()
                >
                    {text}
                </div>
            }
        })
    }
}

/// The layer renderer of a block whose lines look like `lines` (in key order).
pub(super) fn block_render(lines: Vec<BlockLine>) -> LayerRender {
    Arc::new(move |layer: CrossfadeLayer| {
        let content = layer.content;
        let texts = Memo::new(move |_| lines_from_key(&content.get()));
        lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                block_line(
                    texts,
                    layer.leaving,
                    index,
                    line.role,
                    line.class,
                    line.style.clone(),
                )
            })
            .collect_view()
            .into_any()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_round_trips_every_line_even_with_separator_like_text() {
        let lines = vec![
            "Riekol: \"Ja som\",\na \"\\\" | ; \u{1f} ]".to_string(),
            "Ján 3:16-17 (SEB)".to_string(),
            "\",\"".to_string(),
            String::new(),
        ];
        assert_eq!(lines_from_key(&block_key(&lines)), lines);
    }

    #[test]
    fn nothing_shown_is_the_empty_key_and_back() {
        assert_eq!(block_key(&[String::new(), String::new()]), "");
        assert!(lines_from_key("").is_empty());
        assert!(
            lines_from_key("not json").is_empty(),
            "an unreadable key shows nothing"
        );
    }

    #[test]
    fn any_line_change_changes_the_key() {
        let key = |a: &str, b: &str| block_key(&[a.to_string(), b.to_string()]);
        assert_ne!(key("Long text", "Ref"), key("Short", "Ref"));
        assert_ne!(key("Text", "Ref 1"), key("Text", "Ref 2"));
        assert_eq!(key("Text", "Ref"), key("Text", "Ref"));
    }
}
