use crate::{
    slide::{ResolvedSlide, Slide as DomainSlide},
    PlaylistId, PresentationId, SlideId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Default stage layout code used across the application.
pub const DEFAULT_STAGE_LAYOUT_CODE: &str = "worship-snv";

/// API-driven stage layout code. The `/api/stage` endpoint and its gate
/// in `presenter-server` reference this code.
pub const API_STAGE_LAYOUT_CODE: &str = "api";

/// Built-in stage display layouts exposed by the Presenter server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDisplayLayout {
    pub code: String,
    pub name: String,
    pub description: String,
}

impl StageDisplayLayout {
    /// Returns the canonical layouts expected by the stage display endpoints.
    pub fn built_in() -> Vec<Self> {
        vec![
            Self::new(
                DEFAULT_STAGE_LAYOUT_CODE,
                "WORSHIP SNV",
                "Lyrics current/next line with group labels",
            ),
            Self::new(
                "worship-pp",
                "WORSHIP PP",
                "Lyrics view plus presentation overview sidebar",
            ),
            Self::new(
                "timer",
                "TIMER",
                "Countdown emphasis for service start cues",
            ),
            Self::new(
                "preach",
                "PREACH",
                "Stopwatch view for preacher with overtime indicator",
            ),
            Self::new(
                "ndi-fullscreen",
                "NDI FULLSCREEN",
                "Full viewport NDI video stream",
            ),
            Self::new("bible", "BIBLE", "Full-screen Bible passage display"),
            Self::new(
                "fulltext",
                "FULL TEXT",
                "Current slide's stage text auto-scaled across the whole screen",
            ),
            Self::api(),
            Self::new(
                "camera-crew",
                "CAMERA CREW",
                "Group-focused director / camera-crew monitor",
            ),
        ]
    }

    /// The API-driven stage layout (`API_STAGE_LAYOUT_CODE`). Infallible
    /// accessor so callers don't search `built_in()` and risk panicking on a
    /// missing entry.
    pub fn api() -> Self {
        Self::new("api", "API", "External API-driven stage display")
    }

    /// The single source of truth for which layouts the OPERATOR may select —
    /// the layout picker, `POST /stage/layout` validation, and per-slide
    /// stage-layout markers (#515) all share this set. `camera-crew` is
    /// internal-only (served at /ui/camera) and excluded.
    pub fn operator_selectable() -> Vec<Self> {
        Self::built_in()
            .into_iter()
            .filter(|layout| layout.code != "camera-crew")
            .collect()
    }

    /// Look up an operator-selectable layout by code (`None` for unknown
    /// codes AND for internal-only layouts like `camera-crew`).
    pub fn find_operator_selectable(code: &str) -> Option<Self> {
        Self::operator_selectable()
            .into_iter()
            .find(|layout| layout.code == code)
    }

    fn new(code: &str, name: &str, description: &str) -> Self {
        Self {
            code: code.to_string(),
            name: name.to_string(),
            description: description.to_string(),
        }
    }
}

