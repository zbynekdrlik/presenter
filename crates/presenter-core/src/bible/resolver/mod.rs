//! One Bible reference resolver for the UI (WASM) and every server entry point
//! (#850 part A, spec §5). This first part holds the book-text fold and the
//! generated name table (`scripts/dev/gen_bible_book_names.py`); the grammar,
//! ranking and versification follow in A1b. The importer keeps its exact
//! `canonical_book_by_name` and never goes through this resolver.

mod fold;
#[rustfmt::skip]
pub mod names_table;

pub use fold::{fold_book_text, FoldedBook};

#[cfg(test)]
mod tests;
