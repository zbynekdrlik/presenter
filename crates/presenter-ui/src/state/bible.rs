use leptos::prelude::*;
use presenter_core::BibleTranslation;
use std::collections::HashSet;

use crate::api::bible::{BibleBookDto, BiblePresentationSummary, BibleSearchHit, BibleSlideDto};

/// Selected book in the Live tab.
#[derive(Clone, Debug)]
pub struct SelectedBook {
    pub book: String,
    pub code: String,
    pub number: u16,
    pub chapter_count: u16,
    pub verse_counts: Vec<u16>,
}

/// A previously loaded passage for the history list.
#[derive(Clone, Debug)]
pub struct LoadedPassage {
    pub book: String,
    pub book_code: String,
    pub book_number: u16,
    pub chapter: u16,
    pub verse_start: u16,
    pub verse_end: Option<u16>,
    pub translation_code: String,
    pub label: String,
}

/// Bible page specific state.
#[derive(Clone)]
pub struct BibleState {
    // -- Translations --
    pub translations: RwSignal<Vec<BibleTranslation>>,
    pub selected_translation: RwSignal<Option<String>>,
    pub secondary_translation: RwSignal<Option<String>>,

    // -- Search --
    pub search_query: RwSignal<String>,
    pub search_results: RwSignal<Vec<BibleSearchHit>>,
    pub searching: RwSignal<bool>,
    pub has_searched: RwSignal<bool>,

    // -- Book / chapter / verse selection --
    pub books: RwSignal<Vec<BibleBookDto>>,
    pub book_filter: RwSignal<String>,
    pub selected_book: RwSignal<Option<SelectedBook>>,
    pub selected_chapter: RwSignal<u16>,
    pub verse_start: RwSignal<u16>,
    pub verse_end: RwSignal<Option<u16>>,

    // -- Slides --
    pub slides: RwSignal<Vec<BibleSlideDto>>,
    pub loading_slides: RwSignal<bool>,
    pub selected_slide_ids: RwSignal<HashSet<String>>,

    // -- Tabs --
    pub bible_tab: RwSignal<String>,

    // -- Presentations --
    pub presentations: RwSignal<Vec<BiblePresentationSummary>>,
    pub active_presentation_id: RwSignal<Option<String>>,
    pub active_presentation_slides: RwSignal<Vec<BibleSlideDto>>,

    // -- Preferences --
    pub character_limit: RwSignal<u32>,

    // -- Loaded passages history (max 12) --
    pub loaded_passages_history: RwSignal<Vec<LoadedPassage>>,

    // -- Drag state for prepared slide reorder --
    pub drag_source_idx: RwSignal<Option<usize>>,
    pub drag_over_idx: RwSignal<Option<usize>>,
}

impl BibleState {
    pub fn new() -> Self {
        Self {
            translations: RwSignal::new(Vec::new()),
            selected_translation: RwSignal::new(None),
            secondary_translation: RwSignal::new(None),

            search_query: RwSignal::new(String::new()),
            search_results: RwSignal::new(Vec::new()),
            searching: RwSignal::new(false),
            has_searched: RwSignal::new(false),

            books: RwSignal::new(Vec::new()),
            book_filter: RwSignal::new(String::new()),
            selected_book: RwSignal::new(None),
            selected_chapter: RwSignal::new(1),
            verse_start: RwSignal::new(1),
            verse_end: RwSignal::new(None),

            slides: RwSignal::new(Vec::new()),
            loading_slides: RwSignal::new(false),
            selected_slide_ids: RwSignal::new(HashSet::new()),

            bible_tab: RwSignal::new("live".to_string()),

            presentations: RwSignal::new(Vec::new()),
            active_presentation_id: RwSignal::new(None),
            active_presentation_slides: RwSignal::new(Vec::new()),

            character_limit: RwSignal::new(320),

            loaded_passages_history: RwSignal::new(Vec::new()),
            drag_source_idx: RwSignal::new(None),
            drag_over_idx: RwSignal::new(None),
        }
    }

    /// Add a passage to the loaded history (max 12, most recent first).
    pub fn push_history(&self, passage: LoadedPassage) {
        self.loaded_passages_history.update(|history| {
            // Remove duplicate if exists
            history.retain(|p| p.label != passage.label);
            history.insert(0, passage);
            history.truncate(12);
        });
    }

    /// Get filtered books based on the book_filter signal.
    pub fn filtered_books(&self) -> Vec<BibleBookDto> {
        let filter = self.book_filter.get().to_lowercase();
        let all = self.books.get();
        if filter.is_empty() {
            return all;
        }
        all.into_iter()
            .filter(|b| b.book.to_lowercase().contains(&filter))
            .collect()
    }
}

impl Default for BibleState {
    fn default() -> Self {
        Self::new()
    }
}
