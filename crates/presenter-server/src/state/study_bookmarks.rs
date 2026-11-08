//! The study page's named bookmarks (#915, the owner's option 2 of 2026-11-08):
//! a list per reader, kept on the server so the phone and the PC show the same
//! one. The reader is the Cloudflare Access e-mail (`None` = the shared list of
//! the LAN). Every rule lives here; the repository only stores and orders.
//! Lookups and `study.last` never touch a bookmark: only these writes do.

use presenter_core::bible::canonical_book_by_code;
use presenter_core::bible::sermon::translation_short_code;
use presenter_persistence::{BookmarkPlace, StudyBookmarkRecord};

use super::AppState;

/// The longest name, in characters.
pub(crate) const MAX_NAME_CHARS: usize = 80;
/// The most bookmarks one list keeps.
pub(crate) const MAX_BOOKMARKS: usize = 100;

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
}

impl From<StudyBookmarkRecord> for StudyBookmark {
    fn from(row: StudyBookmarkRecord) -> Self {
        Self {
            id: row.id,
            name: row.name,
            translation: row.translation_code,
            book_code: row.book_code,
            book: row.book_name,
            chapter: u16::try_from(row.chapter).unwrap_or(0),
            verse: u16::try_from(row.verse).unwrap_or(0),
            updated_at: row.updated_at,
        }
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
        let rows = self.repository.list_study_bookmarks(owner).await?;
        Ok(rows.into_iter().map(StudyBookmark::from).collect())
    }

    /// A new bookmark at the end of the list, or at `index` (Späť after a
    /// delete puts it back where it stood).
    pub(crate) async fn create_study_bookmark(
        &self,
        owner: Option<&str>,
        name: &str,
        place: &PlaceRequest,
        index: Option<usize>,
    ) -> anyhow::Result<StudyBookmark> {
        let name = check_name(name)?;
        let place = self.bookmark_place(place).await?;
        if self.repository.list_study_bookmarks(owner).await?.len() >= MAX_BOOKMARKS {
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
        Ok(row.into())
    }

    /// A new name, a new place ("Presunúť sem"), or both.
    pub(crate) async fn update_study_bookmark(
        &self,
        owner: Option<&str>,
        id: i64,
        name: Option<&str>,
        place: Option<&PlaceRequest>,
    ) -> anyhow::Result<StudyBookmark> {
        if name.is_none() && place.is_none() {
            return Err(BookmarkRefusal("Nie je čo zmeniť".to_string()).into());
        }
        let name = name.map(check_name).transpose()?;
        let place = match place {
            Some(place) => Some(self.bookmark_place(place).await?),
            None => None,
        };
        let row = self
            .repository
            .update_study_bookmark(owner, id, name.as_deref(), place.as_ref())
            .await?;
        log_write(if place.is_some() { "move" } else { "rename" }, owner, &row);
        Ok(row.into())
    }

    /// Deletes one; answers it with its index, for Späť.
    pub(crate) async fn delete_study_bookmark(
        &self,
        owner: Option<&str>,
        id: i64,
    ) -> anyhow::Result<(StudyBookmark, usize)> {
        let row = self.repository.delete_study_bookmark(owner, id).await?;
        log_write("delete", owner, &row);
        let index = usize::try_from(row.position).unwrap_or(0);
        Ok((row.into(), index))
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
        let chapter = self
            .bible_book_chapter_summaries(&translation.code)
            .await?
            .into_iter()
            .find(|summary| {
                summary.book_code.as_deref() == Some(canon.code) && summary.chapter == place.chapter
            });
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
                canon.code,
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
