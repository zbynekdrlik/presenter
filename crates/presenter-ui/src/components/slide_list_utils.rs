use wasm_bindgen::JsCast;

/// Format text with `<br>` for line breaks and highlight lines exceeding limit.
pub(super) fn format_multiline(text: &str, limit: u32) -> String {
    text.lines()
        .map(|line| {
            let escaped = html_escape(line);
            if limit > 0 && line.chars().count() as u32 > limit {
                format!("<span class=\"operator__slide-overflow\">{escaped}</span>")
            } else {
                escaped
            }
        })
        .collect::<Vec<_>>()
        .join("<br>")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(super) fn field_has_warning(text: &str, limit: u32) -> bool {
    limit > 0 && text.lines().any(|line| line.chars().count() as u32 > limit)
}

pub(super) fn slide_has_any_warning(
    main: &str,
    translation: &str,
    stage: &str,
    limit: u32,
) -> bool {
    field_has_warning(main, limit)
        || field_has_warning(translation, limit)
        || field_has_warning(stage, limit)
}

/// Apply is-focused class to a slide card via DOM manipulation.
pub(super) fn apply_focused_class(slide_id: &str) {
    let doc = crate::utils::window::document();
    if let Ok(cards) = doc.query_selector_all(".is-focused") {
        for i in 0..cards.length() {
            if let Some(el) = cards.item(i) {
                if let Ok(el) = el.dyn_into::<web_sys::Element>() {
                    let _ = el.class_list().remove_1("is-focused");
                }
            }
        }
    }
    let selector = format!("[data-slide-id=\"{}\"]", slide_id);
    if let Ok(Some(el)) = doc.query_selector(&selector) {
        let _ = el.class_list().add_1("is-focused");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_line_over_limit_warns() {
        let long_line = "abcdefghijklmnopqrstuvwxyz0123456";
        assert_eq!(long_line.chars().count(), 33);
        assert!(field_has_warning(long_line, 32));
    }

    #[test]
    fn ascii_line_at_limit_does_not_warn() {
        let line = "abcdefghijklmnopqrstuvwxyz012345";
        assert_eq!(line.chars().count(), 32);
        assert!(!field_has_warning(line, 32));
    }

    #[test]
    fn diacritic_line_within_char_limit_does_not_warn() {
        let line = "Nad všetkých vyvyšený bude P";
        assert_eq!(line.chars().count(), 28);
        assert!(line.len() >= 32, "sanity: byte length meets or exceeds 32");
        assert!(
            !field_has_warning(line, 32),
            "28 visible chars must not warn at limit 32"
        );
    }

    #[test]
    fn diacritic_line_over_char_limit_warns() {
        let line = "Požehnaný kto prichádza v mene P";
        assert_eq!(line.chars().count(), 32);
        assert!(!field_has_warning(line, 32));

        let longer = format!("{line}a");
        assert_eq!(longer.chars().count(), 33);
        assert!(field_has_warning(&longer, 32));
    }

    #[test]
    fn format_multiline_does_not_wrap_diacritic_line_under_limit() {
        let line = "Nad všetkých vyvyšený bude P";
        let html = format_multiline(line, 32);
        assert!(
            !html.contains("operator__slide-overflow"),
            "28-char diacritic line should not be wrapped, got: {html}"
        );
    }

    #[test]
    fn format_multiline_wraps_ascii_line_over_limit() {
        let line = "abcdefghijklmnopqrstuvwxyz0123456";
        let html = format_multiline(line, 32);
        assert!(
            html.contains("operator__slide-overflow"),
            "33-char ASCII line should be wrapped, got: {html}"
        );
    }

    #[test]
    fn slide_has_any_warning_considers_all_fields() {
        let ok = "short";
        let long = "abcdefghijklmnopqrstuvwxyz0123456";
        assert!(!slide_has_any_warning(ok, ok, ok, 32));
        assert!(slide_has_any_warning(long, ok, ok, 32));
        assert!(slide_has_any_warning(ok, long, ok, 32));
        assert!(slide_has_any_warning(ok, ok, long, 32));
    }

    #[test]
    fn zero_limit_disables_warnings() {
        let long = "abcdefghijklmnopqrstuvwxyz0123456";
        assert!(!field_has_warning(long, 0));
    }
}
