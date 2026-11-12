//! The verse sheet's selected range (#936, owner 2026-11-12: "nejasné ako sa
//! vyznačuje viacero veršov … mal by som vidieť v pridať menu ako selectujem
//! viacej veršov"): its size in Slovak and the end stepped by − / +.

/// "1 verš", "3 verše", "11 veršov".
pub fn verse_count_text(count: u16) -> String {
    let word = match count {
        1 => "verš",
        2..=4 => "verše",
        _ => "veršov",
    };
    format!("{count} {word}")
}

/// The verses of `(start, end)`.
pub fn range_len((start, end): (u16, u16)) -> u16 {
    end.saturating_sub(start).saturating_add(1)
}

/// The range with its END moved by `delta`, kept inside `start..=last`;
/// `None` when that changes nothing.
pub fn step_range_end((start, end): (u16, u16), delta: i32, last: u16) -> Option<(u16, u16)> {
    let moved = i32::from(end).saturating_add(delta);
    let bounded = moved.clamp(i32::from(start), i32::from(last.max(start)));
    let bounded = u16::try_from(bounded).ok()?;
    (bounded != end).then_some((start, bounded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_follow_slovak_plurals() {
        assert_eq!(verse_count_text(1), "1 verš");
        assert_eq!(verse_count_text(3), "3 verše");
        assert_eq!(verse_count_text(11), "11 veršov");
    }

    #[test]
    fn a_range_counts_both_ends() {
        assert_eq!(range_len((4, 4)), 1);
        assert_eq!(range_len((1, 11)), 11);
    }

    #[test]
    fn the_end_steps_inside_the_chapter() {
        assert_eq!(step_range_end((1, 1), 1, 22), Some((1, 2)));
        assert_eq!(step_range_end((1, 5), -1, 22), Some((1, 4)));
        assert_eq!(step_range_end((3, 3), -1, 22), None);
        assert_eq!(step_range_end((1, 22), 1, 22), None);
        assert_eq!(step_range_end((1, 21), 5, 22), Some((1, 22)));
    }
}
