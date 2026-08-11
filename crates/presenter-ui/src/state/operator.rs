use leptos::prelude::*;
use std::collections::{HashMap, HashSet};

/// Per-slide save status for the worship slide editor's "Saved" indicator (#313).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStatus {
    Saving,
    Saved,
    Failed,
}

/// Wait (bounded) until NO slide save is in flight (#556 F6 + #515).
///
/// A `get_presentation` refetch dropped by the `slide_edit_seq` staleness
/// guard may retry ONCE with a freshly captured seq — but the retry is only
/// safe AFTER the save whose bump invalidated the first response has
/// actually COMMITTED on the server. The seq bumps at save-START, so a
/// retry issued immediately can still be served from pre-save state, and
/// its tracked apply would blank the just-typed edit back out (the exact
/// #515 symptom, reproduced by `slide-stage-field-bugs.spec.ts`). Polling
/// the save-status map until no entry is `Saving` guarantees the retried
/// GET's server-side read postdates every save that caused the mismatch.
///
/// Returns `true` once quiescent; `false` if still saving after
/// `max_wait_ms` (callers then keep the old drop-the-response behavior,
/// which is always safe).
pub async fn await_saves_quiescent(
    save_status: RwSignal<HashMap<String, (SaveStatus, u64)>>,
    max_wait_ms: u32,
) -> bool {
    let mut waited: u32 = 0;
    loop {
        let any_saving = save_status
            .get_untracked()
            .values()
            .any(|(status, _)| *status == SaveStatus::Saving);
        if !any_saving {
            return true;
        }
        if waited >= max_wait_ms {
            return false;
        }
        gloo_timers::future::TimeoutFuture::new(50).await;
        waited += 50;
    }
}

/// Clipboard mode for slide copy/cut (#554): `Copy` pastes clones, `Cut` MOVES
/// the originals (only when a paste actually succeeds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardMode {
    Copy,
    Cut,
}

/// A slide clipboard captured within ONE presentation (#554). `presentation_id`
/// guards against a stale clipboard after switching songs and is the seam for a
/// future cross-presentation clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlideClipboard {
    /// Ids of the captured slides, in list order at capture time.
    pub slide_ids: Vec<String>,
    pub mode: ClipboardMode,
    pub presentation_id: String,
}

