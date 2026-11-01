//! The study page's back / forward list (#851 spec §6.2), kept next to the
//! browser's own history: each entry remembers the browser history index it
//! sits at (`history.state`), so a `popstate` finds its place and the long-press
//! list knows how far `history.go(-n)` has to go. Pure, host-tested; the page
//! keeps it in sessionStorage.

use serde::{Deserialize, Serialize};

/// The list keeps this many entries (spec §6.2).
pub const MAX_ENTRIES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub url: String,
    pub label: String,
    /// False for a sheet: it is a step for the Back gesture, not a place.
    pub listed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudyHistory {
    entries: Vec<HistoryEntry>,
    /// The browser history index of `entries[0]`.
    base: usize,
    /// The browser history index shown now.
    position: usize,
    /// Written into `history.state` with the index: an entry of another list
    /// (the tab left the page and came back) is never taken for one of this.
    #[serde(default)]
    generation: String,
}

/// `history.state` of an entry: `(generation, browser index)`.
pub fn parse_state(state: &str) -> Option<(&str, usize)> {
    let (generation, index) = state.rsplit_once(':')?;
    Some((generation, index.parse().ok()?))
}

impl StudyHistory {
    /// A saved list; anything unreadable starts a new one.
    pub fn parse(stored: Option<&str>) -> Self {
        stored
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default()
    }

