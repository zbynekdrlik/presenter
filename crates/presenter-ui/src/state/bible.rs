use leptos::prelude::*;
use presenter_core::BibleTranslation;

/// Bible page specific state.
#[derive(Clone)]
pub struct BibleState {
    /// Available Bible translations.
    pub translations: RwSignal<Vec<BibleTranslation>>,
    /// Currently selected translation code.
    pub selected_translation: RwSignal<Option<String>>,
    /// Current search query.
    pub search_query: RwSignal<String>,
    /// Search results.
    pub search_results: RwSignal<Vec<crate::api::bible::BibleSearchHit>>,
    /// Whether a search is in progress.
    pub searching: RwSignal<bool>,
    /// Whether a search has been performed (to differentiate initial state from empty results).
    pub has_searched: RwSignal<bool>,
}

impl BibleState {
    pub fn new() -> Self {
        Self {
            translations: RwSignal::new(Vec::new()),
            selected_translation: RwSignal::new(None),
            search_query: RwSignal::new(String::new()),
            search_results: RwSignal::new(Vec::new()),
            searching: RwSignal::new(false),
            has_searched: RwSignal::new(false),
        }
    }
}

impl Default for BibleState {
    fn default() -> Self {
        Self::new()
    }
}
