//! The sermon editor's gestures (#851 spec §7.2), pure and host-tested: what a
//! tap, a long press, the two cuts and Späť do to an item's edits. The text
//! itself always comes from the core's `render_item`, so the phone and the
//! server's slides never disagree.

use presenter_core::bible::sermon::{
    item_label, normalize_edits, render_item, token_texts, ItemEdits, SourceVerse, VerseEdits,
};

/// A session's undo steps per item.
const UNDO_STEPS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapMode {
    Strike,
    Emphasize,
}

impl TapMode {
    /// A long press applies the other mode.
    pub fn other(self) -> Self {
        match self {
            Self::Strike => Self::Emphasize,
            Self::Emphasize => Self::Strike,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    Kept,
    Struck,
    Emphasized,
}

impl TokenState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Struck => "struck",
            Self::Emphasized => "emphasized",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
    Saved,
    Saving,
    Error,
}

impl SaveState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Saving => "saving",
            Self::Error => "error",
        }
    }
}

/// One item as the editor holds it: the snapshot, the edits and their undo stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDraft {
    pub source: Vec<SourceVerse>,
    pub edits: ItemEdits,
    undo: Vec<ItemEdits>,
    last_tap: Option<(u16, u16)>,
}

fn bracket_closer(opening: char) -> Option<char> {
    match opening {
        '(' => Some(')'),
        '[' => Some(']'),
        _ => None,
    }
}

impl ItemDraft {
    pub fn new(source: Vec<SourceVerse>, edits: ItemEdits) -> Self {
        Self {
            source,
            edits,
            undo: Vec::new(),
            last_tap: None,
        }
    }

    pub fn token_state(&self, verse: u16, index: u16) -> TokenState {
        let Some(edits) = self.edits.verse(verse) else {
            return TokenState::Kept;
        };
        if edits.deleted.contains(&index) {
            TokenState::Struck
        } else if edits.emphasized.contains(&index) {
            TokenState::Emphasized
        } else {
            TokenState::Kept
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_cut(&self) -> bool {
        self.last_tap.is_some()
    }

    /// `Hebrejom 11:1a (SEB)`, computed like the server's.
    pub fn label(&self, book: &str, chapter: u16, translation: &str) -> String {
        let rendered = render_item(&self.source, &self.edits);
        item_label(book, chapter, &rendered.suffix, translation)
    }

    /// Every token struck: the item gives no slide.
    pub fn is_all_struck(&self) -> bool {
        render_item(&self.source, &self.edits).is_empty()
    }

    fn tokens(&self, verse: u16) -> Vec<&str> {
        self.source
            .iter()
            .find(|candidate| candidate.verse == verse)
            .map(|found| token_texts(&found.text))
            .unwrap_or_default()
    }

    /// The token ending what a tap on `index` starts: the closing bracket of a
    /// token opening one, in the same verse, else the token itself.
    fn tap_end(&self, verse: u16, index: u16) -> u16 {
        let tokens = self.tokens(verse);
        let closer = tokens
            .get(usize::from(index))
            .and_then(|token| token.chars().next())
            .and_then(bracket_closer);
        let Some(closer) = closer else {
            return index;
        };
        tokens
            .iter()
            .enumerate()
            .skip(usize::from(index))
            .find(|(_, token)| token.contains(closer))
            .and_then(|(at, _)| u16::try_from(at).ok())
            .unwrap_or(index)
    }

    fn commit(&mut self, next: ItemEdits) -> bool {
        let next = normalize_edits(&self.source, &next).unwrap_or(next);
        if next == self.edits {
            return false;
        }
        self.undo.push(std::mem::replace(&mut self.edits, next));
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
        true
    }

    fn strike(&mut self, verse: u16, range: std::ops::RangeInclusive<u16>, on: bool) -> bool {
        let mut next = self.edits.clone();
        let entry = next.0.entry(verse).or_insert_with(VerseEdits::default);
        for index in range {
            entry.deleted.retain(|held| *held != index);
            entry.emphasized.retain(|held| *held != index);
            if on {
                entry.deleted.push(index);
            }
        }
        self.commit(next)
    }

    /// A tap, or a long press with `mode.other()`. True when the edits changed.
    pub fn tap(&mut self, mode: TapMode, verse: u16, index: u16) -> bool {
        self.last_tap = Some((verse, index));
        let state = self.token_state(verse, index);
        match mode {
            TapMode::Strike => {
                let end = self.tap_end(verse, index);
                self.strike(verse, index..=end, state != TokenState::Struck)
            }
            TapMode::Emphasize => {
                let mut next = self.edits.clone();
                let entry = next.0.entry(verse).or_insert_with(VerseEdits::default);
                entry.deleted.retain(|held| *held != index);
                entry.emphasized.retain(|held| *held != index);
                if state != TokenState::Emphasized {
                    entry.emphasized.push(index);
                }
                self.commit(next)
            }
        }
    }

    /// Do konca verša: strike from the last tapped token to the verse's end.
    pub fn cut_to_end(&mut self) -> bool {
        let Some((verse, index)) = self.last_tap else {
            return false;
        };
        let Some(last) = self.tokens(verse).len().checked_sub(1) else {
            return false;
        };
        self.strike(verse, index..=u16::try_from(last).unwrap_or(index), true)
    }

    /// Od začiatku verša: strike from the verse's start up to the last tapped token.
    pub fn cut_from_start(&mut self) -> bool {
        let Some((verse, index)) = self.last_tap else {
            return false;
        };
        self.strike(verse, 0..=index, true)
    }

    /// Späť: the step before. True when there was one.
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(previous) => {
                self.edits = previous;
                true
            }
            None => false,
        }
    }

