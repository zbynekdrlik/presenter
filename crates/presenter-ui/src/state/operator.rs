use leptos::prelude::*;

/// Focus restoration data: (slide_id, field_name, selection_start, selection_end)
pub type PendingFocus = (String, String, u32, u32);

#[derive(Clone)]
pub struct OperatorState {
    pub focused_slide_id: RwSignal<Option<String>>,
    pub focused_field: RwSignal<Option<String>>,
    /// Pending focus restoration after saves: (slide_id, field, sel_start, sel_end)
    pub pending_focus: RwSignal<Option<PendingFocus>>,
    pub search_query: RwSignal<String>,
    pub search_open: RwSignal<bool>,
    pub open_modal: RwSignal<Option<String>>,
    pub modal_target_id: RwSignal<Option<String>>,
    pub modal_mode: RwSignal<String>,
    pub line_limit: RwSignal<u32>,
    pub catalog_top_height: RwSignal<f64>,
    pub mobile_nav_open: RwSignal<bool>,
    pub submitting: RwSignal<bool>,
    pub paste_text: RwSignal<String>,
    pub import_mode: RwSignal<String>,
    pub dragging_presentation_id: RwSignal<Option<String>>,
    pub dragging_slide_id: RwSignal<Option<String>>,
    /// Skip click trigger to prevent double-fire: (slide_id, expires_at_ms)
    pub skip_click_trigger: RwSignal<Option<(String, f64)>>,

    // === Missing state properties for JS feature parity ===
    /// Whether search results are being dragged
    pub search_dragging: RwSignal<bool>,
    /// Whether a drag originated from search results
    pub dragging_from_search: RwSignal<bool>,
    /// Whether catalog resize is in progress
    pub catalog_resize_active: RwSignal<bool>,
    /// Whether a slide clear operation is in progress (prevents double-clear)
    pub clearing_slide: RwSignal<bool>,
    /// Whether countdown input is focused
    pub countdown_input_active: RwSignal<bool>,
    /// Whether countdown input has unsaved changes
    pub countdown_input_dirty: RwSignal<bool>,
    /// Snapshot of reorder state before drag (for undo)
    pub reorder_snapshot: RwSignal<Option<Vec<String>>>,
    /// Initial playlist state when editing (for change detection)
    pub playlist_edit_initial: RwSignal<Option<String>>,
    /// Whether stage layout is loading
    pub stage_layout_loading: RwSignal<bool>,
    /// Slide ID currently being triggered (for is-loading class)
    pub triggering_slide_id: RwSignal<Option<String>>,
}

impl OperatorState {
    pub fn new() -> Self {
        // Use persistent storage (localStorage) for settings that should survive tab close
        let line_limit = crate::state::session::get_persistent("lineLimit")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(32);

        let catalog_top_height = crate::state::session::get_persistent("catalogTopHeight")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(320.0);

        Self {
            focused_slide_id: RwSignal::new(crate::state::session::get("focusedSlideId")),
            focused_field: RwSignal::new(crate::state::session::get("focusedField")),
            pending_focus: RwSignal::new(None),
            search_query: RwSignal::new(String::new()),
            search_open: RwSignal::new(false),
            open_modal: RwSignal::new(None),
            modal_target_id: RwSignal::new(None),
            modal_mode: RwSignal::new("create".to_string()),
            line_limit: RwSignal::new(line_limit),
            catalog_top_height: RwSignal::new(catalog_top_height),
            mobile_nav_open: RwSignal::new(false),
            submitting: RwSignal::new(false),
            paste_text: RwSignal::new(String::new()),
            import_mode: RwSignal::new(String::new()),
            dragging_presentation_id: RwSignal::new(None),
            dragging_slide_id: RwSignal::new(None),
            skip_click_trigger: RwSignal::new(None),
            // New state properties for JS feature parity
            search_dragging: RwSignal::new(false),
            dragging_from_search: RwSignal::new(false),
            catalog_resize_active: RwSignal::new(false),
            clearing_slide: RwSignal::new(false),
            countdown_input_active: RwSignal::new(false),
            countdown_input_dirty: RwSignal::new(false),
            reorder_snapshot: RwSignal::new(None),
            playlist_edit_initial: RwSignal::new(None),
            stage_layout_loading: RwSignal::new(false),
            triggering_slide_id: RwSignal::new(None),
        }
    }
}

impl Default for OperatorState {
    fn default() -> Self {
        Self::new()
    }
}
