mod ableset;
mod android_stage;
mod app;
mod companion_server;
mod presentations;
mod resolume;
mod settings;
mod stage;
mod timers;

pub use app::*;
pub use settings::FeatureFlags;

pub(super) use ableset::AbleSetLibraryCache;
pub(super) use companion_server::CompanionServerManager;
pub(super) use stage::blank_slide_content;
#[cfg(test)]
pub(super) use timers::format_countdown_text;

#[cfg(test)]
pub(crate) use stage::{sanitize_song_title, stage_resolution_from_presentation};

#[cfg(test)]
mod tests;

// UI line-limit policy shared between server and scripts.
// Exposed here so router handlers can validate inputs without depending on app internals.
pub const LINE_LIMIT_MIN: u16 = 10;
pub const LINE_LIMIT_MAX: u16 = 120;
pub const DEFAULT_LINE_LIMIT: u16 = 32;
