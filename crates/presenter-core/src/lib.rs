//! Core domain models for the Presenter application.

pub mod bible;
pub mod id;
pub mod library;
pub mod playlist;
pub mod presentation;
pub mod slide;
pub mod timer;

pub use bible::{BiblePassage, BibleReference, BibleTranslation};
pub use id::{LibraryId, PlaylistId, PresentationId, SlideId};
pub use library::Library;
pub use playlist::{Playlist, PlaylistEntry};
pub use presentation::Presentation;
pub use slide::{ResolvedSlide, Slide, SlideContent, SlideGroup, SlideText};
pub use timer::{CountdownTimer, PreachTimer, TimerState};

#[cfg(test)]
mod tests;
