//! The study page's bookmarks (#915, the owner's option 2 of 2026-11-08;
//! reshaped by #951 into "where did I stop": newest first, the verse's text in
//! the list, a drag order, a bulk delete, no rename and no move): a list per
//! reader, kept on the server so the phone and the PC show the same one. The reader is the Cloudflare Access e-mail (`None` = the shared list of
//! the LAN). That e-mail picks WHICH list a request reads and writes — a
//! convenience scope for one preacher's devices, not a protection of the
//! rows (`access_identity.rs`). Every rule lives here; the repository only stores and orders.
//! Lookups and `study.last` never touch a bookmark: only these writes do.

use presenter_core::bible::canonical_book_by_code;

use crate::bible_remote::structure_source;
use presenter_core::bible::sermon::translation_short_code;
use presenter_persistence::{BookmarkPlace, DeletedStudyBookmark, StudyBookmarkRecord};

use super::AppState;

/// The longest name, in characters.
pub(crate) const MAX_NAME_CHARS: usize = 80;
/// The most bookmarks one list keeps.
pub(crate) const MAX_BOOKMARKS: usize = 100;
/// The most characters of a verse a list row carries (#951): the row shows one
/// ellipsized line, so the rest would only travel.
pub(crate) const EXCERPT_CHARS: usize = 160;

const LOG_TARGET: &str = "presenter::bible::bookmark";

/// A bookmark request the server refuses with 422 and this Slovak message (no
/// name, a name too long, a full list, a place the translation does not have).
/// Decided here, so its own type (repository-error-pattern rule);
/// `router/bible/bookmarks.rs` maps it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct BookmarkRefusal(pub(crate) String);

/// A place as a request names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlaceRequest {
    pub(crate) translation: String,
    pub(crate) book_code: String,
    pub(crate) chapter: u16,
    pub(crate) verse: u16,
}

/// A stored bookmark as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StudyBookmark {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) translation: String,
    pub(crate) book_code: String,
    pub(crate) book: String,
    pub(crate) chapter: u16,
    pub(crate) verse: u16,
    pub(crate) updated_at: String,
    /// The start of the verse's text (#951), only in the list.
    pub(crate) text: Option<String>,
}

impl TryFrom<StudyBookmarkRecord> for StudyBookmark {
    type Error = anyhow::Error;

    /// A stored place that is no `u16` is a damaged row: an error, never verse 0.
    fn try_from(row: StudyBookmarkRecord) -> anyhow::Result<Self> {
        let place = |value: i64, what: &str| {
            u16::try_from(value)
                .map_err(|_| anyhow::anyhow!("bookmark {}: stored {what} {value}", row.id))
        };
        Ok(Self {
            id: row.id,
            chapter: place(row.chapter, "chapter")?,
            verse: place(row.verse, "verse")?,
            name: row.name,
            translation: row.translation_code,
            book_code: row.book_code,
            book: row.book_name,
            updated_at: row.updated_at,
            text: row.text.as_deref().map(excerpt),
        })
    }
}

/// The first [`EXCERPT_CHARS`] characters of a verse, with "…" when cut.
pub(crate) fn excerpt(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(EXCERPT_CHARS) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text.to_string(),
    }
}

/// The trimmed name, or the refusal.
pub(crate) fn check_name(name: &str) -> Result<String, BookmarkRefusal> {
    let name = name.trim();
    if name.is_empty() {
        return Err(BookmarkRefusal("Záložka potrebuje názov".to_string()));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(BookmarkRefusal(format!(
            "Názov záložky môže mať najviac {MAX_NAME_CHARS} znakov"
        )));
    }
    Ok(name.to_string())
}

impl AppState {
    pub(crate) async fn list_study_bookmarks(
        &self,
        owner: Option<&str>,
    ) -> anyhow::Result<Vec<StudyBookmark>> {
        // A row's text: its own translation, else the main one; the remote NLT
        // has no text in the database, so it borrows its structure source's
        // (the KJV), as a bookmark of its own does through the main one.
        let main = self.get_bible_preferences().await?.main_translation;
        let rows = self
            .repository
            .list_study_bookmarks(owner, main.as_deref().map(structure_source))
            .await?;
        rows.into_iter().map(StudyBookmark::try_from).collect()
    }