#[derive(Clone)]
pub struct OperatorState {
    pub focused_slide_id: RwSignal<Option<String>>,
    pub focused_field: RwSignal<Option<String>>,
    pub search_query: RwSignal<String>,
    pub search_open: RwSignal<bool>,
    pub open_modal: RwSignal<Option<String>>,
    pub modal_target_id: RwSignal<Option<String>>,
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
    /// Whether preach limit input is focused
    pub preach_limit_input_active: RwSignal<bool>,
    /// Whether preach limit input has unsaved changes
    pub preach_limit_input_dirty: RwSignal<bool>,
    /// Snapshot of reorder state before drag (for undo)
    pub reorder_snapshot: RwSignal<Option<Vec<String>>>,
    /// Initial playlist state when editing (for change detection)
    pub playlist_edit_initial: RwSignal<Option<String>>,
    /// Whether stage layout is loading
    pub stage_layout_loading: RwSignal<bool>,
    /// Slide ID currently being triggered (for is-loading class)
    pub triggering_slide_id: RwSignal<Option<String>>,
    /// Per-slide save status keyed by slide_id, with a monotonic token to
    /// prevent stale fade timers from clearing a newer save's entry (#313).
    pub save_status: RwSignal<HashMap<String, (SaveStatus, u64)>>,
    /// Monotonic counter bumped every time a slide-content save (main /
    /// translation / stage / group) completes successfully (#515). A
    /// `get_presentation` refetch captures this value before it is issued;
    /// if the counter has moved by the time the fetch resolves, a save
    /// landed while the fetch was in flight, so the response reflects
    /// pre-edit content and must be dropped instead of clobbering the newer
    /// edit — otherwise a slide's stage text could vanish from the editor
    /// right after typing it (the presentation-open refetch resolving late).
    pub slide_edit_seq: RwSignal<u64>,
    /// Monotonic counter bumped after every slide-content save that can
    /// affect the Group cascade (#556 F2). The Group field's blur save goes
    /// through the same `update_untracked` path as every other field (see
    /// `apply_saved_content_locally` in `slide_save.rs`) to avoid forcing a
    /// full, unkeyed slide-list rebuild on every save — but that means the
    /// header badge/placeholder markup, which is derived from the WHOLE
    /// presentation's group cascade, can't just read `selected_pres.get()`
    /// reactively either. Each slide's badge/placeholder instead renders as
    /// its OWN small fine-grained closure that reads THIS counter (to know
    /// something changed) plus `selected_pres.read_untracked()` (for the
    /// actual data) — the same isolation trick already used for
    /// `stage_snapshot` elsewhere in `slide_list.rs`.
    pub groups_version: RwSignal<u64>,
    /// Multi-selected slide ids in the operator editor (#554). Same shape as
    /// the Bible page's `selected_slide_ids`.
    pub selected_slide_ids: RwSignal<HashSet<String>>,
    /// The copy/cut clipboard; `None` when empty. Cleared on presentation switch.
    pub clipboard: RwSignal<Option<SlideClipboard>>,
    /// Id of the last checkbox-clicked slide — the anchor for Shift+click
    /// range selection (#554). Stored as a SLIDE ID, not a positional index
    /// (#558 V9): a raw index is never invalidated when the list shifts
    /// (paste/delete/reorder), so a later Shift+click could range-select
    /// the wrong slides. Resolved back to an index at Shift+click time via
    /// `range_select_by_anchor_id`.
    pub selection_anchor_id: RwSignal<Option<String>>,
    /// Gap index (0..=len) the user last hovered/selected as the Ctrl/Cmd+V
    /// paste target; `None` → paste at the end (#554).
    pub paste_target_gap: RwSignal<Option<usize>>,
    /// True while the clipboard block is being dragged toward an insertion gap.
    pub dragging_clipboard: RwSignal<bool>,
    /// True while the operator is actively CHOOSING a paste target (#602
    /// defect 3): the single-column grid + "Paste here" bars are bound to
    /// THIS interaction, never to "a clipboard exists". Set true the moment
    /// a copy/cut populates the clipboard; set false the moment a paste
    /// COMPLETES (Copy or Cut alike) — even though a completed Copy
    /// deliberately KEEPS `clipboard` non-empty so the toolbar Paste button
    /// / Ctrl+V can paste again without re-copying. Without this split, a
    /// completed Copy-paste left the grid stuck in single-column forever
    /// (only Clear/Escape/reload recovered it) because the class was keyed
    /// on `clipboard.is_some()`, which Copy never clears.
    pub choosing_paste_target: RwSignal<bool>,
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
            search_query: RwSignal::new(String::new()),
            search_open: RwSignal::new(false),
            open_modal: RwSignal::new(None),
            modal_target_id: RwSignal::new(None),
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
            preach_limit_input_active: RwSignal::new(false),
            preach_limit_input_dirty: RwSignal::new(false),
            reorder_snapshot: RwSignal::new(None),
            playlist_edit_initial: RwSignal::new(None),
            stage_layout_loading: RwSignal::new(false),
            triggering_slide_id: RwSignal::new(None),
            save_status: RwSignal::new(HashMap::new()),
            slide_edit_seq: RwSignal::new(0),
            groups_version: RwSignal::new(0),
            selected_slide_ids: RwSignal::new(HashSet::new()),
            clipboard: RwSignal::new(None),
            selection_anchor_id: RwSignal::new(None),
            paste_target_gap: RwSignal::new(None),
            dragging_clipboard: RwSignal::new(false),
            choosing_paste_target: RwSignal::new(false),
        }
    }
}

impl Default for OperatorState {
    fn default() -> Self {
        Self::new()
    }
}
