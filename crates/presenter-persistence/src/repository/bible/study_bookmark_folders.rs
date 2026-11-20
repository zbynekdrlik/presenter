//! The study page's bookmark folders (#952, owner 2026-11-20: folders, not
//! tags): the reader's own folders for thematic verses ("Viera", "Modlitba"),
//! one list per owner (`owner_email`, NULL = the shared list of the LAN, as the
//! bookmarks). The default folder "Kde som skončil" is NO row: it is
//! `study_bookmarks.folder_id IS NULL`, so the bookmarks stored before #952
//! stay where they are.
//!
//! The name rules (length, the reserved default name, the limit) are the
//! caller's; this layer keeps the order (ascending `position`, new folders at
//! the end), refuses a name the owner already uses (`Conflict`, 409, case
//! does not matter), answers another owner's id with `NotFound` (404), and
//! deletes a folder WITH its bookmarks in one transaction.

use std::collections::HashSet;

use anyhow::Context;
use chrono::Utc;
use sea_orm::{ConnectionTrait, FromQueryResult, Statement, TransactionTrait};
use tracing::instrument;

use super::super::util::RepositoryError;
use super::super::Repository;
use super::study_bookmarks::owner_value;

/// The refusal of a folder id that is not the caller's.
pub const FOLDER_NOT_FOUND: &str = "Priečinok neexistuje";

/// The refusal of a name the owner already gives another folder.
pub const FOLDER_NAME_TAKEN: &str = "Priečinok s týmto názvom už existuje";

/// The refusal of an order that is not the caller's whole folder list.
pub const FOLDER_ORDER_STALE: &str = "Priečinky sa medzitým zmenili — skús znova";

/// One stored folder; `count` = how many bookmarks it holds.
#[derive(Debug, Clone, PartialEq, Eq, FromQueryResult)]
pub struct StudyBookmarkFolderRecord {
    pub id: i64,
    pub owner_email: Option<String>,
    pub name: String,
    pub position: i64,
    pub created_at: String,
    pub updated_at: String,
    pub count: i64,
}

const SELECT_FOLDER: &str = "SELECT f.id, f.owner_email, f.name, f.position, f.created_at, \
     f.updated_at, (SELECT COUNT(*) FROM study_bookmarks b WHERE b.folder_id = f.id) AS count \
     FROM study_bookmark_folders f";

#[derive(FromQueryResult)]
struct IdName {
    id: i64,
    name: String,
}

/// `Ok` for the default folder (`None`) and for one of the owner's folders;
/// any other id is `NotFound`.
pub(super) async fn ensure_folder(
    db: &impl ConnectionTrait,
    owner: Option<&str>,
    folder: Option<i64>,
) -> anyhow::Result<()> {
    let Some(id) = folder else {
        return Ok(());
    };
    folders_in(db, owner)
        .await?
        .iter()
        .any(|held| held.id == id)
        .then_some(())
        .ok_or(RepositoryError::NotFound(FOLDER_NOT_FOUND).into())
}

/// The owner's folders (id + name), in their order.
async fn folders_in(db: &impl ConnectionTrait, owner: Option<&str>) -> anyhow::Result<Vec<IdName>> {
    IdName::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT id, name FROM study_bookmark_folders WHERE owner_email IS ? \
         ORDER BY position, id",
        [owner_value(owner)],
    ))
    .all(db)
    .await
    .context("bookmark folder: ids")
}

/// `Conflict` when another of the owner's folders (not `except`) has this
/// name, whatever its case.
fn ensure_free_name(held: &[IdName], name: &str, except: Option<i64>) -> anyhow::Result<()> {
    let wanted = name.to_lowercase();
    let taken = held
        .iter()
        .any(|folder| Some(folder.id) != except && folder.name.to_lowercase() == wanted);
    if taken {
        return Err(RepositoryError::Conflict(FOLDER_NAME_TAKEN).into());
    }
    Ok(())
}

async fn renumber(db: &impl ConnectionTrait, ids: &[i64]) -> anyhow::Result<()> {
    for (position, id) in ids.iter().enumerate() {
        let position = i64::try_from(position).context("bookmark folder: position")?;
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "UPDATE study_bookmark_folders SET position = ? WHERE id = ?",
            [position.into(), (*id).into()],
        ))
        .await
        .context("bookmark folder: order")?;
    }
    Ok(())
}

