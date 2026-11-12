//! The verse sheet's selected range (#936, owner 2026-11-12: "nejasné ako sa
//! vyznačuje viacero veršov … mal by som vidieť v pridať menu ako selectujem
//! viacej veršov"): its size in Slovak and the end stepped by − / +.

use presenter_core::bible::sermon::MAX_ITEM_VERSES;

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

/// The last verse `+` may reach from `start`: the chapter's end, and no more
/// than a sermon item holds (`MAX_ITEM_VERSES`, else the add is refused).
pub fn range_limit(start: u16, last: u16) -> u16 {
    let most = start.saturating_add(MAX_ITEM_VERSES.saturating_sub(1));
    last.min(most).max(start)
}

/// The range with its END moved by `delta`, inside `start..=range_limit`;
/// `None` when that changes nothing. `+` never shrinks a range that already
/// runs past the limit (a link's `v=1-40` in a shorter chapter).
pub fn step_range_end((start, end): (u16, u16), delta: i32, last: u16) -> Option<(u16, u16)> {
    let limit = range_limit(start, last);
    if delta > 0 && end >= limit {
        return None;
    }
    let moved = i32::from(end).saturating_add(delta);
    let bounded = moved.clamp(i32::from(start), i32::from(limit));
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

    #[test]
    fn plus_stops_at_a_sermon_items_sixty_verses_and_never_shrinks() {
        // Psalm 119: 176 verses, an item holds 60.
        assert_eq!(range_limit(1, 176), 60);
        assert_eq!(range_limit(150, 176), 176);
        assert_eq!(step_range_end((1, 59), 1, 176), Some((1, 60)));
        assert_eq!(step_range_end((1, 60), 1, 176), None);
        // A link's 1-40 in a 22-verse chapter: + refuses, − comes back inside.
        assert_eq!(step_range_end((1, 40), 1, 22), None);
        assert_eq!(step_range_end((1, 40), -1, 22), Some((1, 22)));
    }
}
