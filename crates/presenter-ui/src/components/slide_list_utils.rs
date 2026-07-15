use presenter_core::{resolve_sequence, Presentation};
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

/// Whether ANY lyrics field on a slide exceeds the per-line character limit.
///
/// #515: the stage field is intentionally NOT a parameter here — it is
/// free-form speaker/reading text and must never contribute to this
/// warning, unlike the lyrics `main`/`translation` fields.
pub(super) fn slide_has_any_warning(main: &str, translation: &str, limit: u32) -> bool {
    field_has_warning(main, limit) || field_has_warning(translation, limit)
}

/// The group-cascade-derived pieces of a single slide's rendering that must
/// recompute live after ANY slide's group is saved (#556 F2) — not just once
/// per full slide-list render, since the Group save path deliberately uses
/// `update_untracked` (see `slide_save.rs::apply_saved_content_locally`) to
/// avoid a full, unkeyed list rebuild on every save.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct GroupDisplay {
    /// The effective group badge text for the header pill; `None` suppresses
    /// the badge entirely — no group at all, or a "true blank" slide (empty
    /// main AND no explicit group, the #275 empty-bookend rule).
    pub badge_text: Option<String>,
    /// Whether the badge is the dimmed INHERITED variant vs. this slide's
    /// own explicit group.
    pub badge_inherited: bool,
    /// The Group input's placeholder (ghost text showing what group this
    /// slide would inherit) — empty when the slide has its own explicit
    /// group.
    pub edit_placeholder: String,
}

/// Resolve `slide_id`'s badge/placeholder by re-running the group cascade
/// over the FULL, current `pres.slides` — a fresh, cheap O(n) computation
/// rather than depending on a stale snapshot captured at a full-list render.
pub(super) fn resolve_group_display(pres: &Presentation, slide_id: &str) -> GroupDisplay {
    let resolved = resolve_sequence(&pres.slides);
    let mut prev_effective: Option<String> = None;
    for (raw, resolved_slide) in pres.slides.iter().zip(resolved.iter()) {
        let effective_group_name = resolved_slide
            .effective_group
            .as_ref()
            .map(|g| g.name().to_string());
        let is_new = effective_group_name != prev_effective;
        if raw.id.to_string() == slide_id {
            let explicit_group_name = raw.content.group.as_ref().map(|g| g.name().to_string());
            let main_is_empty = resolved_slide.main.value().is_empty();
            let badge_text = if main_is_empty && explicit_group_name.is_none() {
                None
            } else {
                effective_group_name.clone()
            };
            let badge_inherited = effective_group_name.is_some() && !is_new;
            let edit_placeholder = if explicit_group_name.is_none() {
                effective_group_name.unwrap_or_default()
            } else {
                String::new()
            };
            return GroupDisplay {
                badge_text,
                badge_inherited,
                edit_placeholder,
            };
        }
        prev_effective = effective_group_name;
    }
    GroupDisplay::default()
}

#[cfg(test)]
mod group_display_tests {
    use super::*;
    use presenter_core::{Slide, SlideContent, SlideGroup, SlideText};

    fn slide(main: &str, group: Option<&str>) -> Slide {
        Slide::new(
            0,
            SlideContent::new(
                SlideText::new(main).unwrap(),
                SlideText::new("").unwrap(),
                SlideText::new("").unwrap(),
                group.map(|g| SlideGroup::new(g.to_string())),
            ),
        )
    }

    fn presentation(mut slides: Vec<Slide>) -> Presentation {
        // `Presentation::new` requires unique `order` values; the `slide()`
        // helper above always builds with `order: 0`, so reindex here.
        for (i, s) in slides.iter_mut().enumerate() {
            s.order = i as u32;
        }
        Presentation::new("Test", slides).unwrap()
    }

    #[test]
    fn first_slide_with_explicit_group_is_not_inherited() {
        let pres = presentation(vec![slide("A", Some("Verse 1"))]);
        let display = resolve_group_display(&pres, &pres.slides[0].id.to_string());
        assert_eq!(display.badge_text, Some("Verse 1".to_string()));
        assert!(!display.badge_inherited);
        assert_eq!(display.edit_placeholder, "");
    }

