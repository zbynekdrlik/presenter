//! Pure song-name derivations and sidebar auto-scroll for the worship-pp
//! stage layout.
//!
//! Extracted from `worship_pp.rs` (#461) so the playlist-sourced song-name
//! derivations are unit-testable on the host, and to keep `worship_pp.rs`
//! readable. The current/next badges on the worship-pp stage are sourced from
//! the Presenter PLAYLIST (the user's intent), NOT from AbleSet's server-side
//! `song_name`/`next_song_name`.

use presenter_core::StagePlaylistEntry;

use crate::utils::text::clean_song_name;

/// Current-song name sourced from the Presenter PLAYLIST.
///
/// Returns the cleaned name of the `is_active` entry, or `""` when no entry is
/// active. This is the worship-pp current-song badge source: the badge must
/// reflect the playlist's active entry rather than the AbleSet-first
/// server-side `song_name` (#461).
pub fn current_song_from_entries(entries: &[StagePlaylistEntry]) -> String {
    entries
        .iter()
        .find(|e| e.is_active)
        .map(|e| clean_song_name(&e.name))
        .unwrap_or_default()
}

/// Next-song name sourced from the Presenter PLAYLIST: the entry AFTER the
/// active one. Returns `""` when nothing is active or the active entry is last.
///
/// Mirrors worship-pp's established next-song behavior (formerly an inline
/// closure in `worship_pp.rs`, added in b503ed06).
pub fn next_song_from_entries(entries: &[StagePlaylistEntry]) -> String {
    let mut iter = entries.iter().skip_while(|e| !e.is_active);
    iter.next(); // consume the active entry itself
    iter.next()
        .map(|e| clean_song_name(&e.name))
        .unwrap_or_default()
}

/// Which sidebar row INDEX should be highlighted as the active song.
///
/// #496: a worship set may repeat the same song (a reprise), so two rows share
/// both `name` AND `presentation_id`. The server now disambiguates by marking a
/// single occurrence and reporting its index via `snapshot_active_index`; this
/// resolver prefers that explicit index so the correct OCCURRENCE highlights,
/// falling back to the first `is_active` entry only for legacy/non-playlist
/// snapshots that carry no explicit index. Returns `None` when nothing is
/// active.
pub fn active_sidebar_index(
    entries: &[StagePlaylistEntry],
    snapshot_active_index: Option<u32>,
) -> Option<usize> {
    if let Some(index) = snapshot_active_index {
        let index = index as usize;
        if index < entries.len() {
            return Some(index);
        }
    }
    entries.iter().position(|e| e.is_active)
}

/// CSS selector for the worship-pp playlist sidebar's active entry row.
const ACTIVE_ENTRY_SELECTOR: &str = ".stage-pp__playlist-sidebar .stage-pp__playlist-entry--active";

/// Scroll the worship-pp playlist sidebar so the ACTIVE song row is visible.
///
/// Keeps the currently-triggered song on screen as the service advances past
/// the ~10 rows that fit at 1080p (#461). Centers the active row within the
/// `overflow-y:auto` sidebar — the sidebar BOX size/position is unchanged, only
/// its scroll position moves (CLAUDE.md box-dimensions rule: content scroll is
/// allowed, layout boxes are not). No-op when no active row is rendered yet.
///
/// Adapts the operator slide-list scroll pattern (`slide_list_scroll.rs`) to
/// this component. It queries the DOM rather than holding a `NodeRef` because
/// Leptos's keyed `<For>` reuses row DOM, so the "active row" is whichever row
/// currently carries the `--active` class, not a fixed node.
pub fn scroll_active_entry_into_view() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(Some(active_el)) = document.query_selector(ACTIVE_ENTRY_SELECTOR) else {
        return;
    };
    let opts = web_sys::ScrollIntoViewOptions::new();
    opts.set_block(web_sys::ScrollLogicalPosition::Center);
    active_el.scroll_into_view_with_scroll_into_view_options(&opts);
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::StagePlaylistEntry;

    fn entry(name: &str, is_active: bool) -> StagePlaylistEntry {
        StagePlaylistEntry {
            name: name.to_string(),
            presentation_id: None,
            is_active,
            entry_type: "presentation".to_string(),
        }
    }

    #[test]
    fn current_song_returns_active_entry_name() {
        let entries = vec![
            entry("First", false),
            entry("Second", true),
            entry("Third", false),
        ];
        assert_eq!(current_song_from_entries(&entries), "Second");
    }

    #[test]
    fn current_song_cleans_three_digit_prefix() {
        let entries = vec![entry("042 Amazing Grace", true)];
        assert_eq!(current_song_from_entries(&entries), "Amazing Grace");
    }

    #[test]
    fn current_song_empty_when_no_active() {
        let entries = vec![entry("First", false), entry("Second", false)];
        assert_eq!(current_song_from_entries(&entries), "");
    }

    #[test]
    fn current_song_empty_for_empty_list() {
        assert_eq!(current_song_from_entries(&[]), "");
    }

    #[test]
    fn next_song_returns_entry_after_active() {
        let entries = vec![
            entry("First", false),
            entry("Second", true),
            entry("Third", false),
        ];
        assert_eq!(next_song_from_entries(&entries), "Third");
    }

    #[test]
    fn next_song_cleans_three_digit_prefix() {
        let entries = vec![entry("Active", true), entry("042 Next One", false)];
        assert_eq!(next_song_from_entries(&entries), "Next One");
    }

    #[test]
    fn next_song_empty_when_active_is_last() {
        let entries = vec![entry("First", false), entry("Last", true)];
        assert_eq!(next_song_from_entries(&entries), "");
    }

    #[test]
    fn next_song_empty_when_no_active() {
        let entries = vec![entry("First", false), entry("Second", false)];
        assert_eq!(next_song_from_entries(&entries), "");
    }

    #[test]
    fn next_song_empty_for_empty_list() {
        assert_eq!(next_song_from_entries(&[]), "");
    }

    /// #496: when a set repeats a song, the explicit per-occurrence index from
    /// the snapshot must win over "first is_active row". Simulates the ambiguous
    /// case (both occurrences marked active, as the pre-fix server produced) and
    /// asserts the resolver targets the triggered occurrence (index 2).
    #[test]
    fn active_sidebar_index_prefers_explicit_occurrence_for_repeated_song() {
        let entries = vec![
            entry("Reprise", true), // index 0 — first occurrence
            entry("Other", false),  // index 1
            entry("Reprise", true), // index 2 — the triggered reprise
        ];
        assert_eq!(active_sidebar_index(&entries, Some(2)), Some(2));
    }

    #[test]
    fn active_sidebar_index_falls_back_to_first_active_without_explicit_index() {
        let entries = vec![entry("A", false), entry("B", true), entry("C", false)];
        assert_eq!(active_sidebar_index(&entries, None), Some(1));
    }

    #[test]
    fn active_sidebar_index_none_when_nothing_active() {
        let entries = vec![entry("A", false), entry("B", false)];
        assert_eq!(active_sidebar_index(&entries, None), None);
    }
}
