//! SeaORM repository methods for bible data.
//!
//! Split into focused submodules, each contributing `impl Repository` methods:
//! - [`query`]      — read/search methods (translations, passages, FTS).
//! - [`import`]     — translation write/import methods (fast-import, digests).
//! - [`markup`]     — verse markup + Strong's occurrences (written by the import).
//! - [`occurrences`] — where a Strong's number stands.
//! - [`presentations`] — bible-presentation CRUD + slide row-mapping helpers.
//! - [`sermons`]    — sermons: a presentation plus its source items.
//! - [`study_sources`] — Strong's dictionaries and cross-references.
//!
//! Every method stays a method on [`crate::repository::Repository`], so all
//! public paths are unchanged from when this lived in a single `bible.rs`.

mod import;
mod markup;
mod occurrences;
mod presentation_order;
mod presentations;
mod query;
mod sermons;
mod study_sources;

pub use occurrences::StrongsVerse;
pub use sermons::{
    NewSermon, SermonItemRecord, SermonRecord, SermonSnapshot, SermonSummary, SermonWrite,
    SERMON_CONFLICT, SERMON_NOT_FOUND,
};

#[cfg(test)]
mod tests;
