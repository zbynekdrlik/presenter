//! Stream Deck button numbers of the plate list (#871, the #870 grid).
//!
//! Companion's nameplate grid starts with the song plate (button 1). Person
//! slot n, the n-th plate in the order the server resolves (position, then id,
//! the same order `GET …/nameplates` lists), is button n + 1. The grid is 4 × 3,
//! so a person from button 13 on has no Stream Deck button. Pure, host-tested.

/// The song plate's Stream Deck button: the grid starts with it (#870).
pub const SONG_BUTTON: usize = 1;

/// Buttons in the Companion grid (4 columns × 3 rows, #870).
pub const GRID_BUTTONS: usize = 12;

/// The Stream Deck button of the person plate at `index` (0-based) in list order.
pub fn person_button(index: usize) -> usize {
    index + SONG_BUTTON + 1
}

/// Whether `button` lies outside the 4 × 3 grid (no Stream Deck button).
pub fn beyond_grid(button: usize) -> bool {
    button > GRID_BUTTONS
}

/// Whether a list of `persons` plates has at least one outside the grid.
pub fn any_beyond_grid(persons: usize) -> bool {
    persons
        .checked_sub(1)
        .is_some_and(|last| beyond_grid(person_button(last)))
}

/// The tooltip of a button number.
pub fn button_title(button: usize) -> String {
    if beyond_grid(button) {
        format!("Tlačidlo {button}: mimo mriežky 4 × 3 na Stream Decku")
    } else {
        format!("Tlačidlo {button} na Stream Decku")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_song_is_button_one_and_persons_follow_in_list_order() {
        assert_eq!(SONG_BUTTON, 1);
        assert_eq!(person_button(0), 2);
        assert_eq!(person_button(1), 3);
        assert_eq!(person_button(10), 12);
    }

    #[test]
    fn the_grid_ends_at_button_twelve() {
        assert!(!beyond_grid(SONG_BUTTON));
        assert!(!beyond_grid(person_button(10)));
        assert!(beyond_grid(person_button(11)));
        assert!(beyond_grid(13));
    }

    #[test]
    fn eleven_persons_fit_the_grid_and_twelve_do_not() {
        assert!(!any_beyond_grid(0));
        assert!(!any_beyond_grid(1));
        assert!(!any_beyond_grid(11));
        assert!(any_beyond_grid(12));
        assert!(any_beyond_grid(20));
    }

    #[test]
    fn the_title_says_when_a_button_is_outside_the_grid() {
        assert_eq!(button_title(2), "Tlačidlo 2 na Stream Decku");
        assert_eq!(button_title(12), "Tlačidlo 12 na Stream Decku");
        assert_eq!(
            button_title(13),
            "Tlačidlo 13: mimo mriežky 4 × 3 na Stream Decku"
        );
    }
}
