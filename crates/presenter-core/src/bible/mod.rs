mod canonical;
mod presentation;
mod reference;
pub mod resolver;
pub mod search;
pub mod sermon;
mod translation;

pub use canonical::{
    canonical_book_by_code, canonical_book_by_name, canonical_book_by_number, BibleBookCanonical,
};
pub use presentation::{BiblePresentation, BiblePresentationSlide, BiblePresentationSummary};
pub use reference::{BibleReference, BibleReferenceError};
pub use search::normalise_book_key;
pub use translation::{
    BibleBookChapterSummary, BibleBroadcast, BibleIngestionBatch, BibleIngestionError,
    BiblePassage, BiblePreferences, BiblePreferencesDraft, BibleSlideOutput, BibleTranslation,
};

#[cfg(test)]
mod tests;
