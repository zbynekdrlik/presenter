//! Which source serves a Bible translation (#826): the local SQLite
//! repository, or a remote API (`crate::bible_remote` — the NLT).
//!
//! Every verse and book/chapter read of the server goes through these
//! `AppState` methods — `generate_bible_slides`, `trigger_bible_passage`, the
//! `/bible/*` router and the AI tools — so a remote translation works on every
//! surface (operator, stage, stream, Resolume) with no per-call-site branch.
//! A remote translation has no DB rows: its books and chapters come from its
//! structure translation (`eng-kjv`), its verses from the API.

use super::AppState;
use crate::bible_remote;
use presenter_core::bible::resolver::{resolve_book_name, BookCandidate};
use presenter_core::bible::{
    canonical_book_by_code, BibleBookCanonical, BibleBookChapterSummary, BookScope,
};
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};

pub(crate) mod study;

/// A typed name in a log line is cut here: it is client input.
const TYPED_NAME_MAX_CHARS: usize = 120;

impl AppState {
    /// The installed translations, then the remote ones. `eng-nlt` is listed
    /// only when `eng-kjv` — the source of its books and chapters — is
    /// installed, and AFTER the installed translations (it needs internet).
    pub async fn list_bible_translations(&self) -> anyhow::Result<Vec<BibleTranslation>> {
        let mut translations = self.repository.list_bible_translations().await?;
        let structure_installed = translations.iter().any(|translation| {
            translation
                .code
                .eq_ignore_ascii_case(bible_remote::STRUCTURE_TRANSLATION_CODE)
        });
        if structure_installed {
            translations.push(bible_remote::nlt_translation());
        }
        Ok(translations)
    }

    /// Full-text verse search over the INSTALLED translations. A remote
    /// translation has no local text to search, so asking for one is an empty
    /// result, never an error (and a cross-translation search never sees it).
    pub async fn search_bible_passages_cross(
        &self,
        translation_code: Option<&str>,
        query: &str,
        scope: BookScope,
        limit: u32,
    ) -> anyhow::Result<Vec<BiblePassage>> {
        if let Some(code) =
            translation_code.filter(|code| bible_remote::is_remote_translation(code))
        {
            tracing::debug!(
                translation = code,
                "Bible search: remote translation is not searchable"
            );
            return Ok(Vec::new());
        }
        self.repository
            .search_bible_passages_cross(translation_code, query, scope, limit)
            .await
    }

    /// The passage whose verse span equals `reference`'s. Remote passages
    /// are single verses, so a multi-verse reference finds none — the same
    /// contract as a repository row lookup.
    pub async fn find_bible_passage(
        &self,
        translation_code: &str,
        reference: &BibleReference,
    ) -> anyhow::Result<Option<BiblePassage>> {
        if !bible_remote::is_remote_translation(translation_code) {
            return self.find_local_passage(translation_code, reference).await;
        }
        if reference.verse_start != reference.verse_end {
            return Ok(None);
        }
        let passages = self
            .remote_passage_range(
                &reference.book,
                reference.book_code.as_deref(),
                reference.chapter,
                reference.verse_start,
                reference.verse_end,
            )
            .await?;
        Ok(passages.into_iter().next())
    }

    /// The single-verse passages `verse_start..=verse_end` of one chapter, in
    /// order. `book_code` wins over the `book` name when given. A remote
    /// translation that cannot be reached is an error carrying
    /// `bible_remote::RemoteBibleError` (the router answers 502/503).
    pub async fn bible_passage_range(
        &self,
        translation_code: &str,
        book: &str,
        book_code: Option<&str>,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
    ) -> anyhow::Result<Vec<BiblePassage>> {
        if bible_remote::is_remote_translation(translation_code) {
            return self
                .remote_passage_range(book, book_code, chapter, verse_start, verse_end)
                .await;
        }
        // A code, else the translation's own name first (#850 §5.8), so whatever
        // worked keeps working.
        let given = given_code(book_code);
        let rows = self
            .repository
            .bible_passage_range(
                translation_code,
                book,
                given,
                chapter,
                verse_start,
                verse_end,
            )
            .await?;
        if given.is_some() || !rows.is_empty() {
            return Ok(rows);
        }
        let Some(candidate) = resolve_book_name(book) else {
            return Ok(rows);
        };
        let rows = self
            .repository
            .bible_passage_range(
                translation_code,
                book,
                Some(candidate.code.as_str()),
                chapter,
                verse_start,
                verse_end,
            )
            .await?;
        if !rows.is_empty() {
            log_typed_book(book, &candidate);
        }
        Ok(rows)
    }

