//! #832: the per-browser "slides per row" setting of the operator slide grids.
//!
//! The choice lives in localStorage (each operator's browser keeps its own,
//! nothing goes to the server) and drives every `.operator__slides` grid
//! through the inherited `--operator-slide-columns-choice` custom property
//! on `body` (`pages/operator.rs`). Without a choice the CSS default applies:
//! 3 per row, 2 on a phone (≤480 px).

/// Fewest slides per row the stepper allows.
pub const MIN_SLIDE_COLUMNS: u8 = 1;
/// Most slides per row the stepper allows.
pub const MAX_SLIDE_COLUMNS: u8 = 8;
/// Slides per row without a choice on a desktop (the CSS default).
pub const DEFAULT_SLIDE_COLUMNS: u8 = 3;
/// Slides per row without a choice on a phone (the ≤480 px CSS default).
pub const PHONE_SLIDE_COLUMNS: u8 = 2;
/// Widest viewport (CSS px) that counts as a phone — the `max-width: 480px`
/// query in `styles/operator.css`.
pub const PHONE_MAX_WIDTH: f64 = 480.0;
/// localStorage key of the choice (behind `session`'s prefix).
pub const SLIDE_COLUMNS_KEY: &str = "operatorSlideColumns";

/// The stored choice, if it is a number of slides per row in 1–8.
pub fn parse_slide_columns(stored: Option<&str>) -> Option<u8> {
    stored?
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|columns| (MIN_SLIDE_COLUMNS..=MAX_SLIDE_COLUMNS).contains(columns))
}

/// Slides per row WITHOUT a choice at this viewport width — what the grid
/// shows then: 2 on a phone, 3 otherwise.
pub fn default_slide_columns(viewport_width: f64) -> u8 {
    if viewport_width <= PHONE_MAX_WIDTH {
        PHONE_SLIDE_COLUMNS
    } else {
        DEFAULT_SLIDE_COLUMNS
    }
}

/// One stepper click from the current choice — or, without one, from the
/// `default` the grid shows — kept in 1–8.
pub fn step_slide_columns(current: Option<u8>, default: u8, delta: i8) -> u8 {
    let from = i16::from(current.unwrap_or(default));
    let next =
        (from + i16::from(delta)).clamp(i16::from(MIN_SLIDE_COLUMNS), i16::from(MAX_SLIDE_COLUMNS));
    u8::try_from(next).unwrap_or(DEFAULT_SLIDE_COLUMNS)
}

/// Six or more per row: the cards switch to the dense layout (smaller type,
/// tighter padding) so they stay readable.
pub fn is_dense(columns: u8) -> bool {
    columns >= 6
}

/// The column count of a laid-out grid from its computed
/// `grid-template-columns` ("120px 120px 120px" → 3); `None` when there are
/// no tracks.
pub fn track_count(computed_tracks: &str) -> Option<usize> {
    let tracks = computed_tracks.trim();
    if tracks.is_empty() || tracks == "none" {
        return None;
    }
    Some(tracks.split_whitespace().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_value_in_range_is_the_choice() {
        assert_eq!(parse_slide_columns(Some("5")), Some(5));
        assert_eq!(parse_slide_columns(Some(" 8 ")), Some(8));
        assert_eq!(parse_slide_columns(Some("1")), Some(1));
    }

    #[test]
    fn a_missing_or_invalid_stored_value_is_no_choice() {
        assert_eq!(parse_slide_columns(None), None);
        assert_eq!(parse_slide_columns(Some("")), None);
        assert_eq!(parse_slide_columns(Some("0")), None);
        assert_eq!(parse_slide_columns(Some("9")), None);
        assert_eq!(parse_slide_columns(Some("wide")), None);
    }

    #[test]
    fn stepping_starts_from_the_default_and_stays_within_one_to_eight() {
        let default = DEFAULT_SLIDE_COLUMNS;
        assert_eq!(
            step_slide_columns(None, default, 1),
            DEFAULT_SLIDE_COLUMNS + 1
        );
        assert_eq!(
            step_slide_columns(None, default, -1),
            DEFAULT_SLIDE_COLUMNS - 1
        );
        assert_eq!(step_slide_columns(Some(8), default, 1), MAX_SLIDE_COLUMNS);
        assert_eq!(step_slide_columns(Some(1), default, -1), MIN_SLIDE_COLUMNS);
        assert_eq!(step_slide_columns(Some(4), default, 1), 5);
    }

    #[test]
    fn without_a_choice_a_phone_shows_two_per_row_and_a_desktop_three() {
        assert_eq!(default_slide_columns(400.0), 2);
        assert_eq!(default_slide_columns(480.0), 2);
        assert_eq!(default_slide_columns(481.0), 3);
        assert_eq!(default_slide_columns(1280.0), 3);
    }

    #[test]
    fn stepping_without_a_choice_starts_from_what_the_phone_shows() {
        // The phone grid shows 2: "+" makes it 3 (not 4), "−" makes it 1.
        assert_eq!(step_slide_columns(None, default_slide_columns(400.0), 1), 3);
        assert_eq!(
            step_slide_columns(None, default_slide_columns(400.0), -1),
            1
        );
    }

    #[test]
    fn six_or_more_columns_use_the_dense_card_layout() {
        assert!(!is_dense(5));
        assert!(is_dense(6));
        assert!(is_dense(8));
    }

    #[test]
    fn the_grid_columns_are_the_computed_track_count() {
        assert_eq!(track_count("120px 120px 120px"), Some(3));
        assert_eq!(track_count(" 80.5px  80.5px "), Some(2));
        assert_eq!(track_count(""), None);
        assert_eq!(track_count("none"), None);
    }
}
