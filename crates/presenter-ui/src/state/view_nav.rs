//! The operator's sections (#850 spec §6.2): one label source for the
//! desktop view nav and the phone tab bar (Slovak, decided default §13-1), and
//! the one `set_view` both use.

use leptos::prelude::*;

/// (view id, label), in display order.
pub const VIEWS: [(&str, &str); 6] = [
    ("worship", "Piesne"),
    ("bible", "Biblia"),
    ("timers", "Časovače"),
    ("ai", "AI"),
    ("menovky", "Menovky"),
    ("settings", "Nastavenia"),
];

/// The URL path of a view: worship is the bare operator path.
pub fn view_path(view: &str) -> String {
    if view == "worship" {
        "/ui/operator".to_string()
    } else {
        format!("/ui/operator/{view}")
    }
}

/// The view a URL path names (browser Back / Forward); the inverse of
/// [`view_path`].
pub fn view_from_path(pathname: &str) -> String {
    pathname
        .strip_prefix("/ui/operator/")
        .map(|rest| rest.trim_end_matches('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or("worship")
        .to_string()
}

/// Switch the operator view: the signal, the session key and a history entry,
/// so Back returns to the previous section (`pages/operator.rs` popstate).
/// A tap on the section already shown adds no history entry.
pub fn set_view(view: RwSignal<String>, next: &str) {
    if view.get_untracked() == next {
        return;
    }
    view.set(next.to_string());
    crate::state::session::set("view", next);
    let state = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&state, &"view".into(), &next.into());
    let _ = crate::utils::window::window()
        .history()
        .and_then(|history| history.push_state_with_url(&state, "", Some(&view_path(next))));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_survives_the_round_trip_through_its_url() {
        for (view, _) in VIEWS {
            assert_eq!(view_from_path(&view_path(view)), view, "{view}");
        }
    }

    #[test]
    fn the_bare_operator_path_and_a_trailing_slash_mean_worship_and_bible() {
        assert_eq!(view_from_path("/ui/operator"), "worship");
        assert_eq!(view_from_path("/ui/operator/"), "worship");
        assert_eq!(view_from_path("/ui/operator/bible/"), "bible");
    }
}
