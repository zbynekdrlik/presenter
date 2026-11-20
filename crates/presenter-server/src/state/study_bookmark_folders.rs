//! Bookmark folders of the study page (#952, owner 2026-11-20: "viacej
//! zalozkovych zloziek, ktore si mozem samostatne vyberat"; folders, not tags).
//! The default folder "Kde som skončil" is no row: it is the list of bookmarks
//! with no folder, the one #915/#951 always had. The reader's own folders sit
//! beside it, each its own newest-first list; a folder is renamed in place and
//! deleted WITH its bookmarks (the owner's choice, the page asks first).
//!
//! The owner scope is the bookmarks' (`AccessEmail`, `None` = the LAN list).
//! The rules live here (`BookmarkRefusal` → 422); the repository refuses a
//! name in use (409) and another owner's folder (404).

use presenter_persistence::StudyBookmarkFolderRecord;

use super::study_bookmarks::{BookmarkRefusal, LOG_TARGET};
use super::AppState;

/// The default folder's name, reserved: no own folder may take it.
pub(crate) const DEFAULT_FOLDER_NAME: &str = "Kde som skončil";
/// The longest folder name, in characters (a chip on a phone).
pub(crate) const MAX_FOLDER_NAME_CHARS: usize = 40;
/// The most folders one reader keeps.
pub(crate) const MAX_FOLDERS: usize = 30;

/// A folder as the API shows it; `count` = its bookmarks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BookmarkFolder {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) count: usize,
}

impl From<StudyBookmarkFolderRecord> for BookmarkFolder {
    fn from(row: StudyBookmarkFolderRecord) -> Self {
        Self {
            id: row.id,
            name: row.name,
            // A COUNT(*) is never negative.
            count: usize::try_from(row.count).unwrap_or(0),
        }
    }
}

/// The trimmed folder name, or the refusal.
pub(crate) fn check_folder_name(name: &str) -> Result<String, BookmarkRefusal> {
    let name = name.trim();
    if name.is_empty() {
        return Err(BookmarkRefusal("Priečinok potrebuje názov".to_string()));
    }
    if name.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err(BookmarkRefusal(format!(
            "Názov priečinka môže mať najviac {MAX_FOLDER_NAME_CHARS} znakov"
        )));
    }
    if name.to_lowercase() == DEFAULT_FOLDER_NAME.to_lowercase() {
        return Err(BookmarkRefusal(format!(
            "„{DEFAULT_FOLDER_NAME}“ je predvolený priečinok — zvoľ iný názov"
        )));
    }
    Ok(name.to_string())
}

impl AppState {
    pub(crate) async fn list_study_bookmark_folders(
        &self,
        owner: Option<&str>,
    ) -> anyhow::Result<Vec<BookmarkFolder>> {
        Ok(self
            .repository
            .list_study_bookmark_folders(owner)
            .await?
            .into_iter()
            .map(BookmarkFolder::from)
            .collect())
    }

    /// A new folder at the end of the reader's list.
    pub(crate) async fn create_study_bookmark_folder(
        &self,
        owner: Option<&str>,
        name: &str,
    ) -> anyhow::Result<BookmarkFolder> {
        let name = check_folder_name(name)?;
        if self
            .repository
            .list_study_bookmark_folders(owner)
            .await?
            .len()
            >= MAX_FOLDERS
        {
            return Err(BookmarkRefusal(format!(
                "Priečinkov môže byť najviac {MAX_FOLDERS} — niektorý zmaž"
            ))
            .into());
        }
        let row = self
            .repository
            .insert_study_bookmark_folder(owner, &name)
            .await?;
        log_folder("create", owner, &row);
        Ok(row.into())
    }

    pub(crate) async fn rename_study_bookmark_folder(
        &self,
        owner: Option<&str>,
        id: i64,
        name: &str,
    ) -> anyhow::Result<BookmarkFolder> {
        let name = check_folder_name(name)?;
        let row = self
            .repository
            .rename_study_bookmark_folder(owner, id, &name)
            .await?;
        log_folder("rename", owner, &row);
        Ok(row.into())
    }

    /// Deletes the folder and its bookmarks; answers the folder as it was
    /// (its `count` = the bookmarks gone with it).
    pub(crate) async fn delete_study_bookmark_folder(
        &self,
        owner: Option<&str>,
        id: i64,
    ) -> anyhow::Result<BookmarkFolder> {
        let row = self
            .repository
            .delete_study_bookmark_folder(owner, id)
            .await?;
        log_folder("delete", owner, &row);
        Ok(row.into())
    }

    /// The folders in a new order, the whole list (a stale set is 409).
    pub(crate) async fn reorder_study_bookmark_folders(
        &self,
        owner: Option<&str>,
        ids: &[i64],
    ) -> anyhow::Result<()> {
        self.repository
            .reorder_study_bookmark_folders(owner, ids)
            .await?;
        tracing::info!(
            target: LOG_TARGET,
            op = "folder-reorder",
            owner = ?owner.unwrap_or("-"),
            ids = ?ids,
            "study bookmark folder write"
        );
        Ok(())
    }
}

