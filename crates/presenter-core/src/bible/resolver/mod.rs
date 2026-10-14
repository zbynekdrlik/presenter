//! One Bible reference resolver for the UI (WASM) and every server entry point
//! (#850 part A, spec §5). This first part holds the book-text fold and the
//! generated name table (`scripts/dev/gen_bible_book_names.py`); the grammar,
//! ranking and versification follow in A1b. The importer keeps its exact
//! `canonical_book_by_name` and never goes through this resolver.

mod fold;
mod grammar;
mod hint;
#[rustfmt::skip]
pub mod names_table;
mod rank;
mod types;
mod versification;

pub use fold::{fold_book_text, FoldedBook};
pub use hint::{
    chapters_hint, counted, segment_label, verses_hint, MISSING_BOOK_HINT, NO_BOOK_HINT,
};
pub use types::{
    BookCandidate, BookRef, MatchKind, ParsedQuery, Passage, RawSpec, Resolution, Segment, Tier,
};
pub use versification::{BookStructure, Step, Versification};

#[cfg(test)]
mod tests;