async fn find_folder(
    db: &impl ConnectionTrait,
    owner: Option<&str>,
    id: i64,
) -> anyhow::Result<StudyBookmarkFolderRecord> {
    let row = StudyBookmarkFolderRecord::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        format!("{SELECT_FOLDER} WHERE f.id = ? AND f.owner_email IS ?"),
        [id.into(), owner_value(owner)],
    ))
    .one(db)
    .await
    .context("bookmark folder: read")?;
    Ok(row.ok_or(RepositoryError::NotFound(FOLDER_NOT_FOUND))?)
}

impl Repository {
    /// The owner's folders in their order, each with its bookmark count.
    #[instrument(skip_all)]
    pub async fn list_study_bookmark_folders(
        &self,
        owner: Option<&str>,
    ) -> anyhow::Result<Vec<StudyBookmarkFolderRecord>> {
        StudyBookmarkFolderRecord::find_by_statement(Statement::from_sql_and_values(
            self.db.get_database_backend(),
            format!("{SELECT_FOLDER} WHERE f.owner_email IS ? ORDER BY f.position, f.id"),
            [owner_value(owner)],
        ))
        .all(&self.db)
        .await
        .context("bookmark folder: list")
    }

    /// A new folder at the end of the owner's list.
    #[instrument(skip_all)]
    pub async fn insert_study_bookmark_folder(
        &self,
        owner: Option<&str>,
        name: &str,
    ) -> anyhow::Result<StudyBookmarkFolderRecord> {
        let txn = self.db.begin().await.context("bookmark folder: begin")?;
        let held = folders_in(&txn, owner).await?;
        ensure_free_name(&held, name, None)?;
        let now = Utc::now().to_rfc3339();
        let position = i64::try_from(held.len()).context("bookmark folder: position")?;
        let inserted = txn
            .execute(Statement::from_sql_and_values(
                txn.get_database_backend(),
                "INSERT INTO study_bookmark_folders (owner_email, name, position, created_at, \
                 updated_at) VALUES (?, ?, ?, ?, ?)",
                [
                    owner_value(owner),
                    name.into(),
                    position.into(),
                    now.clone().into(),
                    now.into(),
                ],
            ))
            .await
            .context("bookmark folder: insert")?;
        let id = i64::try_from(inserted.last_insert_id()).context("bookmark folder: id")?;
        let row = find_folder(&txn, owner, id).await?;
        txn.commit().await.context("bookmark folder: commit")?;
        Ok(row)
    }

    /// A new name for one of the owner's folders.
    #[instrument(skip_all)]
    pub async fn rename_study_bookmark_folder(
        &self,
        owner: Option<&str>,
        id: i64,
        name: &str,
    ) -> anyhow::Result<StudyBookmarkFolderRecord> {
        let txn = self.db.begin().await.context("bookmark folder: begin")?;
        let held = folders_in(&txn, owner).await?;
        if !held.iter().any(|folder| folder.id == id) {
            return Err(RepositoryError::NotFound(FOLDER_NOT_FOUND).into());
        }
        ensure_free_name(&held, name, Some(id))?;
        txn.execute(Statement::from_sql_and_values(
            txn.get_database_backend(),
            "UPDATE study_bookmark_folders SET name = ?, updated_at = ? \
             WHERE id = ? AND owner_email IS ?",
            [
                name.into(),
                Utc::now().to_rfc3339().into(),
                id.into(),
                owner_value(owner),
            ],
        ))
        .await
        .context("bookmark folder: rename")?;
        let row = find_folder(&txn, owner, id).await?;
        txn.commit().await.context("bookmark folder: commit")?;
        Ok(row)
    }

    /// Deletes one of the owner's folders AND its bookmarks in one transaction
    /// (owner: "záložky v ňom zmiznú"); answers the folder as it was.
    #[instrument(skip_all)]
    pub async fn delete_study_bookmark_folder(
        &self,
        owner: Option<&str>,
        id: i64,
    ) -> anyhow::Result<StudyBookmarkFolderRecord> {
        let backend = self.db.get_database_backend();
        let txn = self.db.begin().await.context("bookmark folder: begin")?;
        let row = find_folder(&txn, owner, id).await?;
        txn.execute(Statement::from_sql_and_values(
            backend,
            "DELETE FROM study_bookmarks WHERE folder_id = ? AND owner_email IS ?",
            [id.into(), owner_value(owner)],
        ))
        .await
        .context("bookmark folder: delete its bookmarks")?;
        txn.execute(Statement::from_sql_and_values(
            backend,
            "DELETE FROM study_bookmark_folders WHERE id = ? AND owner_email IS ?",
            [id.into(), owner_value(owner)],
        ))
        .await
        .context("bookmark folder: delete")?;
        let kept: Vec<i64> = folders_in(&txn, owner)
            .await?
            .into_iter()
            .map(|folder| folder.id)
            .collect();
        renumber(&txn, &kept).await?;
        txn.commit().await.context("bookmark folder: commit")?;
        Ok(row)
    }

