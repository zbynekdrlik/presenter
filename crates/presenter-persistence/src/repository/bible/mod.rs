//! SeaORM repository methods for bible data.
//!
//! Split into focused submodules, each contributing `impl Repository` methods:
//! - [`query`]      — read/search methods (translations, passages, FTS).
//! - [`import`]     — translation write/import methods (fast-import, digests).
//! - [`presentations`] — bible-presentation CRUD + slide row-mapping helpers.
//!
//! Every method stays a method on [`crate::repository::Repository`], so all
//! public paths are unchanged from when this lived in a single `bible.rs`.

mod import;
mod presentations;
mod query;

#[cfg(test)]
mod tests;
