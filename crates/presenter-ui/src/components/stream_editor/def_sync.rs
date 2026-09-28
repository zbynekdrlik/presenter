//! Keeping the editor's local `def` consistent with the operator's own writes
//! (#787 reopen). Pure + host-tested; `mod.rs` wires it into `reload_def` and
//! `save_props`.
//!
//! Two rules:
//! - A def response never replaces a def with a NEWER `config_revision` for
//!   the same output (a refetch started before a write can land after the
//!   write's own refetch).
//! - A successful element PATCH is applied to the local def at once, so the
//!   draft stops reading as "dirty" the moment the save succeeded — not only
//!   after the follow-up refetch lands (a click in between would otherwise
//!   hit the unsaved-changes question about edits that are already saved).

use presenter_core::{StreamElementDef, StreamOutputDef};

/// Whether `incoming` may replace `current`. `None` (nothing loaded, or just
/// cleared by an output switch) always accepts; a different output (slug)
/// always accepts; the same output accepts only a revision that is not older.
pub fn should_install(current: Option<&StreamOutputDef>, incoming: &StreamOutputDef) -> bool {
    match current {
        None => true,
        Some(cur) if cur.slug != incoming.slug => true,
        Some(cur) => incoming.config_revision >= cur.config_revision,
    }
}

/// Replace the element with `saved.id` in `def` by the server's saved copy.
/// Returns whether the element was found.
pub fn apply_saved_element(def: &mut StreamOutputDef, saved: &StreamElementDef) -> bool {
    for scene in &mut def.scenes {
        if let Some(el) = scene.elements.iter_mut().find(|e| e.id == saved.id) {
            *el = saved.clone();
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::stream_editor::props_access::{default_element_props, with_frame_mut};
    use presenter_core::{SceneKind, StreamSceneDef};

    fn element(id: i64) -> StreamElementDef {
        StreamElementDef {
            id,
            z_order: 0,
            props: default_element_props("color"),
        }
    }

    fn def(slug: &str, revision: u64) -> StreamOutputDef {
        StreamOutputDef {
            id: 1,
            slug: slug.to_string(),
            name: slug.to_string(),
            default_transition_ms: 0,
            base_transition_ms: None,
            overlay_transition_ms: None,
            active_scene_id: None,
            config_revision: revision,
            scenes: vec![StreamSceneDef {
                id: 10,
                name: "S".to_string(),
                kind: SceneKind::Base,
                position: 0,
                is_active: false,
                transition_ms: None,
                elements: vec![element(100), element(101)],
            }],
        }
    }

    #[test]
    fn installs_into_an_empty_editor() {
        assert!(should_install(None, &def("stream", 0)));
    }

    #[test]
    fn an_older_revision_never_replaces_a_newer_def() {
        let cur = def("stream", 7);
        assert!(!should_install(Some(&cur), &def("stream", 6)));
    }

    #[test]
    fn same_or_newer_revision_installs() {
        let cur = def("stream", 7);
        assert!(should_install(Some(&cur), &def("stream", 7)));
        assert!(should_install(Some(&cur), &def("stream", 8)));
    }

    #[test]
    fn another_output_installs_regardless_of_revision() {
        let cur = def("stream", 7);
        assert!(should_install(Some(&cur), &def("timer", 1)));
    }

    #[test]
    fn a_saved_element_replaces_only_its_own_row() {
        let mut d = def("stream", 3);
        let mut saved = element(101);
        with_frame_mut(&mut saved.props, |f| f.x_pct = 60.0);
        assert!(apply_saved_element(&mut d, &saved));
        assert_eq!(d.scenes[0].elements[1], saved);
        assert_eq!(d.scenes[0].elements[0], element(100));
        assert_eq!(d.config_revision, 3);
    }

    #[test]
    fn an_unknown_element_leaves_the_def_unchanged() {
        let mut d = def("stream", 3);
        let before = d.clone();
        assert!(!apply_saved_element(&mut d, &element(999)));
        assert_eq!(d, before);
    }
}