    /// The owner's folders in a new order: the WHOLE list once each, else
    /// `Conflict` (409, a stale view).
    #[instrument(skip_all)]
    pub async fn reorder_study_bookmark_folders(
        &self,
        owner: Option<&str>,
        ids: &[i64],
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await.context("bookmark folder: begin")?;
        let held: HashSet<i64> = folders_in(&txn, owner)
            .await?
            .into_iter()
            .map(|folder| folder.id)
            .collect();
        let asked: HashSet<i64> = ids.iter().copied().collect();
        if asked.len() != ids.len() || asked != held {
            return Err(RepositoryError::Conflict(FOLDER_ORDER_STALE).into());
        }
        renumber(&txn, ids).await?;
        txn.commit().await.context("bookmark folder: commit")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::bible::study_bookmarks::BookmarkPlace;

    const ME: Option<&str> = Some("preacher@example.org");
    const OTHER: Option<&str> = Some("other@example.org");

    fn place() -> BookmarkPlace {
        BookmarkPlace {
            translation_code: "slk-seb".to_string(),
            book_code: "HEB".to_string(),
            book_name: "Hebrejom".to_string(),
            chapter: 11,
            verse: 1,
        }
    }

    async fn repo() -> Repository {
        Repository::connect_in_memory().await.unwrap()
    }

    fn refusal(err: &anyhow::Error) -> Option<&RepositoryError> {
        err.downcast_ref::<RepositoryError>()
    }

    fn names(rows: &[StudyBookmarkFolderRecord]) -> Vec<&str> {
        rows.iter().map(|row| row.name.as_str()).collect()
    }

    #[tokio::test]
    async fn folders_keep_their_order_and_a_name_is_used_once_per_owner() {
        let repo = repo().await;
        let viera = repo
            .insert_study_bookmark_folder(ME, "Viera")
            .await
            .unwrap();
        repo.insert_study_bookmark_folder(ME, "Modlitba")
            .await
            .unwrap();
        let err = repo
            .insert_study_bookmark_folder(ME, "VIERA")
            .await
            .unwrap_err();
        assert!(matches!(
            refusal(&err),
            Some(RepositoryError::Conflict(FOLDER_NAME_TAKEN))
        ));
        // Another owner may use the same name; the LAN list is its own.
        repo.insert_study_bookmark_folder(OTHER, "Viera")
            .await
            .unwrap();
        let mine = repo.list_study_bookmark_folders(ME).await.unwrap();
        assert_eq!(names(&mine), vec!["Viera", "Modlitba"]);
        assert_eq!(mine[0].id, viera.id);
        assert_eq!(mine[0].count, 0);
        assert!(repo
            .list_study_bookmark_folders(None)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_bookmark_lives_in_one_folder_and_the_default_keeps_the_old_rows() {
        let repo = repo().await;
        let viera = repo
            .insert_study_bookmark_folder(ME, "Viera")
            .await
            .unwrap();
        repo.insert_study_bookmark(ME, None, "Domov", &place(), None)
            .await
            .unwrap();
        let made = repo
            .insert_study_bookmark(ME, Some(viera.id), "Téma", &place(), None)
            .await
            .unwrap();
        assert_eq!(made.folder_id, Some(viera.id));
        let default = repo.list_study_bookmarks(ME, None, None).await.unwrap();
        assert_eq!(default.len(), 1);
        assert_eq!(default[0].name, "Domov");
        assert_eq!(default[0].folder_id, None);
        let themed = repo
            .list_study_bookmarks(ME, Some(viera.id), None)
            .await
            .unwrap();
        assert_eq!(themed.len(), 1);
        assert_eq!(themed[0].name, "Téma");
        assert_eq!(
            repo.list_study_bookmark_folders(ME).await.unwrap()[0].count,
            1
        );
        // A bookmark of the folder is not one of the default folder's ids.
        let err = repo
            .delete_study_bookmarks(ME, None, &[made.id])
            .await
            .unwrap_err();
        assert!(matches!(refusal(&err), Some(RepositoryError::NotFound(_))));
        assert_eq!(
            repo.count_study_bookmarks(ME, Some(viera.id))
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn another_owners_folder_is_not_found_for_any_use() {
        let repo = repo().await;
        let theirs = repo
            .insert_study_bookmark_folder(OTHER, "Ich")
            .await
            .unwrap();
        let not_found = |err: anyhow::Error| {
            matches!(
                refusal(&err),
                Some(RepositoryError::NotFound(FOLDER_NOT_FOUND))
            )
        };
        assert!(not_found(
            repo.list_study_bookmarks(ME, Some(theirs.id), None)
                .await
                .unwrap_err()
        ));
        assert!(not_found(
            repo.insert_study_bookmark(ME, Some(theirs.id), "x", &place(), None)
                .await
                .unwrap_err()
        ));
        assert!(not_found(
            repo.rename_study_bookmark_folder(ME, theirs.id, "Moje")
                .await
                .unwrap_err()
        ));
        assert!(not_found(
            repo.delete_study_bookmark_folder(ME, theirs.id)
                .await
                .unwrap_err()
        ));
        assert!(not_found(
            repo.reorder_study_bookmarks(ME, Some(theirs.id), &[])
                .await
                .unwrap_err()
        ));
        assert_eq!(
            names(&repo.list_study_bookmark_folders(OTHER).await.unwrap()),
            vec!["Ich"]
        );
    }

    #[tokio::test]
    async fn a_rename_keeps_the_folder_and_refuses_a_name_in_use() {
        let repo = repo().await;
        let viera = repo
            .insert_study_bookmark_folder(ME, "Viera")
            .await
            .unwrap();
        repo.insert_study_bookmark_folder(ME, "Modlitba")
            .await
            .unwrap();
        let renamed = repo
            .rename_study_bookmark_folder(ME, viera.id, "Viera a skutky")
            .await
            .unwrap();
        assert_eq!(renamed.id, viera.id);
        assert_eq!(renamed.name, "Viera a skutky");
        // Its own name in another case is fine; another folder's is not.
        repo.rename_study_bookmark_folder(ME, viera.id, "VIERA A SKUTKY")
            .await
            .unwrap();
        let err = repo
            .rename_study_bookmark_folder(ME, viera.id, "modlitba")
            .await
            .unwrap_err();
        assert!(matches!(
            refusal(&err),
            Some(RepositoryError::Conflict(FOLDER_NAME_TAKEN))
        ));
    }

    #[tokio::test]
    async fn a_delete_takes_only_that_folder_and_its_bookmarks() {
        let repo = repo().await;
        let viera = repo
            .insert_study_bookmark_folder(ME, "Viera")
            .await
            .unwrap();
        let modlitba = repo
            .insert_study_bookmark_folder(ME, "Modlitba")
            .await
            .unwrap();
        let laska = repo
            .insert_study_bookmark_folder(ME, "Láska")
            .await
            .unwrap();
        for folder in [None, Some(viera.id), Some(viera.id), Some(modlitba.id)] {
            repo.insert_study_bookmark(ME, folder, "x", &place(), None)
                .await
                .unwrap();
        }
        // Another owner's bookmark in the LAN list stays.
        repo.insert_study_bookmark(None, None, "lan", &place(), None)
            .await
            .unwrap();
        let gone = repo
            .delete_study_bookmark_folder(ME, viera.id)
            .await
            .unwrap();
        assert_eq!((gone.name.as_str(), gone.count), ("Viera", 2));
        let left = repo.list_study_bookmark_folders(ME).await.unwrap();
        assert_eq!(names(&left), vec!["Modlitba", "Láska"]);
        assert_eq!(
            left.iter().map(|f| f.position).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(left[0].count, 1);
        assert_eq!(left[1].id, laska.id);
        assert_eq!(repo.count_study_bookmarks(ME, None).await.unwrap(), 1);
        assert_eq!(repo.count_study_bookmarks(None, None).await.unwrap(), 1);
        let err = repo
            .list_study_bookmarks(ME, Some(viera.id), None)
            .await
            .unwrap_err();
        assert!(matches!(refusal(&err), Some(RepositoryError::NotFound(_))));
    }

    #[tokio::test]
    async fn a_reorder_and_a_delete_inside_a_folder_touch_only_that_folder() {
        let repo = repo().await;
        let viera = repo
            .insert_study_bookmark_folder(ME, "Viera")
            .await
            .unwrap()
            .id;
        let other = repo
            .insert_study_bookmark_folder(ME, "Iné")
            .await
            .unwrap()
            .id;
        let mut ids = Vec::new();
        for name in ["A", "B", "C"] {
            let made = repo
                .insert_study_bookmark(ME, Some(viera), name, &place(), None)
                .await
                .unwrap();
            ids.push(made.id);
        }
        repo.insert_study_bookmark(ME, Some(other), "X", &place(), None)
            .await
            .unwrap();
        repo.insert_study_bookmark(ME, None, "D", &place(), None)
            .await
            .unwrap();
        let shown = |rows: Vec<crate::repository::StudyBookmarkRecord>| -> Vec<String> {
            rows.into_iter().map(|row| row.name).collect()
        };
        // Shown newest first: C B A. Reorder to A C B.
        repo.reorder_study_bookmarks(ME, Some(viera), &[ids[0], ids[2], ids[1]])
            .await
            .unwrap();
        assert_eq!(
            shown(
                repo.list_study_bookmarks(ME, Some(viera), None)
                    .await
                    .unwrap()
            ),
            vec!["A", "C", "B"]
        );
        // The default folder's or another folder's ids are a stale set here.
        let err = repo
            .reorder_study_bookmarks(ME, Some(other), &[ids[0], ids[2], ids[1]])
            .await
            .unwrap_err();
        assert!(matches!(refusal(&err), Some(RepositoryError::Conflict(_))));
        let deleted = repo
            .delete_study_bookmarks(ME, Some(viera), &[ids[2]])
            .await
            .unwrap();
        assert_eq!(
            deleted
                .iter()
                .map(|item| (item.row.name.as_str(), item.index, item.row.folder_id))
                .collect::<Vec<_>>(),
            vec![("C", 1, Some(viera))]
        );
        assert_eq!(
            shown(
                repo.list_study_bookmarks(ME, Some(viera), None)
                    .await
                    .unwrap()
            ),
            vec!["A", "B"]
        );
        // Späť: back into its folder at its old place.
        repo.insert_study_bookmark(ME, Some(viera), "C", &place(), Some(deleted[0].index))
            .await
            .unwrap();
        assert_eq!(
            shown(
                repo.list_study_bookmarks(ME, Some(viera), None)
                    .await
                    .unwrap()
            ),
            vec!["A", "C", "B"]
        );
        assert_eq!(
            shown(
                repo.list_study_bookmarks(ME, Some(other), None)
                    .await
                    .unwrap()
            ),
            vec!["X"]
        );
        assert_eq!(
            shown(repo.list_study_bookmarks(ME, None, None).await.unwrap()),
            vec!["D"]
        );
    }

    #[tokio::test]
    async fn a_folder_reorder_takes_the_whole_list_or_nothing() {
        let repo = repo().await;
        let a = repo.insert_study_bookmark_folder(ME, "A").await.unwrap().id;
        let b = repo.insert_study_bookmark_folder(ME, "B").await.unwrap().id;
        let c = repo.insert_study_bookmark_folder(ME, "C").await.unwrap().id;
        repo.reorder_study_bookmark_folders(ME, &[c, a, b])
            .await
            .unwrap();
        assert_eq!(
            names(&repo.list_study_bookmark_folders(ME).await.unwrap()),
            vec!["C", "A", "B"]
        );
        for stale in [vec![a, b], vec![a, b, c, c], vec![a, b, 999]] {
            let err = repo
                .reorder_study_bookmark_folders(ME, &stale)
                .await
                .unwrap_err();
            assert!(
                matches!(
                    refusal(&err),
                    Some(RepositoryError::Conflict(FOLDER_ORDER_STALE))
                ),
                "{stale:?}"
            );
        }
        assert_eq!(
            names(&repo.list_study_bookmark_folders(ME).await.unwrap()),
            vec!["C", "A", "B"]
        );
    }
}
