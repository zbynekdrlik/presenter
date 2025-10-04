//! Core domain models for the Presenter application.

pub mod ableset;
pub mod bible;
pub mod id;
pub mod library;
pub mod osc;
pub mod playlist;
pub mod presentation;
pub mod resolume;
pub mod search;
pub mod slide;
pub mod stage_display;
pub mod timer;

pub use ableset::{
    extract_song_prefix, AbleSetSettings, AbleSetSettingsDraft, AbleSetSettingsValidationError,
    AbleSetSongSnapshot,
};
pub use bible::{BibleBroadcast, BiblePassage, BibleReference, BibleTranslation};
pub use id::{LibraryId, PlaylistEntryId, PlaylistId, PresentationId, ResolumeHostId, SlideId};
pub use library::{Library, LibrarySummary, PresentationSummary};
pub use osc::{OscSettings, OscSettingsDraft, OscSettingsValidationError, VelocityMode};
pub use playlist::{Playlist, PlaylistEntry};
pub use presentation::Presentation;
pub use resolume::{ResolumeHost, ResolumeHostDraft, ResolumeHostValidationError};
pub use search::{SearchMatchField, SearchResult, SearchResultKind};
pub use slide::{ResolvedSlide, Slide, SlideContent, SlideGroup, SlideText};
pub use stage_display::{StageDisplayLayout, StageDisplaySlide, StageDisplaySnapshot, StageState};
pub use timer::{
    CountdownTimer, CountdownTimerSnapshot, PreachTimer, PreachTimerSnapshot, TimerCommand,
    TimerState, TimersOverview, TimersState,
};

#[cfg(test)]
mod tests;
