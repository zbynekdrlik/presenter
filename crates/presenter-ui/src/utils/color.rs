/// Compute WCAG relative luminance from a hex color string.
/// Returns a value between 0.0 (black) and 1.0 (white).
fn luminance(hex: &str) -> f64 {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return 0.0;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f64 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f64 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f64 / 255.0;
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Returns "#000000" for light backgrounds, "#ffffff" for dark backgrounds.
pub fn text_color_for_bg(hex: &str) -> &'static str {
    if luminance(hex) > 0.4 {
        "#000000"
    } else {
        "#ffffff"
    }
}

/// Build inline style for a group pill: solid background + contrast text.
pub fn group_pill_style(bg_color: &str) -> String {
    let text = text_color_for_bg(bg_color);
    format!("background:{bg_color};color:{text};")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_background_gets_white_text() {
        assert_eq!(text_color_for_bg("#2E2E8F"), "#ffffff");
        assert_eq!(text_color_for_bg("#1A1A1A"), "#ffffff");
        assert_eq!(text_color_for_bg("#8B1A1A"), "#ffffff");
    }

    #[test]
    fn light_background_gets_black_text() {
        assert_eq!(text_color_for_bg("#F0E020"), "#000000");
        assert_eq!(text_color_for_bg("#E89B7A"), "#000000");
        assert_eq!(text_color_for_bg("#6FE020"), "#000000");
    }

    #[test]
    fn medium_backgrounds() {
        assert_eq!(text_color_for_bg("#E08A3C"), "#000000");
        assert_eq!(text_color_for_bg("#3CB371"), "#000000");
        assert_eq!(text_color_for_bg("#9A9A9A"), "#000000");
    }

    #[test]
    fn group_pill_style_format() {
        let style = group_pill_style("#E02020");
        assert_eq!(style, "background:#E02020;color:#ffffff;");
    }

    #[test]
    fn invalid_hex_defaults_dark() {
        assert_eq!(text_color_for_bg(""), "#ffffff");
        assert_eq!(text_color_for_bg("xyz"), "#ffffff");
    }
}
