pub mod bible;
pub mod operator;
pub mod session;

use leptos::prelude::*;
use presenter_core::{
    LibrarySummary, Playlist, Presentation, StageDisplaySnapshot, TimersOverview,
};

/// Global application context shared across all components.
#[derive(Clone)]
pub struct AppContext {
    /// All library summaries.
    pub libraries: RwSignal<Vec<LibrarySummary>>,
    /// Currently selected library ID.
    pub selected_library_id: RwSignal<Option<String>>,
    /// Presentations in the selected library.
    pub presentations: RwSignal<Vec<presenter_core::PresentationSummary>>,
    /// Currently selected presentation (full data with slides).
    pub selected_presentation: RwSignal<Option<Presentation>>,
    /// All playlists.
    pub playlists: RwSignal<Vec<Playlist>>,
    /// Currently selected playlist ID.
    pub selected_playlist_id: RwSignal<Option<String>>,
    /// Current stage display snapshot.
    pub stage_snapshot: RwSignal<Option<StageDisplaySnapshot>>,
    /// Current timers overview.
    pub timers: RwSignal<Option<TimersOverview>>,
    /// Whether broadcast live mode is enabled.
    pub broadcast_live: RwSignal<bool>,
}

impl AppContext {
    /// Create a new empty context with default signal values.
    pub fn new() -> Self {
        Self {
            libraries: RwSignal::new(Vec::new()),
            selected_library_id: RwSignal::new(None),
            presentations: RwSignal::new(Vec::new()),
            selected_presentation: RwSignal::new(None),
            playlists: RwSignal::new(Vec::new()),
            selected_playlist_id: RwSignal::new(None),
            stage_snapshot: RwSignal::new(None),
            timers: RwSignal::new(None),
            broadcast_live: RwSignal::new(false),
        }
    }
}

impl Default for AppContext {
    fn default() -> Self {
        Self::new()
    }
}
