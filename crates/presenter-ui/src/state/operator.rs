use leptos::prelude::*;

/// Operator-specific UI state.
#[derive(Clone)]
pub struct OperatorState {
    /// Currently focused slide ID in the editor.
    pub focused_slide_id: RwSignal<Option<String>>,
    /// Current search query text.
    pub search_query: RwSignal<String>,
    /// Whether search results are visible.
    pub search_open: RwSignal<bool>,
    /// Which modal is currently open (None = no modal).
    pub open_modal: RwSignal<Option<String>>,
    /// Modal edit target ID (library/playlist/presentation being edited).
    pub modal_target_id: RwSignal<Option<String>>,
    /// Line limit for slide display.
    pub line_limit: RwSignal<u32>,
    /// Catalog top panel height in pixels.
    pub catalog_top_height: RwSignal<Option<f64>>,
    /// Whether mobile nav is open.
    pub mobile_nav_open: RwSignal<bool>,
    /// Whether a submitting operation is in progress.
    pub submitting: RwSignal<bool>,
}

impl OperatorState {
    pub fn new() -> Self {
        let line_limit = crate::state::session::get("lineLimit")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(32);

        Self {
            focused_slide_id: RwSignal::new(crate::state::session::get("focusedSlideId")),
            search_query: RwSignal::new(String::new()),
            search_open: RwSignal::new(false),
            open_modal: RwSignal::new(None),
            modal_target_id: RwSignal::new(None),
            line_limit: RwSignal::new(line_limit),
            catalog_top_height: RwSignal::new(
                crate::state::session::get("catalogTopHeight").and_then(|v| v.parse::<f64>().ok()),
            ),
            mobile_nav_open: RwSignal::new(false),
            submitting: RwSignal::new(false),
        }
    }
}

impl Default for OperatorState {
    fn default() -> Self {
        Self::new()
    }
}
