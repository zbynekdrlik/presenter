//! Remote, API-served Bible translations (#826).
//!
//! Every other translation is a local file ingested into SQLite. The New
//! Living Translation (NLT, Tyndale House) cannot be one: it is copyrighted
//! and this repository is public. Tyndale's own NLT API (`api.nlt.to`) exists
//! so non-commercial apps can DISPLAY NLT text, so `eng-nlt` is served from it
//! on demand — fetched when a passage is loaded, kept in a bounded in-memory
//! cache, never stored and never bulk-copied (owner decision on #826).
//!
//! The NLT's book/chapter STRUCTURE (book picker, chapter and verse counts)
//! and its English book names come from `eng-kjv` — the same 66-book canon —
//! so the NLT has no DB rows and needs no migration. Which source serves a
//! translation code is decided in ONE place: `state/bible_source.rs`.

mod books;
mod cache;
mod client;
mod parse;
#[cfg(test)]
mod tests;

use std::time::Duration;

use presenter_core::bible::{BibleBookCanonical, BibleBookChapterSummary};
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};

pub(crate) use client::NltClient;
pub(crate) use parse::NltVerse;

/// Translation code of the New Living Translation.
pub(crate) const NLT_CODE: &str = "eng-nlt";

/// The installed translation whose books, chapters and English book names
/// the NLT reuses.
pub(crate) const STRUCTURE_TRANSLATION_CODE: &str = "eng-kjv";

/// `true` for a translation served by a remote API instead of the database.
pub(crate) fn is_remote_translation(code: &str) -> bool {
    code.trim().eq_ignore_ascii_case(NLT_CODE)
}

/// The NLT descriptor listed next to the installed translations. Its short
/// code (`NLT`, from the code suffix) is the "(NLT)" attribution Tyndale
/// requires after every quotation.
pub(crate) fn nlt_translation() -> BibleTranslation {
    BibleTranslation::new(NLT_CODE, "New Living Translation", "en")
        .with_source("Tyndale NLT API (api.nlt.to), fetched on demand")
}

/// The translation whose `bible_passages` rows describe `code`'s books and
/// chapters: `eng-kjv` for the NLT, the translation itself otherwise.
pub(crate) fn structure_source(code: &str) -> &str {
    if is_remote_translation(code) {
        STRUCTURE_TRANSLATION_CODE
    } else {
        code
    }
}

/// Chapters where the NLT numbers MORE verses than eng-kjv, whose structure
/// it otherwise reuses: `(book code, chapter, NLT verse count)`. Checked live
/// on 2026-10-09 against eng-kjv (3 John 1 = 14 verses, Revelation 12 = 17).
const NLT_LONGER_CHAPTERS: [(&str, u16, u16); 2] = [("3JN", 1, 15), ("REV", 12, 18)];

/// eng-kjv's chapter summaries with the NLT's longer chapters applied, so a
/// whole-chapter load with the NLT as main reaches its last verse.
pub(crate) fn nlt_structure(
    mut summaries: Vec<BibleBookChapterSummary>,
) -> Vec<BibleBookChapterSummary> {
    for summary in &mut summaries {
        let Some(code) = summary.book_code.as_deref() else {
            continue;
        };
        let longer = NLT_LONGER_CHAPTERS.iter().find(|(book, chapter, _)| {
            book.eq_ignore_ascii_case(code) && *chapter == summary.chapter
        });
        if let Some((_, _, verses)) = longer {
            summary.verse_count = summary.verse_count.max(*verses);
        }
    }
    summaries
}

/// One single-verse NLT passage per fetched verse, named with `book` (the
/// `eng-kjv` name of the book) and the canonical code/number.
pub(crate) fn nlt_passages(
    verses: &[NltVerse],
    book: &str,
    canon: BibleBookCanonical,
) -> Vec<BiblePassage> {
    let translation = nlt_translation();
    verses
        .iter()
        .filter_map(|verse| {
            BibleReference::new_with_code(
                book,
                canon.code,
                canon.number,
                verse.chapter,
                verse.verse,
                verse.verse,
            )
            .ok()
            .map(|reference| BiblePassage::new(reference, translation.clone(), verse.text.clone()))
        })
        .collect()
}

/// A failed NLT fetch. As the MAIN translation's error the router maps it to
/// 503 (the API could not be reached) or 502 (it answered, but unusably) with
/// this Slovak message, shown in the "Failed to load passage" toast; as the
/// SECONDARY translation's error the main text loads anyway and this message
/// becomes the resolve `warning` toast (`AppState::generate_bible_slides`).
#[derive(Debug, thiserror::Error)]
pub(crate) enum RemoteBibleError {
    /// No answer within the request timeout.
    #[error("NLT nedostupné — API/internet: {reference} neodpovedalo do {timeout:?}")]
    Timeout {
        reference: String,
        timeout: Duration,
    },
    /// The request got no HTTP answer at all (DNS, refused connection, TLS).
    #[error("NLT nedostupné — API/internet: {reference}: {detail}")]
    Network { reference: String, detail: String },
    /// The API answered with a non-success HTTP status.
    #[error("NLT nedostupné — API/internet: {reference} vrátilo HTTP {status}")]
    Status { reference: String, status: u16 },
    /// The API answered 200, but the page held no verse at all.
    #[error("NLT nedostupné — API/internet: {reference} nevrátilo žiadny verš")]
    NoVerses { reference: String },
    /// The API was unreachable a moment ago; not asked again yet.
    #[error(
        "NLT nedostupné — API/internet: {reference}: posledný pokus zlyhal, \
         ďalší o {retry_secs} s"
    )]
    Backoff { reference: String, retry_secs: u64 },
}

impl RemoteBibleError {
    /// `true` when the API could not be reached (→ 503); `false` when it
    /// answered with something unusable (→ 502).
    pub(crate) fn is_unreachable(&self) -> bool {
        matches!(
            self,
            Self::Timeout { .. } | Self::Network { .. } | Self::Backoff { .. }
        )
    }
}
