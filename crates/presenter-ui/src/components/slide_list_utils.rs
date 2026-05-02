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

/// Pure reorder: given a slide id list and a drag/target pair, returns the new
/// ordering, or `None` if the drag is a no-op (same id, missing ids).
///
/// Direction-based insertion: forward drags land AFTER the target, backward
/// drags land BEFORE. This guarantees every distinct drag visibly moves the
/// slide (the previous drop-position heuristic could be a no-op on forward
/// drags into a target's upper half).
pub(super) fn reorder_slide_ids(
    ids: Vec<String>,
    dragged: &str,
    target: &str,
) -> Option<Vec<String>> {
    if dragged == target {
        return None;
    }
    let drag_pos = ids.iter().position(|id| id == dragged)?;
    let target_pos = ids.iter().position(|id| id == target)?;
    let forward = drag_pos < target_pos;
    let mut new_ids = ids;
    new_ids.remove(drag_pos);
    // After removal, target_pos shifts down by 1 if the dragged slide was before it.
    let adjusted_target = if forward { target_pos - 1 } else { target_pos };
    let insert_idx = if forward {
        adjusted_target + 1
    } else {
        adjusted_target
    };
    new_ids.insert(insert_idx, dragged.to_string());
    Some(new_ids)
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

    fn ids(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reorder_forward_drag_lands_after_target() {
        // Drag "a" (pos 0) onto "d" (pos 3) → "a" ends up at pos 3.
        let result = reorder_slide_ids(ids(&["a", "b", "c", "d", "e"]), "a", "d").unwrap();
        assert_eq!(result, ids(&["b", "c", "d", "a", "e"]));
    }

    #[test]
    fn reorder_backward_drag_lands_on_target_position() {
        // Drag "d" (pos 3) onto "a" (pos 0) → "d" ends up at pos 0.
        let result = reorder_slide_ids(ids(&["a", "b", "c", "d", "e"]), "d", "a").unwrap();
        assert_eq!(result, ids(&["d", "a", "b", "c", "e"]));
    }

    #[test]
    fn reorder_adjacent_forward_swap() {
        // Drag "b" onto "c" → "b" and "c" swap.
        let result = reorder_slide_ids(ids(&["a", "b", "c", "d"]), "b", "c").unwrap();
        assert_eq!(result, ids(&["a", "c", "b", "d"]));
    }

    #[test]
    fn reorder_adjacent_backward_swap() {
        // Drag "c" onto "b" → "c" and "b" swap.
        let result = reorder_slide_ids(ids(&["a", "b", "c", "d"]), "c", "b").unwrap();
        assert_eq!(result, ids(&["a", "c", "b", "d"]));
    }

    #[test]
    fn reorder_same_id_returns_none() {
        assert!(reorder_slide_ids(ids(&["a", "b"]), "a", "a").is_none());
    }

    #[test]
    fn reorder_missing_dragged_returns_none() {
        assert!(reorder_slide_ids(ids(&["a", "b"]), "z", "a").is_none());
    }

    #[test]
    fn reorder_missing_target_returns_none() {
        assert!(reorder_slide_ids(ids(&["a", "b"]), "a", "z").is_none());
    }

    #[test]
    fn reorder_preserves_length() {
        let result = reorder_slide_ids(ids(&["a", "b", "c", "d", "e"]), "a", "e").unwrap();
        assert_eq!(result.len(), 5);
    }
}
