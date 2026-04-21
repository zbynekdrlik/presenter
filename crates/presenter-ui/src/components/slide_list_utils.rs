use wasm_bindgen::JsCast;

/// Format text with `<br>` for line breaks and highlight lines exceeding limit.
pub(super) fn format_multiline(text: &str, limit: u32) -> String {
    text.lines()
        .map(|line| {
            let escaped = html_escape(line);
            if limit > 0 && line.len() as u32 > limit {
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
    limit > 0 && text.lines().any(|line| line.len() as u32 > limit)
}

pub(super) fn slide_has_any_warning(main: &str, translation: &str, stage: &str, limit: u32) -> bool {
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
