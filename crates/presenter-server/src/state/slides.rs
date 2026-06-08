//! Slide composition and slide-edit operations for `AppState`.
//!
//! - [`compose`]: pure bible-slide composition (live mode + AI item stream).
//! - `edit_ops`: `AppState` slide CRUD (update/insert/duplicate/delete/reorder).
//!
//! Public composition symbols are re-exported here so external callers keep
//! their `crate::state::slides::{...}` paths.

mod compose;
mod edit_ops;

pub(crate) use compose::{
    compose_bible_items_into_slides, compose_bible_slides, BibleItem, ComposedBibleSlide,
};

#[cfg(test)]
mod tests;
