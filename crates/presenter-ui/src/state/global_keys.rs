//! The operator's window-wide keys, scoped to the shown section (#850 B2d,
//! spec section 7.3). Before this, ←/→ moved the song's slides on every
//! section in Live mode, so an arrow pressed on Biblia changed the song on the
//! wall. Now:
//!
//! - **Piesne**, Live mode: ←/→ = the previous / next song slide, Space = the
//!   header search.
//! - **Biblia**, Live tab (either mode): ←/→ = the console's ◀ / ▶ (they move
//!   the preview only, nothing goes to the wall), Space or `/` = the console
//!   field.
//! - Any other section, Live mode: Space = the header search; arrows do
//!   nothing.
//!
//! Never taken (the browser keeps the key): any key while a modal or the
//! "Viac" sheet is open; Space, `/` and Enter while a button or link has the
//! focus, so Space presses Zhasnúť, ◀ / ▶, a translation button or a modal
//! button like any button (review of 65eab96c). Arrows still step from a
//! focused button: after the desktop Enter the focus sits on Zobraziť.
//! The caller already returned for a key typed into a field.

use crate::utils::keyboard::{KEY_ARROW_LEFT, KEY_ARROW_RIGHT};

/// What a window keydown outside a field does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalKey {
    /// Send the previous (`false`) / next (`true`) song slide to the wall.
    SongSlide { forward: bool },
    /// Focus the header search.
    FocusSearch,
    /// Press the console's ◀ (`false`) / ▶ (`true`): the preview only.
    ConsoleStep { forward: bool },
    /// Focus the console field.
    FocusConsole,
    /// Leave the key to the browser.
    Nothing,
}

/// What has the keyboard focus when a window key arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFocus {
    /// The page itself, or an element that is no control.
    Page,
    /// A button, a link or an element with `role="button"`.
    Button,
}

/// The shown section and mode a key lands in.
#[derive(Debug, Clone, Copy)]
pub struct KeyScope<'a> {
    /// `AppContext::view` (`worship`, `bible`, `timers`, ...).
    pub view: &'a str,
    /// Live mode (not Edit).
    pub live: bool,
    /// Biblia's Live tab is shown (not Pripravené / Nastavenie).
    pub bible_live_tab: bool,
    pub focus: KeyFocus,
    /// A modal or the "Viac" sheet is open over the page.
    pub overlay_open: bool,
}

pub fn global_key(scope: KeyScope<'_>, key: &str) -> GlobalKey {
    if scope.overlay_open {
        return GlobalKey::Nothing;
    }
    if scope.focus == KeyFocus::Button && matches!(key, " " | "/" | "Enter") {
        return GlobalKey::Nothing;
    }
    let arrow = match key {
        KEY_ARROW_LEFT => Some(false),
        KEY_ARROW_RIGHT => Some(true),
        _ => None,
    };
    match scope.view {
        "bible" if scope.bible_live_tab => match (arrow, key) {
            (Some(forward), _) => GlobalKey::ConsoleStep { forward },
            (None, " " | "/") => GlobalKey::FocusConsole,
            _ => GlobalKey::Nothing,
        },
        "bible" => GlobalKey::Nothing,
        "worship" if scope.live => match (arrow, key) {
            (Some(forward), _) => GlobalKey::SongSlide { forward },
            (None, " ") => GlobalKey::FocusSearch,
            _ => GlobalKey::Nothing,
        },
        _ if scope.live && key == " " => GlobalKey::FocusSearch,
        _ => GlobalKey::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(view: &str, live: bool, bible_live_tab: bool) -> KeyScope<'_> {
        KeyScope {
            view,
            live,
            bible_live_tab,
            focus: KeyFocus::Page,
            overlay_open: false,
        }
    }

    #[test]
    fn space_and_slash_on_a_focused_button_stay_with_the_button() {
        for (view, live, tab) in [
            ("bible", true, true),
            ("bible", false, true),
            ("worship", true, false),
            ("timers", true, false),
        ] {
            let s = KeyScope {
                focus: KeyFocus::Button,
                ..scope(view, live, tab)
            };
            for key in [" ", "/", "Enter"] {
                assert_eq!(global_key(s, key), GlobalKey::Nothing, "{view} {key:?}");
            }
        }
    }

    #[test]
    fn arrows_from_a_focused_button_still_step() {
        let bible = KeyScope {
            focus: KeyFocus::Button,
            ..scope("bible", true, true)
        };
        assert_eq!(
            global_key(bible, "ArrowRight"),
            GlobalKey::ConsoleStep { forward: true }
        );
        let worship = KeyScope {
            focus: KeyFocus::Button,
            ..scope("worship", true, false)
        };
        assert_eq!(
            global_key(worship, "ArrowLeft"),
            GlobalKey::SongSlide { forward: false }
        );
    }

    #[test]
    fn an_open_modal_or_sheet_takes_no_key() {
        for (view, live, tab) in [
            ("bible", true, true),
            ("worship", true, false),
            ("timers", true, false),
        ] {
            let s = KeyScope {
                overlay_open: true,
                ..scope(view, live, tab)
            };
            for key in ["ArrowLeft", "ArrowRight", " ", "/"] {
                assert_eq!(global_key(s, key), GlobalKey::Nothing, "{view} {key:?}");
            }
        }
    }

    #[test]
    fn arrows_on_piesne_in_live_mode_move_the_song_slides() {
        let s = scope("worship", true, false);
        assert_eq!(
            global_key(s, "ArrowRight"),
            GlobalKey::SongSlide { forward: true }
        );
        assert_eq!(
            global_key(s, "ArrowLeft"),
            GlobalKey::SongSlide { forward: false }
        );
        assert_eq!(global_key(s, " "), GlobalKey::FocusSearch);
        assert_eq!(global_key(s, "/"), GlobalKey::Nothing);
    }

    #[test]
    fn arrows_on_biblia_step_the_console_and_never_the_song() {
        for live in [true, false] {
            let s = scope("bible", live, true);
            assert_eq!(
                global_key(s, "ArrowRight"),
                GlobalKey::ConsoleStep { forward: true }
            );
            assert_eq!(
                global_key(s, "ArrowLeft"),
                GlobalKey::ConsoleStep { forward: false }
            );
            assert_eq!(global_key(s, " "), GlobalKey::FocusConsole);
            assert_eq!(global_key(s, "/"), GlobalKey::FocusConsole);
            assert_eq!(global_key(s, "a"), GlobalKey::Nothing);
        }
    }

    #[test]
    fn biblia_tabs_without_the_console_take_no_key() {
        let s = scope("bible", true, false);
        for key in ["ArrowLeft", "ArrowRight", " ", "/"] {
            assert_eq!(global_key(s, key), GlobalKey::Nothing, "{key}");
        }
    }

    #[test]
    fn edit_mode_on_piesne_takes_no_key() {
        let s = scope("worship", false, false);
        for key in ["ArrowLeft", "ArrowRight", " "] {
            assert_eq!(global_key(s, key), GlobalKey::Nothing, "{key}");
        }
    }

    #[test]
    fn other_sections_keep_space_for_the_search_and_ignore_arrows() {
        for view in ["timers", "ai", "settings", "menovky"] {
            let s = scope(view, true, false);
            assert_eq!(global_key(s, " "), GlobalKey::FocusSearch, "{view}");
            assert_eq!(global_key(s, "ArrowRight"), GlobalKey::Nothing, "{view}");
            assert_eq!(global_key(s, "ArrowLeft"), GlobalKey::Nothing, "{view}");
            assert_eq!(
                global_key(scope(view, false, false), " "),
                GlobalKey::Nothing
            );
        }
    }
}
