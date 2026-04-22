use leptos::prelude::*;
use presenter_core::BibleTranslation;
use std::collections::HashSet;

use crate::api::bible::{BibleBookDto, BiblePresentationSummary, BibleSlideDto};

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

    // -- Presentation edit modal --
    pub modal_presentation_id: RwSignal<Option<String>>,
    pub modal_presentation_name: RwSignal<String>,
}

impl BibleState {
    pub fn new() -> Self {
        Self {
            translations: RwSignal::new(Vec::new()),
            selected_translation: RwSignal::new(None),
            secondary_translation: RwSignal::new(None),

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
            modal_presentation_id: RwSignal::new(None),
            modal_presentation_name: RwSignal::new(String::new()),
        }
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

/// Result of clamping a chapter/verse selection against a book's structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClampedSelection {
    pub chapter: u16,
    pub verse_start: u16,
    pub verse_end: Option<u16>,
}

/// Clamp chapter/verse values against a book's chapter and verse counts.
///
/// Preserves values when they fit; clamps when they don't. If `verse_end`
/// becomes less than or equal to `verse_start`, returns `verse_end = None`
/// (single-verse semantics).
///
/// `chapter_count` is the number of chapters in the book (1-based max chapter).
/// `verse_counts` must have one entry per chapter, indexed by `chapter - 1`.
pub fn clamp_selection(
    chapter_count: u16,
    verse_counts: &[u16],
    chapter: u16,
    verse_start: u16,
    verse_end: Option<u16>,
) -> ClampedSelection {
    if chapter_count == 0 || verse_counts.is_empty() {
        return ClampedSelection {
            chapter: 1,
            verse_start: 1,
            verse_end: None,
        };
    }

    let clamped_chapter = chapter.clamp(1, chapter_count);
    let idx = (clamped_chapter - 1) as usize;
    let max_verse = verse_counts.get(idx).copied().unwrap_or(1).max(1);

    let clamped_start = verse_start.clamp(1, max_verse);

    let clamped_end = match verse_end {
        Some(end) => {
            let bounded = end.min(max_verse);
            if bounded <= clamped_start {
                None
            } else {
                Some(bounded)
            }
        }
        None => None,
    };

    ClampedSelection {
        chapter: clamped_chapter,
        verse_start: clamped_start,
        verse_end: clamped_end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_values_when_they_fit() {
        let result = clamp_selection(50, &vec![31; 50], 3, 5, Some(10));
        assert_eq!(
            result,
            ClampedSelection {
                chapter: 3,
                verse_start: 5,
                verse_end: Some(10),
            }
        );
    }

    #[test]
    fn clamps_chapter_when_too_high() {
        let result = clamp_selection(5, &vec![20; 5], 10, 3, Some(7));
        assert_eq!(result.chapter, 5);
        assert_eq!(result.verse_start, 3);
        assert_eq!(result.verse_end, Some(7));
    }

    #[test]
    fn clamps_verse_start_when_chapter_has_fewer_verses() {
        let result = clamp_selection(5, &vec![10, 5, 10, 10, 10], 2, 8, None);
        assert_eq!(result.chapter, 2);
        assert_eq!(result.verse_start, 5);
        assert_eq!(result.verse_end, None);
    }

    #[test]
    fn clamps_verse_end_to_chapter_max() {
        let result = clamp_selection(5, &vec![10, 5, 10, 10, 10], 2, 1, Some(20));
        assert_eq!(result.verse_start, 1);
        assert_eq!(result.verse_end, Some(5));
    }

    #[test]
    fn clears_verse_end_when_it_collapses_to_verse_start() {
        let result = clamp_selection(5, &vec![10, 5, 10, 10, 10], 2, 5, Some(20));
        assert_eq!(result.verse_start, 5);
        assert_eq!(result.verse_end, None);
    }

    #[test]
    fn returns_defaults_for_empty_book() {
        let result = clamp_selection(0, &[], 3, 5, Some(10));
        assert_eq!(
            result,
            ClampedSelection {
                chapter: 1,
                verse_start: 1,
                verse_end: None,
            }
        );
    }
}
