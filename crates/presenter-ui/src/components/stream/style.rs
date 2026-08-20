//! Shared CSS mapping helpers for stream-graphics text elements (#710).
//!
//! The typed `Frame` / `TextStyle` / `Shadow` props map to inline CSS the same
//! way for every text element (countdown / lyrics / verse). #709's countdown
//! introduced this mapping inline; #710 extracts the pure helpers here so the
//! three elements share ONE implementation instead of duplicating it. The
//! output is byte-equivalent to #709's original countdown CSS (only the source
//! location moved), so the countdown E2E asserts (font-size in `vh`, text-shadow
//! present) stay green.

use presenter_core::{Frame, TextAlign, TextStyle};

/// CSS font stack for a whitelisted family. The three OFL families (Inter,
/// Bebas Neue, Oswald) gracefully fall back until their `@font-face` woff2
/// assets are bundled (see the #709 hand-back) — no `@font-face` pointing at a
/// missing file, so no 404 breaks the zero-console E2E gate.
pub(super) fn css_font_family(family: &str) -> String {
    format!("\"{family}\", system-ui, sans-serif")
}

/// CSS `text-align` value for a [`TextAlign`].
pub(super) fn css_align(align: TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
    }
}

/// CSS flex `justify-content` value matching a [`TextAlign`] — used when the
/// element itself is the flex line box (the countdown).
pub(super) fn css_justify(align: TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "flex-start",
        TextAlign::Center => "center",
        TextAlign::Right => "flex-end",
    }
}

/// `left/top/width/height` (percent of the 16:9 canvas) + `z-index` from a
/// [`Frame`] and the element's `z_order`.
pub(super) fn frame_css(frame: &Frame, z: i32) -> String {
    format!(
        "left:{}%;top:{}%;width:{}%;height:{}%;z-index:{};",
        frame.x_pct, frame.y_pct, frame.w_pct, frame.h_pct, z
    )
}

/// Typography CSS from a [`TextStyle`]: font family, size (`size_pct` ⇒ `vh`),
/// color, weight, text-align, line-height, and an optional text-shadow.
pub(super) fn text_style_css(style: &TextStyle) -> String {
    let mut css = format!(
        "font-family:{};font-size:{}vh;color:{};font-weight:{};text-align:{};line-height:{};",
        css_font_family(&style.font_family),
        style.size_pct,
        style.color,
        style.weight,
        css_align(style.align),
        style.line_height,
    );
    if let Some(shadow) = &style.shadow {
        css.push_str(&format!(
            "text-shadow:{}px {}px {}px {};",
            shadow.x_px, shadow.y_px, shadow.blur_px, shadow.color
        ));
    }
    css
}
