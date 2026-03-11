pub mod bible;
pub mod operator;
pub mod session;

use leptos::prelude::*;
use presenter_core::{
    LibrarySummary, Playlist, Presentation, PresentationSummary, SearchResult, StageClientSnapshot,
    StageDisplaySnapshot, TimersOverview,
};

/// Global application context shared across all components.
#[derive(Clone)]
pub struct AppContext {
    /// Current view: worship, bible, timers, settings
    pub view: RwSignal<String>,
    /// Current mode: live, edit
    pub mode: RwSignal<String>,
    /// All library summaries.
    pub libraries: RwSignal<Vec<LibrarySummary>>,
    /// Currently selected library ID.
    pub selected_library_id: RwSignal<Option<String>>,
    /// Presentations in the selected library/playlist.
    pub presentations: RwSignal<Vec<PresentationSummary>>,
    /// Currently selected presentation (full data with slides).
    pub selected_presentation: RwSignal<Option<Presentation>>,
    /// Currently selected presentation ID (for highlighting).
    pub selected_presentation_id: RwSignal<Option<String>>,
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
    /// Context title (library name or playlist name).
    pub context_title: RwSignal<String>,
    /// Stage connections.
    pub stage_connections: RwSignal<Vec<StageClientSnapshot>>,
    /// Toast message.
    pub toast_message: RwSignal<Option<String>>,
    /// Search results.
    pub search_results: RwSignal<Vec<SearchResult>>,
    /// Whether search is loading.
    pub search_loading: RwSignal<bool>,
}

impl AppContext {
    pub fn new() -> Self {
        let view = session::get("view").unwrap_or_else(|| "worship".to_string());
        let mode = session::get("mode").unwrap_or_else(|| "live".to_string());

        Self {
            view: RwSignal::new(view),
            mode: RwSignal::new(mode),
            libraries: RwSignal::new(Vec::new()),
            selected_library_id: RwSignal::new(session::get("activeLibraryId")),
            presentations: RwSignal::new(Vec::new()),
            selected_presentation: RwSignal::new(None),
            selected_presentation_id: RwSignal::new(session::get("currentPresentationId")),
            playlists: RwSignal::new(Vec::new()),
            selected_playlist_id: RwSignal::new(session::get("activePlaylistId")),
            stage_snapshot: RwSignal::new(None),
            timers: RwSignal::new(None),
            broadcast_live: RwSignal::new(false),
            context_title: RwSignal::new("Presentations".to_string()),
            stage_connections: RwSignal::new(Vec::new()),
            toast_message: RwSignal::new(None),
            search_results: RwSignal::new(Vec::new()),
            search_loading: RwSignal::new(false),
        }
    }

    /// Show a toast notification that auto-hides.
    pub fn show_toast(&self, msg: &str) {
        let toast = self.toast_message;
        toast.set(Some(msg.to_string()));
        gloo_timers::callback::Timeout::new(3_000, move || {
            toast.set(None);
        })
        .forget();
    }
}

impl Default for AppContext {
    fn default() -> Self {
        Self::new()
    }
}
