//! Shared CSS mapping helpers for stream-graphics text elements (#710).
//!
//! The typed `Frame` / `TextStyle` / `Shadow` props map to inline CSS the same
//! way for every text element (countdown / lyrics / verse). #709's countdown
//! introduced this mapping inline; #710 extracts the pure helpers here so the
//! three elements share ONE implementation instead of duplicating it. The
//! output is byte-equivalent to #709's original countdown CSS (only the source
//! location moved), so the countdown E2E asserts (font-size in `vh`, text-shadow
//! present) stay green.

use presenter_core::{Frame, TextAlign, TextBox, TextStyle};

/// CSS font stack for a whitelisted family. The three OFL families (Inter,
/// Bebas Neue, Oswald) gracefully fall back until their `@font-face` woff2
/// assets are bundled (see the #709 hand-back) — no `@font-face` pointing at a
/// missing file, so no 404 breaks the zero-console E2E gate. Used only by
/// `text_style_css` below, so private.
fn css_font_family(family: &str) -> String {
    // Escape a quote/backslash so an (upload-validated, but be-safe) family name
    // cannot break out of the quoted value (#778 defense in depth).
    let safe = family.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{safe}\", system-ui, sans-serif")
}

/// CSS `text-align` value for a [`TextAlign`]. Used only by `text_style_css`, so
/// private (`css_justify` below is the cross-module one).
fn css_align(align: TextAlign) -> &'static str {
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
    if let Some(letter_spacing) = style.letter_spacing_em {
        css.push_str(&format!("letter-spacing:{letter_spacing}em;"));
    }
    if let Some(shadow) = &style.shadow {
        css.push_str(&format!(
            "text-shadow:{}px {}px {}px {};",
            shadow.x_px, shadow.y_px, shadow.blur_px, shadow.color
        ));
    }
    css
}

/// Inline CSS for a countdown's optional background box (#785): a
/// semi-transparent card with padding + rounded corners, drawn behind the timer
/// text. The box's transparency lives in the `rgba()` BACKGROUND (its own
/// `opacity` as the alpha), NOT a CSS `opacity` — a CSS opacity would fade the
/// TEXT too (same reason `element_lower_third::bar_background` uses rgba).
/// `padding_pct`/`radius_pct` are percentages of canvas height (⇒ `vh`),
/// matching how `size_pct` scales.
pub(super) fn text_box_css(text_box: &TextBox) -> String {
    let (r, g, b) = hex_rgb(&text_box.color).unwrap_or((0, 0, 0));
    let a = text_box.opacity.clamp(0.0, 1.0);
    format!(
        "background:rgba({r},{g},{b},{a});padding:{}vh;border-radius:{}vh;",
        text_box.padding_pct, text_box.radius_pct
    )
}

/// Parse the RGB bytes from a `#rrggbb` / `#rrggbbaa` colour (alpha ignored — a
/// separate `opacity` field is the transparency control). `None` for a malformed
/// value. Shared by `text_box_css` (#785) and `element_lower_third::bar_background`.
pub(super) fn hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}