/// One upcoming distinct group name for camera-crew layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingGroup {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagePlaylistEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_id: Option<PresentationId>,
    pub is_active: bool,
    pub entry_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDisplaySlide {
    pub main: String,
    pub translation: String,
    pub stage: String,
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDisplaySnapshot {
    pub layout: StageDisplayLayout,
    pub generated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_id: Option<PresentationId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_song_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_slide_id: Option<SlideId>,
    pub current: Option<StageDisplaySlide>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_slide_id: Option<SlideId>,
    pub next: Option<StageDisplaySlide>,
    pub timers: crate::timer::TimersOverview,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_position: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_slides: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_id: Option<PlaylistId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_entries: Option<Vec<StagePlaylistEntry>>,
    /// Index of the active playlist entry (#496). Lets the worship-pp sidebar
    /// highlight/scroll the exact triggered OCCURRENCE when a set repeats a
    /// song. `None` for non-playlist snapshots; the sidebar then falls back to
    /// the first `is_active` entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_entry_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upcoming_groups: Vec<UpcomingGroup>,
}

impl From<&DomainSlide> for StageDisplaySlide {
    fn from(slide: &DomainSlide) -> Self {
        let content = &slide.content;
        Self {
            main: content.main.value().to_string(),
            translation: content.translation.value().to_string(),
            stage: content.stage.value().to_string(),
            group: content.group.as_ref().map(|g| g.name().to_string()),
            group_color: None,
        }
    }
}

impl From<&ResolvedSlide> for StageDisplaySlide {
    fn from(slide: &ResolvedSlide) -> Self {
        Self {
            main: slide.main.value().to_string(),
            translation: slide.translation.value().to_string(),
            stage: slide.stage.value().to_string(),
            group: slide
                .effective_group
                .as_ref()
                .map(|group| group.name().to_string()),
            group_color: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StageState {
    pub presentation_id: Option<PresentationId>,
    pub current_slide_id: Option<SlideId>,
    pub next_slide_id: Option<SlideId>,
    #[serde(default)]
    pub playlist_id: Option<PlaylistId>,
    /// Zero-based index of the triggered playlist entry. Disambiguates which
    /// occurrence is active when a set repeats a song (the same
    /// `presentation_id` appears in multiple entries). `None` for non-playlist
    /// triggers or legacy state written before #496.
    #[serde(default)]
    pub active_entry_index: Option<u32>,
}

impl StageState {
    pub fn new(
        presentation_id: Option<PresentationId>,
        current_slide_id: Option<SlideId>,
        next_slide_id: Option<SlideId>,
        playlist_id: Option<PlaylistId>,
    ) -> Self {
        Self {
            presentation_id,
            current_slide_id,
            next_slide_id,
            playlist_id,
            active_entry_index: None,
        }
    }

    /// Builder-style setter for the triggered playlist-entry index (#496).
    pub fn with_active_entry_index(mut self, index: Option<u32>) -> Self {
        self.active_entry_index = index;
        self
    }

    pub fn cleared() -> Self {
        Self::new(None, None, None, None)
    }
}

impl StageDisplaySnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        layout: StageDisplayLayout,
        generated_at: DateTime<Utc>,
        presentation_id: Option<PresentationId>,
        presentation_name: Option<String>,
        library_name: Option<String>,
        song_name: Option<String>,
        song_number: Option<String>,
        next_song_name: Option<String>,
        current_slide_id: Option<SlideId>,
        current: Option<StageDisplaySlide>,
        next_slide_id: Option<SlideId>,
        next: Option<StageDisplaySlide>,
        timers: crate::timer::TimersOverview,
        latency_ms: Option<f64>,
        current_position: Option<u32>,
        total_slides: Option<u32>,
        playlist_id: Option<PlaylistId>,
        playlist_name: Option<String>,
        playlist_entries: Option<Vec<StagePlaylistEntry>>,
        upcoming_groups: Vec<UpcomingGroup>,
    ) -> Self {
        Self {
            layout,
            generated_at,
            presentation_id,
            presentation_name,
            library_name,
            song_name,
            song_number,
            next_song_name,
            current_slide_id,
            current,
            next_slide_id,
            next,
            timers,
            latency_ms,
            current_position,
            total_slides,
            playlist_id,
            playlist_name,
            playlist_entries,
            active_entry_index: None,
            upcoming_groups,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_selectable_excludes_only_camera_crew() {
        let selectable = StageDisplayLayout::operator_selectable();
        assert_eq!(selectable.len(), StageDisplayLayout::built_in().len() - 1);
        assert!(!selectable.iter().any(|layout| layout.code == "camera-crew"));
        assert!(selectable.iter().any(|layout| layout.code == "fulltext"));
    }

    #[test]
    fn find_operator_selectable_rejects_camera_crew_and_unknown() {
        assert!(StageDisplayLayout::find_operator_selectable("fulltext").is_some());
        assert!(StageDisplayLayout::find_operator_selectable("camera-crew").is_none());
        assert!(StageDisplayLayout::find_operator_selectable("nope").is_none());
    }

    #[test]
    fn built_in_layouts_cover_expected_variants() {
        let layouts = StageDisplayLayout::built_in();
        assert_eq!(layouts.len(), 9);
        let codes: Vec<_> = layouts.iter().map(|layout| layout.code.as_str()).collect();
        assert!(codes.contains(&DEFAULT_STAGE_LAYOUT_CODE));
        assert!(codes.contains(&"worship-pp"));
        assert!(codes.contains(&"timer"));
        assert!(codes.contains(&"preach"));
        assert!(codes.contains(&"ndi-fullscreen"));
        assert!(codes.contains(&"bible"));
        assert!(codes.contains(&"fulltext"));
        assert!(codes.contains(&"api"));
        assert!(codes.contains(&"camera-crew"));
    }
}

#[cfg(test)]
mod camera_crew_tests {
    use super::*;

    #[test]
    fn upcoming_group_round_trips_through_json() {
        let g = UpcomingGroup {
            name: "Verse 1".to_string(),
        };
        let json = serde_json::to_string(&g).unwrap();
        assert_eq!(json, r#"{"name":"Verse 1"}"#);
        let back: UpcomingGroup = serde_json::from_str(&json).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn built_in_layouts_include_camera_crew() {
        let codes: Vec<String> = StageDisplayLayout::built_in()
            .into_iter()
            .map(|l| l.code)
            .collect();
        assert!(codes.iter().any(|c| c == "camera-crew"), "codes={codes:?}");
    }

    #[test]
    fn stage_display_snapshot_omits_empty_upcoming_groups_in_json() {
        let now = chrono::Utc::now();
        let layout = StageDisplayLayout::built_in().into_iter().next().unwrap();
        let snap = StageDisplaySnapshot::new(
            layout,
            now,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            crate::timer::TimersOverview::demo(now),
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(), // upcoming_groups (NEW positional arg)
        );
        let json = serde_json::to_string(&snap).unwrap();
        assert!(
            !json.contains("upcomingGroups"),
            "empty upcoming_groups must not serialize: {json}"
        );
    }
}
