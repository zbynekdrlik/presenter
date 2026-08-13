//! Slide composition and slide-edit operations for `AppState`.
//!
//! - [`compose`]: pure bible-slide composition (live mode + AI item stream).
//! - `edit_ops`: `AppState` slide CRUD (update/insert/duplicate/delete/reorder).
//!
//! Public composition symbols are re-exported here so external callers keep
//! their `crate::state::slides::{...}` paths.

mod compose;
mod edit_ops;

// `pub use` (not `pub(crate) use`): the #680 `ai_eval` Layer-1 scorer replays
// a captured `create_bible_presentation` call's items through these SAME
// pure functions/types from a separate crate root (`src/bin/ai_eval/`) — see
// that binary's `scorer` module. `compose_bible_slides`/`PasteSlidesError`
// ride along on the same re-export line for consistency; nothing else here
// changes.
pub use compose::{
    compose_bible_items_into_slides, compose_bible_slides, BibleItem, ComposedBibleSlide,
};
pub use edit_ops::PasteSlidesError;

#[cfg(test)]
mod tests;
