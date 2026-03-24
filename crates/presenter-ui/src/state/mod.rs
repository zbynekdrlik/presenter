pub mod bible;
pub mod operator;
pub mod session;
pub mod tablet;

use leptos::prelude::*;
use presenter_core::{
    BibleBroadcast, LibrarySummary, Playlist, Presentation, PresentationSummary, SearchResult,
    Slide, StageClientSnapshot, StageDisplayLayout, StageDisplaySnapshot, TimersOverview,
};

use crate::api::bible::BibleSearchHit;
use std::collections::{HashMap, HashSet};

use crate::api::settings::AbleSetStatusSnapshot;

/// Load stage monitor baseline from persistent storage.
fn load_baseline_from_storage() -> Option<(usize, usize)> {
    let connected = session::get_persistent("stageMonitorBaselineConnected")
        .and_then(|s| s.parse::<usize>().ok())?;
    let issues = session::get_persistent("stageMonitorBaselineIssues")
        .and_then(|s| s.parse::<usize>().ok())?;
    Some((connected, issues))
}

/// Save stage monitor baseline to persistent storage.
pub fn save_baseline_to_storage(connected: usize, issues: usize) {
    session::set_persistent("stageMonitorBaselineConnected", &connected.to_string());
    session::set_persistent("stageMonitorBaselineIssues", &issues.to_string());
}

#[derive(Clone)]
pub struct AppContext {
    pub view: RwSignal<String>,
    pub mode: RwSignal<String>,
    pub libraries: RwSignal<Vec<LibrarySummary>>,
    pub selected_library_id: RwSignal<Option<String>>,
    pub favorite_library_ids: RwSignal<HashSet<String>>,
    pub presentations: RwSignal<Vec<PresentationSummary>>,
    pub selected_presentation: RwSignal<Option<Presentation>>,
    pub selected_presentation_id: RwSignal<Option<String>>,
    pub playlists: RwSignal<Vec<Playlist>>,
    pub selected_playlist_id: RwSignal<Option<String>>,
    pub selected_playlist: RwSignal<Option<Playlist>>,
    pub stage_snapshot: RwSignal<Option<StageDisplaySnapshot>>,
    pub stage_layout_code: RwSignal<String>,
    pub stage_layouts: RwSignal<Vec<StageDisplayLayout>>,
    pub timers: RwSignal<Option<TimersOverview>>,
    pub broadcast_live: RwSignal<bool>,
    pub context_title: RwSignal<String>,
    pub stage_connections: RwSignal<Vec<StageClientSnapshot>>,
    pub stage_monitor_baseline: RwSignal<Option<(usize, usize)>>,
    pub toast_message: RwSignal<Option<String>>,
    pub toast_variant: RwSignal<String>,
    pub search_results: RwSignal<Vec<SearchResult>>,
    pub search_loading: RwSignal<bool>,
    pub ws_connected: RwSignal<bool>,
    pub ableset_status: RwSignal<Option<AbleSetStatusSnapshot>>,
    pub slides_cache: RwSignal<HashMap<String, Vec<Slide>>>,
    pub presentation_index: RwSignal<HashMap<String, String>>,
    // === Additional state for JS feature parity ===
    /// Reverse index: presentation_id -> list of playlist_ids containing it
    pub presentation_playlist_index: RwSignal<HashMap<String, Vec<String>>>,
    /// Quick lookup: playlist_id -> Playlist
    pub playlist_lookup: RwSignal<HashMap<String, Playlist>>,
    /// Active bible broadcast passage
    pub active_bible_broadcast: RwSignal<Option<BibleBroadcast>>,
    /// Loading state for libraries
    pub libraries_loading: RwSignal<bool>,
    /// Loading state for playlists
    pub playlists_loading: RwSignal<bool>,
    /// Bible search signals (shared between header and bible page)
    pub bible_search_query: RwSignal<String>,
    pub bible_search_results: RwSignal<Vec<BibleSearchHit>>,
    pub bible_searching: RwSignal<bool>,
    pub bible_has_searched: RwSignal<bool>,
    /// Bumped when a BibleSlidesChanged event arrives, so Bible page can re-fetch
    pub bible_presentations_version: RwSignal<u64>,
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
            favorite_library_ids: RwSignal::new(HashSet::new()),
            presentations: RwSignal::new(Vec::new()),
            selected_presentation: RwSignal::new(None),
            selected_presentation_id: RwSignal::new(session::get("currentPresentationId")),
            playlists: RwSignal::new(Vec::new()),
            selected_playlist_id: RwSignal::new(session::get("activePlaylistId")),
            selected_playlist: RwSignal::new(None),
            stage_snapshot: RwSignal::new(None),
            stage_layout_code: RwSignal::new(String::new()),
            stage_layouts: RwSignal::new(Vec::new()),
            timers: RwSignal::new(None),
            broadcast_live: RwSignal::new(false),
            context_title: RwSignal::new("Presentations".to_string()),
            stage_connections: RwSignal::new(Vec::new()),
            stage_monitor_baseline: RwSignal::new(load_baseline_from_storage()),
            toast_message: RwSignal::new(None),
            toast_variant: RwSignal::new("info".to_string()),
            search_results: RwSignal::new(Vec::new()),
            search_loading: RwSignal::new(false),
            ws_connected: RwSignal::new(false),
            ableset_status: RwSignal::new(None),
            slides_cache: RwSignal::new(HashMap::new()),
            presentation_index: RwSignal::new(HashMap::new()),
            presentation_playlist_index: RwSignal::new(HashMap::new()),
            playlist_lookup: RwSignal::new(HashMap::new()),
            active_bible_broadcast: RwSignal::new(None),
            libraries_loading: RwSignal::new(false),
            playlists_loading: RwSignal::new(false),
            bible_search_query: RwSignal::new(String::new()),
            bible_search_results: RwSignal::new(Vec::new()),
            bible_searching: RwSignal::new(false),
            bible_has_searched: RwSignal::new(false),
            bible_presentations_version: RwSignal::new(0),
        }
    }

    pub fn show_toast(&self, msg: &str, variant: &str) {
        let toast = self.toast_message;
        let toast_variant = self.toast_variant;
        toast_variant.set(variant.to_string());
        toast.set(Some(msg.to_string()));
        gloo_timers::callback::Timeout::new(3_500, move || {
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
