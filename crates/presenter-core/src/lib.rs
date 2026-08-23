#![allow(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use
)]

//! Core domain models for the Presenter application.
//!
//! # Naming convention
//! - Unprefixed types (`Library`, `Presentation`, `Slide`, `SlideContent`) mean WORSHIP.
//! - Bible-prefixed types (`BiblePresentation`, `BiblePresentationSlide`) mean BIBLE.
//! - Bible has no library wrapper — there is exactly one bible per system.

pub mod ableset;
pub mod ai_auth;
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
pub mod stage_client;
pub mod stage_display;
pub mod stream;
pub mod sync;
pub mod timer;
pub mod video_source;

pub mod feature_flags;

pub use ableset::{
    extract_song_prefix, normalize_title_for_mismatch, strip_song_prefix, AbleSetResolutionAttempt,
    AbleSetSettings, AbleSetSettingsDraft, AbleSetSettingsValidationError, AbleSetSongSnapshot,
    AbleSetStatusSnapshot, AbleSetTitleMismatch,
};
pub use ai_auth::{is_expiring_soon, EXPIRY_WARNING_WINDOW};
pub use android_stage_display::{
    stage_app_install_action, AndroidStageDisplay, AndroidStageDisplayDraft,
    AndroidStageDisplayValidationError, StageAppInstallAction, DEFAULT_ADB_PORT,
    DEFAULT_LAUNCH_PACKAGE,
};
pub use bible::{
    BibleBroadcast, BiblePassage, BiblePreferences, BiblePreferencesDraft, BiblePresentation,
    BiblePresentationSlide, BiblePresentationSummary, BibleReference, BibleSlideOutput,
    BibleTranslation,
};
pub use feature_flags::FeatureFlags;
pub use id::{
    AndroidStageDisplayId, BiblePresentationId, BibleSlideId, LibraryId, PlaylistEntryId,
    PlaylistId, PresentationId, ResolumeHostId, SlideId, VideoSourceId,
};
pub use library::{Library, LibrarySummary, PresentationSummary};
pub use live::{InboundMessage, LiveEvent};
pub use osc::{OscSettings, OscSettingsDraft, OscSettingsValidationError, VelocityMode};
pub use playlist::{Playlist, PlaylistEntry};
pub use presentation::Presentation;
pub use resolume::{ResolumeHost, ResolumeHostDraft, ResolumeHostValidationError};
pub use search::{SearchMatchField, SearchResult, SearchResultKind};
pub use slide::{resolve_sequence, ResolvedSlide, Slide, SlideContent, SlideGroup, SlideText};
pub use stage_client::{NdiVideoDiag, StageClientSnapshot, StageClientStatus};
pub use stage_display::{
    StageDisplayLayout, StageDisplaySlide, StageDisplaySnapshot, StagePlaylistEntry, StageState,
    UpcomingGroup, API_STAGE_LAYOUT_CODE, DEFAULT_STAGE_LAYOUT_CODE,
};
pub use stream::{
    validate_props, validate_scene_name, validate_slug, ContentTransition, Frame, ImageFit,
    SceneKind, Shadow, StreamAsset, StreamElementDef, StreamElementProps, StreamOutputDef,
    StreamOutputSummary, StreamSceneDef, StreamShowState, StreamValidationError, TextAlign,
    TextStyle, RESERVED_STREAM_SLUGS, STREAM_DEFAULT_FADE_MS, STREAM_FONT_FAMILIES,
    STREAM_SCENE_NAME_MAX, STREAM_SLUG_MAX, STREAM_TRANSITION_MAX_MS,
};
pub use sync::{sync_id_for_name, SYNC_ID_NAMESPACE};
pub use timer::{
    format_countdown, CountdownTimer, CountdownTimerSnapshot, PreachTimer, PreachTimerSnapshot,
    TimerCommand, TimerState, TimersOverview, TimersState,
};
pub use video_source::{VideoSource, VideoSourceDraft, VideoSourceValidationError};

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod tests;