/// One INFO line per folder write. The owner and the name are client input: quoted.
fn log_folder(op: &str, owner: Option<&str>, row: &StudyBookmarkFolderRecord) {
    tracing::info!(
        target: LOG_TARGET,
        op,
        id = row.id,
        owner = ?owner.unwrap_or("-"),
        name = ?row.name,
        bookmarks = row.count,
        "study bookmark folder write"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::study_bookmarks::PlaceRequest;
    use presenter_core::bible::BibleIngestionBatch;
    use presenter_core::{BiblePassage, BibleReference, BibleTranslation};
    use presenter_persistence::RepositoryError;

    async fn state() -> AppState {
        let state = AppState::in_memory().await.expect("state");
        let seb = BibleTranslation::new("slk-seb", "Slovenský ekumenický preklad", "sk");
        let passages = (1..=3)
            .map(|verse| {
                let reference =
                    BibleReference::new_with_code("Hebrejom", "HEB", 58, 11, verse, verse)
                        .expect("reference");
                BiblePassage::new(reference, seb.clone(), format!("verš {verse}"))
            })
            .collect();
        let batch = BibleIngestionBatch::new(seb, passages).expect("batch");
        state
            .repository()
            .replace_bible_translation_passages(&batch)
            .await
            .expect("seed");
        state
    }

    fn heb(verse: u16) -> PlaceRequest {
        PlaceRequest {
            translation: "slk-seb".to_string(),
            book_code: "HEB".to_string(),
            chapter: 11,
            verse,
        }
    }

    fn refused(err: &anyhow::Error) -> Option<String> {
        err.downcast_ref::<BookmarkRefusal>()
            .map(|refusal| refusal.0.clone())
    }

    #[test]
    fn a_folder_name_is_trimmed_bounded_and_never_the_default_one() {
        assert_eq!(check_folder_name("  Viera ").as_deref(), Ok("Viera"));
        assert_eq!(
            check_folder_name("   ").map_err(|e| e.0),
            Err("Priečinok potrebuje názov".to_string())
        );
        let fits = "ž".repeat(MAX_FOLDER_NAME_CHARS);
        assert_eq!(check_folder_name(&fits).as_deref(), Ok(fits.as_str()));
        let long = "ž".repeat(MAX_FOLDER_NAME_CHARS + 1);
        assert!(check_folder_name(&long).is_err());
        let reserved = check_folder_name(" kde som SKONČIL ").unwrap_err();
        assert!(
            reserved.0.contains("predvolený priečinok"),
            "{}",
            reserved.0
        );
    }

    #[tokio::test]
    async fn folders_hold_their_own_bookmarks_and_a_delete_takes_them() {
        let state = state().await;
        let me = Some("preacher@example.org");
        let viera = state
            .create_study_bookmark_folder(me, "  Viera ")
            .await
            .expect("folder");
        assert_eq!(viera.name, "Viera");
        state
            .create_study_bookmark(me, None, "Hebrejom 11:1", &heb(1), None)
            .await
            .expect("default");
        state
            .create_study_bookmark(me, Some(viera.id), "Hebrejom 11:2", &heb(2), None)
            .await
            .expect("in folder");
        let list = state
            .list_study_bookmarks(me, Some(viera.id))
            .await
            .expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Hebrejom 11:2");
        assert_eq!(list[0].text.as_deref(), Some("verš 2"));
        let folders = state
            .list_study_bookmark_folders(me)
            .await
            .expect("folders");
        assert_eq!(
            folders,
            vec![BookmarkFolder {
                id: viera.id,
                name: "Viera".to_string(),
                count: 1
            }]
        );
        let renamed = state
            .rename_study_bookmark_folder(me, viera.id, " Viera a nádej ")
            .await
            .expect("rename");
        assert_eq!(renamed.name, "Viera a nádej");
        let gone = state
            .delete_study_bookmark_folder(me, viera.id)
            .await
            .expect("delete");
        assert_eq!(gone.count, 1);
        assert!(state
            .list_study_bookmark_folders(me)
            .await
            .expect("folders")
            .is_empty());
        let default = state.list_study_bookmarks(me, None).await.expect("default");
        assert_eq!(default.len(), 1, "the default folder is untouched");
    }

    #[tokio::test]
    async fn a_name_in_use_is_409_and_the_limit_is_422() {
        let state = state().await;
        state
            .create_study_bookmark_folder(None, "Viera")
            .await
            .expect("folder");
        let err = state
            .create_study_bookmark_folder(None, "viera")
            .await
            .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::Conflict(_))
        ));
        let err = state
            .create_study_bookmark_folder(None, "Kde som skončil")
            .await
            .unwrap_err();
        assert!(refused(&err).is_some());
        for index in 1..MAX_FOLDERS {
            state
                .create_study_bookmark_folder(None, &format!("T {index}"))
                .await
                .expect("under the limit");
        }
        let err = state
            .create_study_bookmark_folder(None, "Navyše")
            .await
            .unwrap_err();
        assert_eq!(
            refused(&err),
            Some(format!(
                "Priečinkov môže byť najviac {MAX_FOLDERS} — niektorý zmaž"
            ))
        );
    }
}
