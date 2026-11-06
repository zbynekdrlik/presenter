use leptos::prelude::*;
use presenter_core::BibleTranslation;
use std::collections::HashSet;

use crate::api::bible::{BibleBookDto, BiblePresentationSummary, BibleSlideDto};

/// Bible page specific state.
#[derive(Clone, Copy)]
pub struct BibleState {
    // -- Translations --
    pub translations: RwSignal<Vec<BibleTranslation>>,
    pub selected_translation: RwSignal<Option<String>>,
    pub secondary_translation: RwSignal<Option<String>>,
    /// The translation code whose books are currently loaded into `books`.
    /// Set by the books effect as its LAST write, once the async
    /// `list_books` round-trip has resolved, so observers (and E2E) can await
    /// switch completion instead of racing the re-render
    /// (`[data-role=bible-console]` `data-books-translation`). `None` until
    /// the first book list loads.
    pub books_translation: RwSignal<Option<String>>,

    /// The main translation's books: the console's versification.
    pub books: RwSignal<Vec<BibleBookDto>>,

    // -- Slides: the console's current target only (#850 B2c) --
    pub slides: RwSignal<Vec<BibleSlideDto>>,
    pub selected_slide_ids: RwSignal<HashSet<String>>,

    // -- Tabs --
    pub bible_tab: RwSignal<String>,

    // -- Presentations --
    pub presentations: RwSignal<Vec<BiblePresentationSummary>>,
    /// The archive under the active list (#874), most recently archived first.
    pub archived_presentations: RwSignal<Vec<BiblePresentationSummary>>,
    /// A presentation card being dragged to a new place (#874).
    pub presentation_drag: RwSignal<Option<PresentationDrag>>,
    /// Bumped by every list request and every local order change: a list
    /// answer applies only while its number is the newest (#874).
    pub presentation_list_gen: RwSignal<u64>,
    /// A reorder PUT is on its way; list answers wait until the queue drains.
    pub presentation_order_sending: RwSignal<bool>,
    /// The newest order chosen while a PUT was on its way (sent next).
    pub presentation_order_waiting: RwSignal<Option<Vec<String>>>,
    pub active_presentation_id: RwSignal<Option<String>>,
    pub active_presentation_slides: RwSignal<Vec<BibleSlideDto>>,

    // -- Preferences --
    pub character_limit: RwSignal<u32>,

    // -- Drag state for prepared slide reorder --
    pub drag_source_idx: RwSignal<Option<usize>>,
    pub drag_over_idx: RwSignal<Option<usize>>,

    // -- Presentation edit modal --
    pub modal_presentation_id: RwSignal<Option<String>>,
    pub modal_presentation_name: RwSignal<String>,
}

/// One presentation-card drag in progress (#874): the dragged card and the
/// slot it would drop into now (`order::drop_slot`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentationDrag {
    pub id: String,
    pub slot: usize,
}

impl BibleState {
    pub fn new() -> Self {
        Self {
            translations: RwSignal::new(Vec::new()),
            selected_translation: RwSignal::new(None),
            secondary_translation: RwSignal::new(None),
            books_translation: RwSignal::new(None),

            books: RwSignal::new(Vec::new()),

            slides: RwSignal::new(Vec::new()),
            selected_slide_ids: RwSignal::new(HashSet::new()),

            bible_tab: RwSignal::new("live".to_string()),

            presentations: RwSignal::new(Vec::new()),
            archived_presentations: RwSignal::new(Vec::new()),
            presentation_drag: RwSignal::new(None),
            presentation_list_gen: RwSignal::new(0),
            presentation_order_sending: RwSignal::new(false),
            presentation_order_waiting: RwSignal::new(None),
            active_presentation_id: RwSignal::new(None),
            active_presentation_slides: RwSignal::new(Vec::new()),

            character_limit: RwSignal::new(320),
            drag_source_idx: RwSignal::new(None),
            drag_over_idx: RwSignal::new(None),
            modal_presentation_id: RwSignal::new(None),
            modal_presentation_name: RwSignal::new(String::new()),
        }
    }
}

impl Default for BibleState {
    fn default() -> Self {
        Self::new()
    }
}

/// #828: a slide text's character count (never bytes — a Slovak diacritic is
/// one character, as the server's split counts it) and whether it is over the
/// Bible character limit. Drives the slide card's count badge.
pub fn slide_char_count(text: &str, limit: u32) -> (usize, bool) {
    let count = text.chars().count();
    (count, count > limit as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slide_char_count_counts_characters_not_bytes() {
        // #828: "čšž" is 3 characters but 6 UTF-8 bytes.
        assert_eq!(slide_char_count("čšž", 320), (3, false));
        assert_eq!(slide_char_count("", 320), (0, false));
    }

    #[test]
    fn slide_char_count_flags_only_a_count_over_the_limit() {
        assert_eq!(slide_char_count("abcd", 4), (4, false));
        assert_eq!(slide_char_count("abcde", 4), (5, true));
    }
}
