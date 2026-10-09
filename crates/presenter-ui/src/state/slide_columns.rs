//! #832: the per-browser "slides per row" setting of the operator slide grids.

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
        assert_eq!(step_slide_columns(None, 1), DEFAULT_SLIDE_COLUMNS + 1);
        assert_eq!(step_slide_columns(None, -1), DEFAULT_SLIDE_COLUMNS - 1);
        assert_eq!(step_slide_columns(Some(8), 1), MAX_SLIDE_COLUMNS);
        assert_eq!(step_slide_columns(Some(1), -1), MIN_SLIDE_COLUMNS);
        assert_eq!(step_slide_columns(Some(4), 1), 5);
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
