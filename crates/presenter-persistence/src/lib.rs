//! Persistence layer for Presenter.

#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::assigning_clones,
    clippy::cast_lossless,
    clippy::cast_sign_loss,
    clippy::items_after_statements,
    clippy::too_many_lines,
    clippy::map_unwrap_or,
    clippy::unnecessary_mut_passed,
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::redundant_closure,
    clippy::manual_let_else,
    clippy::clone_on_copy,
    clippy::unnecessary_wraps,
    clippy::match_same_arms,
    clippy::needless_pass_by_value
)]

pub mod audit;
pub mod entities;
mod repository;

pub use audit::{ResolumePushAuditEntry, SettingsAuditEntry, SettingsAuditSource};
pub use repository::{
    sync_should_apply, BookmarkPlace, DatabaseSettings, DeletedStudyBookmark,
    ListenTranscriptError, NewInterpreterTip, NewSermon, NewStreamAsset, NewStreamFont,
    NewTranscriptSentence, Repository, RepositoryError, SermonItemRecord, SermonRecord,
    SermonSnapshot, SermonSummary, SermonWrite, StrongsVerse, StudyBookmarkFolderRecord,
    StudyBookmarkRecord, SyncApplyOutcome, SyncLibraryManifestRow, SyncManifestRow,
    SyncPresentation, TrashedLibrary, TrashedPresentation, UiUsageCount, BOOKMARK_NOT_FOUND,
    FOLDER_NAME_TAKEN, FOLDER_NOT_FOUND, FOLDER_ORDER_STALE, PRUNE_HORIZON, SERMON_CONFLICT,
    SERMON_NOT_FOUND,
};