    pub fn to_stored(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// A new list that starts at `entry`: nothing lies behind it.
    pub fn starting(generation: String, entry: HistoryEntry) -> Self {
        Self {
            entries: vec![entry],
            base: 0,
            position: 0,
            generation,
        }
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// What goes into `history.state` for the current entry.
    pub fn state_text(&self) -> String {
        format!("{}:{}", self.generation, self.position)
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A new place after the current one; whatever was ahead of it is gone,
    /// as in the browser. Returns the new entry's browser history index.
    pub fn push(&mut self, entry: HistoryEntry) -> usize {
        if self.position < self.base {
            // Back went past the kept list: nothing kept is behind or ahead now.
            self.entries.clear();
            self.base = self.position + 1;
            self.position = self.base;
        } else if !self.entries.is_empty() {
            self.entries.truncate(self.offset() + 1);
            self.position += 1;
        }
        self.entries.push(entry);
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
            self.base += 1;
        }
        self.position
    }

    /// The current place changed in place (a selection grew, a sheet went).
    pub fn replace(&mut self, entry: HistoryEntry) {
        let offset = self.offset();
        match self.entries.get_mut(offset) {
            Some(held) => *held = entry,
            None => self.entries.push(entry),
        }
    }

    /// The current entry's row, once the chapter's real book name is known.
    pub fn relabel(&mut self, label: String) {
        let offset = self.offset();
        if let Some(entry) = self.entries.get_mut(offset) {
            entry.label = label;
        }
    }

    /// The browser moved to this history index (`popstate`). An index before
    /// the list's start stays at its first entry.
    pub fn moved_to(&mut self, index: usize) {
        self.position = index;
    }

    fn offset(&self) -> usize {
        self.position
            .saturating_sub(self.base)
            .min(self.entries.len().saturating_sub(1))
    }

    /// ◀ works when an entry lies behind the current one.
    pub fn can_go_back(&self) -> bool {
        !self.entries.is_empty() && self.offset() > 0
    }

    /// ▶ works when a PLACE lies ahead: a closed sheet is no place to return to.
    pub fn can_go_forward(&self) -> bool {
        self.entries
            .iter()
            .skip(self.offset() + 1)
            .any(|entry| entry.listed)
    }

    /// How far `history.go` moves to reach the next place ahead, past any
    /// closed sheet.
    pub fn forward_steps(&self) -> Option<i32> {
        let ahead = self
            .entries
            .iter()
            .skip(self.offset() + 1)
            .position(|entry| entry.listed)?;
        i32::try_from(ahead + 1).ok()
    }

    /// The places behind the current one, nearest first, with the `history.go`
    /// step that returns to each. Under an open sheet the nearest place is the
    /// one the reader is on: not a place to go back to.
    pub fn back_list(&self) -> Vec<(i32, HistoryEntry)> {
        let offset = self.offset();
        let on_sheet = self.entries.get(offset).is_some_and(|entry| !entry.listed);
        self.entries
            .iter()
            .take(offset)
            .enumerate()
            .rev()
            .filter(|(_, entry)| entry.listed)
            .skip(usize::from(on_sheet))
            .filter_map(|(index, entry)| {
                let steps = i32::try_from(offset - index).ok()?;
                Some((-steps, entry.clone()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(name: &str) -> HistoryEntry {
        HistoryEntry {
            url: format!("/ui/study?p={name}"),
            label: name.to_string(),
            listed: true,
        }
    }

    fn sheet(name: &str) -> HistoryEntry {
        HistoryEntry {
            listed: false,
            ..place(name)
        }
    }

    fn walked(names: &[&str]) -> StudyHistory {
        let mut history = StudyHistory::default();
        for name in names {
            history.push(place(name));
        }
        history
    }

    #[test]
    fn the_first_entry_has_nothing_behind_or_ahead() {
        let history = walked(&["a"]);
        assert_eq!(history.position(), 0);
        assert!(!history.can_go_back() && !history.can_go_forward());
    }

    #[test]
    fn the_back_list_goes_nearest_first_with_the_step_to_each() {
        let history = walked(&["a", "b", "c"]);
        let list: Vec<(i32, String)> = history
            .back_list()
            .into_iter()
            .map(|(steps, entry)| (steps, entry.label))
            .collect();
        assert_eq!(list, [(-1, "b".to_string()), (-2, "a".to_string())]);
    }

    #[test]
    fn going_back_opens_forward_and_a_new_place_ends_it() {
        let mut history = walked(&["a", "b", "c"]);
        history.moved_to(1);
        assert!(history.can_go_back() && history.can_go_forward());
        assert_eq!(history.forward_steps(), Some(1));
        let index = history.push(place("d"));
        assert_eq!(index, 2);
        assert!(!history.can_go_forward());
        assert_eq!(history.back_list().len(), 2);
    }

    #[test]
    fn a_sheet_is_a_step_back_but_not_a_row_of_the_list() {
        let mut history = walked(&["a", "b"]);
        history.push(sheet("b+sheet"));
        history.push(place("c"));
        assert!(history.can_go_back());
        let labels: Vec<String> = history
            .back_list()
            .into_iter()
            .map(|row| row.1.label)
            .collect();
        assert_eq!(labels, ["b", "a"]);
    }

    #[test]
    fn the_list_open_over_a_place_leaves_that_place_out() {
        let mut history = walked(&["a", "b", "c"]);
        history.push(sheet("c+list"));
        let list: Vec<(i32, String)> = history
            .back_list()
            .into_iter()
            .map(|(steps, entry)| (steps, entry.label))
            .collect();
        assert_eq!(list, [(-2, "b".to_string()), (-3, "a".to_string())]);
    }

    #[test]
    fn a_closed_sheet_is_no_reason_to_light_forward() {
        let mut history = walked(&["a"]);
        history.push(sheet("a+sheet"));
        history.moved_to(0);
        assert!(!history.can_go_forward());
        assert_eq!(history.forward_steps(), None);
    }

    #[test]
    fn forward_jumps_over_a_closed_sheet_to_the_next_place() {
        let mut history = walked(&["a"]);
        history.push(sheet("a+sheet"));
        history.push(place("b"));
        history.moved_to(0);
        assert_eq!(history.forward_steps(), Some(2));
    }

    #[test]
    fn a_push_after_going_below_the_kept_list_starts_over_there() {
        let mut history = StudyHistory::default();
        for index in 0..60 {
            history.push(place(&index.to_string()));
        }
        history.moved_to(3);
        assert_eq!(history.push(place("new")), 4);
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
        assert_eq!(history.push(place("next")), 5);
        assert_eq!(history.back_list()[0].0, -1);
        assert_eq!(history.back_list()[0].1.label, "new");
    }

    #[test]
    fn a_new_list_has_nothing_behind_it_and_names_its_generation() {
        let mut history = StudyHistory::starting("g1".to_string(), place("a"));
        assert!(!history.can_go_back());
        assert_eq!(history.state_text(), "g1:0");
        history.push(place("b"));
        assert_eq!(history.state_text(), "g1:1");
        assert_eq!(parse_state("g1:1"), Some(("g1", 1)));
        assert_eq!(parse_state("nothing"), None);
        assert_eq!(parse_state("g1:x"), None);
    }

    #[test]
    fn relabel_keeps_a_closed_sheet_unlisted() {
        let mut history = walked(&["a"]);
        history.push(sheet("b"));
        history.relabel("Hebrejom 11 · SEB".to_string());
        history.push(place("c"));
        assert_eq!(history.back_list().len(), 1);
    }

    #[test]
    fn replace_changes_the_current_entry_only() {
        let mut history = walked(&["a", "b"]);
        history.moved_to(0);
        history.replace(place("a2"));
        history.moved_to(1);
        let list = history.back_list();
        assert_eq!(list[0].1.label, "a2");
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn replace_on_a_new_list_starts_it() {
        let mut history = StudyHistory::default();
        history.replace(place("a"));
        assert!(!history.is_empty());
        assert_eq!(history.position(), 0);
    }

    #[test]
    fn the_list_keeps_50_and_the_browser_index_keeps_counting() {
        let mut history = StudyHistory::default();
        for index in 0..60 {
            history.push(place(&index.to_string()));
        }
        assert_eq!(history.position(), 59);
        let list = history.back_list();
        assert_eq!(list.len(), MAX_ENTRIES - 1);
        assert_eq!(list.last().expect("the oldest").1.label, "10");
        assert_eq!(list.last().expect("the oldest").0, -49);
    }

    #[test]
    fn a_step_before_the_kept_list_stays_at_its_start() {
        let mut history = StudyHistory::default();
        for index in 0..60 {
            history.push(place(&index.to_string()));
        }
        history.moved_to(3);
        assert!(!history.can_go_back());
        assert!(history.can_go_forward());
    }

    #[test]
    fn the_list_survives_a_reload_through_storage() {
        let mut history = walked(&["a", "b"]);
        history.moved_to(0);
        assert_eq!(StudyHistory::parse(Some(&history.to_stored())), history);
        assert_eq!(
            StudyHistory::parse(Some("not json")),
            StudyHistory::default()
        );
        assert_eq!(StudyHistory::parse(None), StudyHistory::default());
    }
}