    /// One row of the repository: under the translation's own name, else
    /// under the book the typed name means (#850 §5.8).
    async fn find_local_passage(
        &self,
        translation_code: &str,
        reference: &BibleReference,
    ) -> anyhow::Result<Option<BiblePassage>> {
        if given_code(reference.book_code.as_deref()).is_some() {
            return self
                .repository
                .find_bible_passage(translation_code, reference)
                .await;
        }
        let mut wanted = reference.clone();
        wanted.book_code = None;
        let stored = self
            .repository
            .find_bible_passage(translation_code, &wanted)
            .await?;
        if stored.is_some() {
            return Ok(stored);
        }
        let Some(candidate) = resolve_book_name(&reference.book) else {
            return Ok(None);
        };
        wanted.book_code = Some(candidate.code.clone());
        let found = self
            .repository
            .find_bible_passage(translation_code, &wanted)
            .await?;
        if found.is_some() {
            log_typed_book(&reference.book, &candidate);
        }
        Ok(found)
    }

    /// Books + chapter verse counts of a translation (the book picker).
    pub async fn list_bible_books(
        &self,
        translation_code: &str,
    ) -> anyhow::Result<Vec<BibleBookChapterSummary>> {
        self.bible_book_chapter_summaries(translation_code).await
    }

    /// Per-chapter summaries of a translation; a remote translation answers
    /// with its structure translation's (`eng-kjv`, with the NLT's longer
    /// chapters applied) — no request is made.
    pub async fn bible_book_chapter_summaries(
        &self,
        translation_code: &str,
    ) -> anyhow::Result<Vec<BibleBookChapterSummary>> {
        let summaries = self
            .repository
            .bible_book_chapter_summaries(bible_remote::structure_source(translation_code))
            .await?;
        if bible_remote::is_remote_translation(translation_code) {
            return Ok(bible_remote::nlt_structure(summaries));
        }
        Ok(summaries)
    }

    async fn remote_passage_range(
        &self,
        book: &str,
        book_code: Option<&str>,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
    ) -> anyhow::Result<Vec<BiblePassage>> {
        let Some(canon) = canonical_book(book, book_code) else {
            tracing::debug!(book, ?book_code, "NLT: unknown book — no verses");
            return Ok(Vec::new());
        };
        let verses = self
            .bible
            .nlt
            .fetch_verses(canon.code, chapter, verse_start, verse_end)
            .await?;
        if verses.is_empty() {
            return Ok(Vec::new());
        }
        let book_name = self.structure_book_name(canon).await;
        Ok(bible_remote::nlt_passages(&verses, &book_name, canon))
    }

    /// The name `eng-kjv` gives this book ("Psalms", "Song of Songs",
    /// "1 John"): the book picker shows eng-kjv's books for the NLT, and the
    /// #824 secondary reference label takes its book name from these
    /// passages — so a SEB + NLT slide reads `1 John 1:1-3 (NLT)`. Falls back
    /// to the canonical English name when eng-kjv cannot be read.
    async fn structure_book_name(&self, canon: BibleBookCanonical) -> String {
        let rows = self
            .repository
            .bible_passage_range(
                bible_remote::STRUCTURE_TRANSLATION_CODE,
                canon.english_name,
                Some(canon.code),
                1,
                1,
                1,
            )
            .await;
        let name = match rows {
            Ok(rows) => rows.into_iter().next().map(|row| row.reference.book),
            Err(err) => {
                tracing::warn!(
                    book_code = canon.code,
                    error = %err,
                    "NLT: eng-kjv book name unreadable — using the canonical name"
                );
                None
            }
        };
        name.unwrap_or_else(|| canon.english_name.to_string())
    }

    /// Point the NLT client at a mock API.
    #[cfg(test)]
    pub(crate) fn set_test_nlt_client(&mut self, client: crate::bible_remote::NltClient) {
        self.bible.nlt = std::sync::Arc::new(client);
    }
}

/// A book code that says something; a blank one counts as none.
fn given_code(book_code: Option<&str>) -> Option<&str> {
    book_code.map(str::trim).filter(|code| !code.is_empty())
}

/// One line when a typed name was not the translation's own (`zid` → HEB).
fn log_typed_book(name: &str, candidate: &BookCandidate) {
    let name: String = name.chars().take(TYPED_NAME_MAX_CHARS).collect();
    tracing::info!(
        target: "presenter::bible::name",
        name = name.as_str(),
        code = candidate.code.as_str(),
        r#match = ?candidate.kind,
        "bible book name resolved"
    );
}

/// The canonical book of a code (`"1JN"`) or, without a code, of whatever a
/// person calls it (`"1 John"`, `"1 Ján"`, `"zid"`).
pub(super) fn canonical_book(book: &str, book_code: Option<&str>) -> Option<BibleBookCanonical> {
    match given_code(book_code) {
        Some(code) => canonical_book_by_code(code),
        None => canonical_book_by_code(&resolve_book_name(book)?.code),
    }
}

#[cfg(test)]
mod tests;
