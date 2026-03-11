use leptos::prelude::*;

/// Operator-specific UI state.
#[derive(Clone)]
pub struct OperatorState {
    /// Currently focused slide ID in the editor.
    pub focused_slide_id: RwSignal<Option<String>>,
    /// Whether the slide editor panel is visible.
    pub editor_visible: RwSignal<bool>,
    /// Current search query.
    pub search_query: RwSignal<String>,
    /// Whether a modal dialog is open.
    pub modal_open: RwSignal<bool>,
}

impl OperatorState {
    pub fn new() -> Self {
        Self {
            focused_slide_id: RwSignal::new(None),
            editor_visible: RwSignal::new(false),
            search_query: RwSignal::new(String::new()),
            modal_open: RwSignal::new(false),
        }
    }
}

impl Default for OperatorState {
    fn default() -> Self {
        Self::new()
    }
}
