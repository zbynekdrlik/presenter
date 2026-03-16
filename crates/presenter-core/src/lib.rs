#![allow(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use
)]

//! Core domain models for the Presenter application.

pub mod ableset;
pub mod android_stage_display;
pub mod bible;
pub mod id;
pub mod library;
pub mod live;
pub mod osc;
pub mod playlist;
pub mod presentation;
pub mod resolume;
pub mod search;
pub mod slide;
pub mod stage_appearance;
pub mod stage_client;
pub mod stage_design;
pub mod stage_display;
pub mod timer;

pub use ableset::{
    extract_song_prefix, AbleSetSettings, AbleSetSettingsDraft, AbleSetSettingsValidationError,
    AbleSetSongSnapshot,
};
pub use android_stage_display::{
    AndroidStageDisplay, AndroidStageDisplayDraft, AndroidStageDisplayValidationError,
    DEFAULT_ADB_PORT, DEFAULT_LAUNCH_COMPONENT,
};
pub use bible::{
    BibleBroadcast, BiblePassage, BiblePreferences, BiblePreferencesDraft, BibleReference,
    BibleSlideOutput, BibleTranslation,
};
pub use id::{
    AndroidStageDisplayId, LibraryId, PlaylistEntryId, PlaylistId, PresentationId, ResolumeHostId,
    SlideId,
};
pub use library::{Library, LibrarySummary, PresentationSummary};
pub use live::{InboundMessage, LiveEvent};
pub use osc::{OscSettings, OscSettingsDraft, OscSettingsValidationError, VelocityMode};
pub use playlist::{Playlist, PlaylistEntry};
pub use presentation::Presentation;
pub use resolume::{ResolumeHost, ResolumeHostDraft, ResolumeHostValidationError};
pub use search::{SearchMatchField, SearchResult, SearchResultKind};
pub use slide::{ResolvedSlide, Slide, SlideContent, SlideGroup, SlideText};
pub use stage_appearance::StageAppearance;
pub use stage_client::{StageClientSnapshot, StageClientStatus};
pub use stage_design::{StageBox, StageBoxType, StageDesign, TextAlign};
pub use stage_display::{
    StageDisplayLayout, StageDisplaySlide, StageDisplaySnapshot, StagePlaylistEntry, StageState,
    DEFAULT_STAGE_LAYOUT_CODE,
};
pub use timer::{
    CountdownTimer, CountdownTimerSnapshot, PreachTimer, PreachTimerSnapshot, TimerCommand,
    TimerState, TimersOverview, TimersState,
};

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod tests;