    /// A new bookmark on TOP of the list, or at `index` (top-first; Späť after
    /// a delete puts it back where it stood).
    pub(crate) async fn create_study_bookmark(
        &self,
        owner: Option<&str>,
        name: &str,
        place: &PlaceRequest,
        index: Option<usize>,
    ) -> anyhow::Result<StudyBookmark> {
        let name = check_name(name)?;
        let place = self.bookmark_place(place).await?;
        if self
            .repository
            .list_study_bookmarks(owner, None)
            .await?
            .len()
            >= MAX_BOOKMARKS
        {
            return Err(BookmarkRefusal(format!(
                "Záložiek môže byť najviac {MAX_BOOKMARKS} — niektorú zmaž"
            ))
            .into());
        }
        let row = self
            .repository
            .insert_study_bookmark(owner, &name, &place, index)
            .await?;
        log_write("create", owner, &row);
        row.try_into()
    }

    /// The list in a new order: `ids` top-first, the whole list (a stale set is
    /// the repository's 409).
    pub(crate) async fn reorder_study_bookmarks(
        &self,
        owner: Option<&str>,
        ids: &[i64],
    ) -> anyhow::Result<()> {
        self.repository.reorder_study_bookmarks(owner, ids).await?;
        tracing::info!(
            target: LOG_TARGET,
            op = "reorder",
            owner = ?owner.unwrap_or("-"),
            ids = ?ids,
            "study bookmark write"
        );
        Ok(())
    }

    /// Deletes the chosen ones at once (#951 "Vybrať" → "Zmazať (N)"); answers
    /// each with its index in the list as it was, ascending, for Späť.
    pub(crate) async fn delete_study_bookmarks(
        &self,
        owner: Option<&str>,
        ids: &[i64],
    ) -> anyhow::Result<Vec<(StudyBookmark, usize)>> {
        if ids.is_empty() {
            return Err(BookmarkRefusal("Nie je čo zmazať".to_string()).into());
        }
        let deleted = self.repository.delete_study_bookmarks(owner, ids).await?;
        deleted
            .into_iter()
            .map(|DeletedStudyBookmark { row, index }| {
                log_write("delete", owner, &row);
                Ok((row.try_into()?, index))
            })
            .collect()
    }

    /// A place this translation really has, with its own name of the book. The
    /// NLT is checked against its structure (the KJV's), so no request is made.
    async fn bookmark_place(&self, place: &PlaceRequest) -> anyhow::Result<BookmarkPlace> {
        let translation = self
            .list_bible_translations()
            .await?
            .into_iter()
            .find(|candidate| {
                candidate
                    .code
                    .eq_ignore_ascii_case(place.translation.trim())
            })
            .ok_or_else(|| {
                BookmarkRefusal(format!(
                    "Preklad {} nie je nainštalovaný",
                    place.translation
                ))
            })?;
        let canon = canonical_book_by_code(&place.book_code)
            .ok_or_else(|| BookmarkRefusal(format!("Neznáma kniha {}", place.book_code)))?;
        let book: Vec<_> = self
            .bible_book_chapter_summaries(&translation.code)
            .await?
            .into_iter()
            .filter(|summary| summary.book_code.as_deref() == Some(canon.code))
            .collect();
        // The refusal names the book as this translation does, when it has it.
        let book_name = book
            .first()
            .map_or_else(|| canon.code.to_string(), |summary| summary.book.clone());
        let chapter = book
            .into_iter()
            .find(|summary| summary.chapter == place.chapter);
        match chapter {
            Some(summary) if place.verse >= 1 && place.verse <= summary.verse_count => {
                Ok(BookmarkPlace {
                    translation_code: translation.code,
                    book_code: canon.code.to_string(),
                    book_name: summary.book,
                    chapter: place.chapter,
                    verse: place.verse,
                })
            }
            _ => Err(BookmarkRefusal(format!(
                "{} {}:{} v preklade {} nie je",
                book_name,
                place.chapter,
                place.verse,
                translation_short_code(&translation.code)
            ))
            .into()),
        }
    }
}

/// One INFO line per write. The owner and the name are client input: quoted.
fn log_write(op: &str, owner: Option<&str>, row: &StudyBookmarkRecord) {
    tracing::info!(
        target: LOG_TARGET,
        op,
        id = row.id,
        owner = ?owner.unwrap_or("-"),
        name = ?row.name,
        place = %format!("{} {} {}:{}", row.translation_code, row.book_code, row.chapter, row.verse),
        "study bookmark write"
    );
}

#[cfg(test)]
mod tests;