    #[test]
    fn following_slide_without_its_own_group_inherits_and_is_marked_inherited() {
        let pres = presentation(vec![slide("A", Some("Verse 1")), slide("B", None)]);
        let second_id = pres.slides[1].id.to_string();
        let display = resolve_group_display(&pres, &second_id);
        assert_eq!(display.badge_text, Some("Verse 1".to_string()));
        assert!(display.badge_inherited);
        assert_eq!(
            display.edit_placeholder, "Verse 1",
            "an inheriting slide's placeholder shows what it would inherit"
        );
    }

    #[test]
    fn a_later_explicit_group_starts_a_new_non_inherited_section() {
        let pres = presentation(vec![
            slide("A", Some("Verse 1")),
            slide("B", None),
            slide("C", Some("Chorus")),
        ]);
        let third_id = pres.slides[2].id.to_string();
        let display = resolve_group_display(&pres, &third_id);
        assert_eq!(display.badge_text, Some("Chorus".to_string()));
        assert!(!display.badge_inherited);
    }

    #[test]
    fn true_blank_bookend_slide_suppresses_the_badge_entirely() {
        // #275: an empty main AND no explicit group must render NO badge,
        // even though it would otherwise inherit the previous group.
        let pres = presentation(vec![slide("A", Some("Verse 1")), slide("", None)]);
        let blank_id = pres.slides[1].id.to_string();
        let display = resolve_group_display(&pres, &blank_id);
        assert_eq!(display.badge_text, None);
    }

    #[test]
    fn unknown_slide_id_returns_default() {
        let pres = presentation(vec![slide("A", Some("Verse 1"))]);
        let display = resolve_group_display(&pres, "does-not-exist");
        assert_eq!(display, GroupDisplay::default());
    }

    #[test]
    fn editing_an_earlier_slides_group_cascades_forward_when_recomputed() {
        // #556 F2 regression shape: slide[0] gets a NEW explicit group after
        // slides[1..] were already rendered inheriting the OLD state. A
        // fresh `resolve_group_display` call (as the fine-grained closure
        // does on every `groups_version` bump) must reflect the new value
        // immediately, with no separate "re-render the whole list" step.
        let mut pres = presentation(vec![slide("A", None), slide("B", None), slide("C", None)]);
        // Simulate the save: slide[0] now carries an explicit group.
        pres.slides[0].content.group = Some(SlideGroup::new("Verse 1".to_string()));

        let second_id = pres.slides[1].id.to_string();
        let display = resolve_group_display(&pres, &second_id);
        assert_eq!(display.badge_text, Some("Verse 1".to_string()));
        assert!(display.badge_inherited);
    }
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
    fn slide_has_any_warning_considers_lyrics_fields() {
        let ok = "short";
        let long = "abcdefghijklmnopqrstuvwxyz0123456";
        assert!(!slide_has_any_warning(ok, ok, 32));
        assert!(slide_has_any_warning(long, ok, 32));
        assert!(slide_has_any_warning(ok, long, 32));
    }

    #[test]
    fn slide_has_any_warning_ignores_stage_field_length_entirely() {
        // #515: a long stage field must NOT be passed to (or trip)
        // slide_has_any_warning — it is free-form speaker text, exempt from
        // the lyrics per-line character limit.
        let ok = "short";
        let over_limit_stage = "abcdefghijklmnopqrstuvwxyz0123456";
        assert!(
            field_has_warning(over_limit_stage, 32),
            "sanity: this text IS over the limit"
        );
        assert!(
            !slide_has_any_warning(ok, ok, 32),
            "a slide with only ok main/translation must not warn regardless of any stage text"
        );
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

/// Tags whose clicks/pointerdowns must NOT trigger/focus the slide card —
/// the card's own interactive controls (#515: one predicate shared by the
/// click AND pointerdown guards so a new control can't be wired into only
/// one of them).
pub fn is_interactive_tag(tag: &str) -> bool {
    matches!(tag, "button" | "textarea" | "input" | "select" | "option")
}

#[cfg(test)]
mod interactive_tag_tests {
    use super::is_interactive_tag;

    #[test]
    fn interactive_tags_are_skipped_and_plain_tags_are_not() {
        for tag in ["button", "textarea", "input", "select", "option"] {
            assert!(is_interactive_tag(tag), "{tag} must be interactive");
        }
        for tag in ["div", "span", "article", "header"] {
            assert!(!is_interactive_tag(tag), "{tag} must not be interactive");
        }
    }
}
