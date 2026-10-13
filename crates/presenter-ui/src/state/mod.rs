pub mod bible;
pub mod bible_range;
pub mod operator;
pub mod session;
pub mod slide_columns;
pub mod song_load;
pub mod stage;
pub mod tablet;

use leptos::prelude::*;
use presenter_core::{
    BibleBroadcast, LibrarySummary, Playlist, Presentation, PresentationId, PresentationSummary,
    SearchResult, Slide, StageClientSnapshot, StageDisplayLayout, StageDisplaySnapshot,
    StageTextMode, TimersOverview,
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

/// An AbleSet Follow open in flight (#853): the stage's song being fetched.
/// The selection moves only when the song arrives, so this records the open
/// until then (`components/stage_follow.rs`).
#[derive(Clone, Debug)]
pub struct FollowPending {
    /// Unique per open: a newer open replaces it, a stale one clears it; only
    /// an answer carrying the current token may be applied.
    pub token: u64,
    pub presentation_id: String,
    /// The selected song when the open started; the operator picking another
    /// song meanwhile cancels it. A failed load that moves the selection
    /// re-points it (`song_load::give_up_now`), since that move is no pick.
    pub selected_at_start: Option<String>,
    /// The Follow ON click: may open in Edit mode too (Follow must still be
    /// on when the song arrives).
    pub forced: bool,
    /// The stage's playlist occurrence (#496), applied with the song.
    pub entry_index: Option<u32>,
    /// `Date.now()` when the song was last requested; an open still waiting
    /// long after (a hung or very slow request) is requested again by the
    /// next stage change, under the same token.
    pub started_ms: f64,
    /// Bumped each time the song is requested again: only a failure of the
    /// latest request ends the open (an older one may fail while the newer
    /// one is still on its way).
    pub request: u32,
    /// The selected library and playlist when the open started: a pick by
    /// the operator since then keeps Follow's library step out. Follow's own
    /// library step moves it along, so it never counts as a pick.
    pub context_at_start: (Option<String>, Option<String>),
}

/// Whether `id` is still the selected song. A late fetch for a song the
/// operator or Follow has since replaced must not be shown (#853).
pub fn is_still_selected(selected: RwSignal<Option<String>>, id: &str) -> bool {
    selected.get_untracked().as_deref() == Some(id)
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
    /// #496: index of the playlist entry the operator selected/triggered, so a
    /// repeated song's correct OCCURRENCE is sent to the server. `None` when the
    /// selection came from a library (no playlist position).
    pub selected_entry_index: RwSignal<Option<u32>>,
    /// The playlist occurrence of the song actually SHOWN (#496, #853): what
    /// a click that cannot be shown restores, even when another click is
    /// still loading.
    pub shown_entry_index: StoredValue<Option<u32>>,
    /// The AbleSet Follow open in flight, if any (#853).
    pub follow_pending: StoredValue<Option<FollowPending>>,
    /// The last stage snapshot Follow acts on: every frame except a
    /// songplayer push to an API layout (a frame with no song, #799), which
    /// would otherwise hide the stage song from the catch-up paths.
    pub follow_stage: StoredValue<Option<StageDisplaySnapshot>>,
    /// The song a Follow open just showed: its slide list lands on the stage
    /// slide, every other newly shown song opens at the top (#271, #853).
    pub follow_landing: StoredValue<Option<PresentationId>>,
    pub stage_snapshot: RwSignal<Option<StageDisplaySnapshot>>,
    pub stage_layout_code: RwSignal<String>,
    pub stage_layouts: RwSignal<Vec<StageDisplayLayout>>,
    /// #799: which lyric text(s) the API stage layouts show.
    pub stage_text_mode: RwSignal<StageTextMode>,
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
            selected_entry_index: RwSignal::new(None),
            shown_entry_index: StoredValue::new(None),
            follow_pending: StoredValue::new(None),
            follow_stage: StoredValue::new(None),
            follow_landing: StoredValue::new(None),
            stage_snapshot: RwSignal::new(None),
            stage_layout_code: RwSignal::new(String::new()),
            stage_layouts: RwSignal::new(Vec::new()),
            stage_text_mode: RwSignal::new(StageTextMode::default()),
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