    /// The server's answer for this item, the undo stack kept.
    pub fn accept(&mut self, source: Vec<SourceVerse>, edits: ItemEdits) {
        if source != self.source {
            self.undo.clear();
            self.last_tap = None;
        }
        self.source = source;
        self.edits = edits;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> ItemDraft {
        ItemDraft::new(
            vec![
                SourceVerse {
                    verse: 1,
                    text: "Viera je zárukou toho, v čo dúfame, a zdôvodnením toho, čo nevidíme,"
                        .to_string(),
                },
                SourceVerse {
                    verse: 2,
                    text: "Pre ňu (a nie pre skutky) dostali svedectvo [starší]".to_string(),
                },
            ],
            ItemEdits::default(),
        )
    }

    #[test]
    fn a_tap_strikes_and_a_second_tap_restores() {
        let mut item = draft();
        assert!(item.tap(TapMode::Strike, 1, 2));
        assert_eq!(item.token_state(1, 2), TokenState::Struck);
        assert!(item.tap(TapMode::Strike, 1, 2));
        assert_eq!(item.token_state(1, 2), TokenState::Kept);
        assert_eq!(
            item.edits,
            ItemEdits::default(),
            "an untouched verse leaves"
        );
    }

    #[test]
    fn a_long_press_in_strike_mode_emphasizes() {
        let mut item = draft();
        item.tap(TapMode::Strike.other(), 1, 0);
        assert_eq!(item.token_state(1, 0), TokenState::Emphasized);
    }

    #[test]
    fn emphasizing_a_struck_word_brings_it_back_and_striking_ends_the_emphasis() {
        let mut item = draft();
        item.tap(TapMode::Strike, 1, 0);
        item.tap(TapMode::Emphasize, 1, 0);
        assert_eq!(item.token_state(1, 0), TokenState::Emphasized);
        item.tap(TapMode::Strike, 1, 0);
        assert_eq!(item.token_state(1, 0), TokenState::Struck);
        assert!(item.edits.verse(1).expect("edits").emphasized.is_empty());
    }

    #[test]
    fn a_tap_on_a_parenthesis_strikes_to_its_close_as_one_step() {
        let mut item = draft();
        item.tap(TapMode::Strike, 2, 2);
        for index in 2..=5 {
            assert_eq!(item.token_state(2, index), TokenState::Struck, "{index}");
        }
        assert_eq!(item.token_state(2, 6), TokenState::Kept);
        item.undo();
        assert_eq!(item.edits, ItemEdits::default(), "one step");
    }

    #[test]
    fn a_tap_on_a_bracket_in_one_token_strikes_only_it() {
        let mut item = draft();
        item.tap(TapMode::Strike, 2, 8);
        assert_eq!(item.token_state(2, 8), TokenState::Struck);
        assert_eq!(item.token_state(2, 7), TokenState::Kept);
    }

    #[test]
    fn an_unclosed_parenthesis_strikes_only_its_token() {
        let mut item = ItemDraft::new(
            vec![SourceVerse {
                verse: 1,
                text: "jeden (dva tri".to_string(),
            }],
            ItemEdits::default(),
        );
        item.tap(TapMode::Strike, 1, 1);
        assert_eq!(item.token_state(1, 1), TokenState::Struck);
        assert_eq!(item.token_state(1, 2), TokenState::Kept);
    }

    #[test]
    fn the_cuts_work_from_the_last_tap_and_gives_the_truthful_label() {
        let mut item = draft();
        assert!(!item.can_cut());
        assert!(!item.cut_to_end(), "nothing tapped yet");
        item.tap(TapMode::Strike, 1, 7);
        assert!(item.cut_to_end());
        assert_eq!(
            item.edits.verse(1).expect("edits").deleted,
            [7, 8, 9, 10, 11]
        );
        assert_eq!(
            ItemDraft::new(item.source[..1].to_vec(), item.edits.clone())
                .label("Hebrejom", 11, "slk-seb"),
            "Hebrejom 11:1a (SEB)"
        );
    }

    #[test]
    fn cut_from_start_strikes_up_to_the_tap() {
        let mut item = draft();
        item.tap(TapMode::Strike, 1, 2);
        item.tap(TapMode::Strike, 1, 2);
        assert!(item.cut_from_start());
        assert_eq!(item.edits.verse(1).expect("edits").deleted, [0, 1, 2]);
    }

    #[test]
    fn undo_walks_back_and_keeps_only_50_steps() {
        let mut item = draft();
        for _ in 0..60 {
            item.tap(TapMode::Strike, 1, 0);
        }
        let mut steps = 0;
        while item.undo() {
            steps += 1;
        }
        assert_eq!(steps, 50);
        assert!(!item.can_undo());
    }

    #[test]
    fn a_step_changing_nothing_is_no_undo_step() {
        let mut item = draft();
        item.tap(TapMode::Strike, 1, 7);
        item.cut_to_end();
        assert!(!item.cut_to_end());
        item.undo();
        assert_eq!(item.edits.verse(1).expect("edits").deleted, [7]);
    }

    #[test]
    fn all_struck_is_known() {
        let mut item = ItemDraft::new(
            vec![SourceVerse {
                verse: 1,
                text: "jeden dva".to_string(),
            }],
            ItemEdits::default(),
        );
        item.tap(TapMode::Strike, 1, 0);
        assert!(!item.is_all_struck());
        item.cut_to_end();
        assert!(item.is_all_struck());
    }

    #[test]
    fn a_new_snapshot_forgets_the_undo_steps_that_named_its_tokens() {
        let mut item = draft();
        item.tap(TapMode::Strike, 1, 0);
        let edits = item.edits.clone();
        item.accept(item.source.clone(), edits.clone());
        assert!(item.can_undo(), "same text");
        item.accept(
            vec![SourceVerse {
                verse: 1,
                text: "iný text".to_string(),
            }],
            ItemEdits::default(),
        );
        assert!(!item.can_undo());
        assert!(!item.can_cut());
    }
}
