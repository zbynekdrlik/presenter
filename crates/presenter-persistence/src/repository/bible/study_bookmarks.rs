//! The study page's named bookmarks (#915): plain rows of `study_bookmarks`,
//! one list per owner (`owner_email`, NULL = the shared list of the LAN). Every
//! rule (a name, a real place, the limit) is checked by the caller; this layer
//! only keeps the order and answers an unknown id — or another owner's — with
//! `NotFound` (404), so one list never reveals another's rows.

use anyhow::Context;
use chrono::Utc;
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, FromQueryResult, Statement, TransactionTrait, Value,
};
use tracing::instrument;

use super::super::util::RepositoryError;
use super::super::Repository;

/// The refusal of an id that is not in the caller's list.
pub const BOOKMARK_NOT_FOUND: &str = "Záložka neexistuje";

/// A place in a translation, as a bookmark keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkPlace {
    pub translation_code: String,
    pub book_code: String,
    pub book_name: String,
    pub chapter: u16,
    pub verse: u16,
}

/// One stored bookmark. `position` orders the owner's list from 0.
#[derive(Debug, Clone, PartialEq, Eq, FromQueryResult)]
pub struct StudyBookmarkRecord {
    pub id: i64,
    pub owner_email: Option<String>,
    pub name: String,
    pub translation_code: String,
    pub book_code: String,
    pub book_name: String,
    pub chapter: i64,
    pub verse: i64,
    pub position: i64,
    pub created_at: String,
    pub updated_at: String,
}

const COLUMNS: &str = "id, owner_email, name, translation_code, book_code, book_name, chapter, \
     verse, position, created_at, updated_at";

fn owner_value(owner: Option<&str>) -> Value {
    owner.map(str::to_string).into()
}

impl Repository {
    /// The owner's list in its order.
    #[instrument(skip_all)]
    pub async fn list_study_bookmarks(
        &self,
        owner: Option<&str>,
    ) -> anyhow::Result<Vec<StudyBookmarkRecord>> {
        list_in(&self.db, owner).await
    }

    /// A new bookmark at `index` of the owner's list (the end when `None` or
    /// past it); the list is numbered again from 0 in the same transaction.
    #[instrument(skip_all)]
    pub async fn insert_study_bookmark(
        &self,
        owner: Option<&str>,
        name: &str,
        place: &BookmarkPlace,
        index: Option<usize>,
    ) -> anyhow::Result<StudyBookmarkRecord> {
        let backend = self.db.get_database_backend();
        let txn = self.db.begin().await.context("bookmark: begin")?;
        let now = Utc::now().to_rfc3339();
        let inserted = txn
            .execute(Statement::from_sql_and_values(
                backend,
                "INSERT INTO study_bookmarks (owner_email, name, translation_code, book_code, \
                 book_name, chapter, verse, position, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, -1, ?, ?)",
                [
                    owner_value(owner),
                    name.into(),
                    place.translation_code.clone().into(),
                    place.book_code.clone().into(),
                    place.book_name.clone().into(),
                    i64::from(place.chapter).into(),
                    i64::from(place.verse).into(),
                    now.clone().into(),
                    now.into(),
                ],
            ))
            .await
            .context("bookmark: insert")?;
        let id = i64::try_from(inserted.last_insert_id()).context("bookmark: id")?;
        let mut ids: Vec<i64> = list_in(&txn, owner)
            .await?
            .into_iter()
            .map(|row| row.id)
            .filter(|row_id| *row_id != id)
            .collect();
        let at = index.unwrap_or(ids.len()).min(ids.len());
        ids.insert(at, id);
        renumber(&txn, &ids).await?;
        let row = find_in(&txn, owner, id).await?;
        txn.commit().await.context("bookmark: commit")?;
        Ok(row)
    }

    /// A new name and/or a new place for one of the owner's bookmarks.
    #[instrument(skip_all)]
    pub async fn update_study_bookmark(
        &self,
        owner: Option<&str>,
        id: i64,
        name: Option<&str>,
        place: Option<&BookmarkPlace>,
    ) -> anyhow::Result<StudyBookmarkRecord> {
        let backend = self.db.get_database_backend();
        let txn = self.db.begin().await.context("bookmark: begin")?;
        let now = Utc::now().to_rfc3339();
        let changed = txn
            .execute(Statement::from_sql_and_values(
                backend,
                "UPDATE study_bookmarks SET updated_at = ?, name = COALESCE(?, name) \
                 WHERE id = ? AND owner_email IS ?",
                [
                    now.clone().into(),
                    name.map(str::to_string).into(),
                    id.into(),
                    owner_value(owner),
                ],
            ))
            .await
            .context("bookmark: rename")?;
        if changed.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(BOOKMARK_NOT_FOUND).into());
        }
        if let Some(place) = place {
            txn.execute(Statement::from_sql_and_values(
                backend,
                "UPDATE study_bookmarks SET translation_code = ?, book_code = ?, book_name = ?, \
                 chapter = ?, verse = ? WHERE id = ?",
                [
                    place.translation_code.clone().into(),
                    place.book_code.clone().into(),
                    place.book_name.clone().into(),
                    i64::from(place.chapter).into(),
                    i64::from(place.verse).into(),
                    id.into(),
                ],
            ))
            .await
            .context("bookmark: move")?;
        }
        let row = find_in(&txn, owner, id).await?;
        txn.commit().await.context("bookmark: commit")?;
        Ok(row)
    }

    /// Delete one of the owner's bookmarks; answers what was deleted (the page
    /// offers Späť with it).
    #[instrument(skip_all)]
    pub async fn delete_study_bookmark(
        &self,
        owner: Option<&str>,
        id: i64,
    ) -> anyhow::Result<StudyBookmarkRecord> {
        let backend = self.db.get_database_backend();
        let txn = self.db.begin().await.context("bookmark: begin")?;
        let row = find_in(&txn, owner, id).await?;
        txn.execute(Statement::from_sql_and_values(
            backend,
            "DELETE FROM study_bookmarks WHERE id = ?",
            [id.into()],
        ))
        .await
        .context("bookmark: delete")?;
        let ids: Vec<i64> = list_in(&txn, owner)
            .await?
            .into_iter()
            .map(|row| row.id)
            .collect();
        renumber(&txn, &ids).await?;
        txn.commit().await.context("bookmark: commit")?;
        Ok(row)
    }
}

async fn list_in(
    db: &impl ConnectionTrait,
    owner: Option<&str>,
) -> anyhow::Result<Vec<StudyBookmarkRecord>> {
    StudyBookmarkRecord::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        format!(
            "SELECT {COLUMNS} FROM study_bookmarks WHERE owner_email IS ? ORDER BY position, id"
        ),
        [owner_value(owner)],
    ))
    .all(db)
    .await
    .context("bookmark: list")
}

async fn find_in(
    db: &impl ConnectionTrait,
    owner: Option<&str>,
    id: i64,
) -> anyhow::Result<StudyBookmarkRecord> {
    let row = StudyBookmarkRecord::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        format!("SELECT {COLUMNS} FROM study_bookmarks WHERE id = ? AND owner_email IS ?"),
        [id.into(), owner_value(owner)],
    ))
    .one(db)
    .await
    .context("bookmark: read")?;
    Ok(row.ok_or(RepositoryError::NotFound(BOOKMARK_NOT_FOUND))?)
}

async fn renumber(txn: &DatabaseTransaction, ids: &[i64]) -> anyhow::Result<()> {
    let backend = txn.get_database_backend();
    for (position, id) in ids.iter().enumerate() {
        let position = i64::try_from(position).context("bookmark: position")?;
        txn.execute(Statement::from_sql_and_values(
            backend,
            "UPDATE study_bookmarks SET position = ? WHERE id = ?",
            [position.into(), (*id).into()],
        ))
        .await
        .context("bookmark: order")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(book: &str, chapter: u16, verse: u16) -> BookmarkPlace {
        BookmarkPlace {
            translation_code: "slk-seb".to_string(),
            book_code: book.to_string(),
            book_name: "Hebrejom".to_string(),
            chapter,
            verse,
        }
    }

    async fn repo() -> Repository {
        Repository::connect_in_memory().await.unwrap()
    }

    fn names(rows: &[StudyBookmarkRecord]) -> Vec<(&str, i64)> {
        rows.iter()
            .map(|row| (row.name.as_str(), row.position))
            .collect()
    }

    #[tokio::test]
    async fn a_new_bookmark_goes_to_the_end_or_to_its_index() {
        let repo = repo().await;
        let owner = Some("preacher@example.org");
        repo.insert_study_bookmark(owner, "Kázeň", &place("HEB", 11, 4), None)
            .await
            .unwrap();
        repo.insert_study_bookmark(owner, "Osobné", &place("JHN", 3, 16), None)
            .await
            .unwrap();
        repo.insert_study_bookmark(owner, "Prvá", &place("GEN", 1, 1), Some(0))
            .await
            .unwrap();
        repo.insert_study_bookmark(owner, "Ďaleko", &place("GEN", 1, 1), Some(99))
            .await
            .unwrap();
        let rows = repo.list_study_bookmarks(owner).await.unwrap();
        assert_eq!(
            names(&rows),
            vec![("Prvá", 0), ("Kázeň", 1), ("Osobné", 2), ("Ďaleko", 3)]
        );
        assert_eq!(rows[1].book_code, "HEB");
        assert_eq!((rows[1].chapter, rows[1].verse), (11, 4));
        assert_eq!(rows[1].owner_email.as_deref(), owner);
    }

    #[tokio::test]
    async fn each_owner_sees_only_its_own_list_and_the_lan_has_one_of_its_own() {
        let repo = repo().await;
        let mine = Some("preacher@example.org");
        let kept = repo
            .insert_study_bookmark(mine, "Moja", &place("HEB", 11, 4), None)
            .await
            .unwrap();
        repo.insert_study_bookmark(None, "Spoločná", &place("JHN", 1, 1), None)
            .await
            .unwrap();
        assert_eq!(
            names(&repo.list_study_bookmarks(mine).await.unwrap()),
            vec![("Moja", 0)]
        );
        assert_eq!(
            names(&repo.list_study_bookmarks(None).await.unwrap()),
            vec![("Spoločná", 0)]
        );
        assert!(repo
            .list_study_bookmarks(Some("other@example.org"))
            .await
            .unwrap()
            .is_empty());
        // Another owner's id is not found, in either direction.
        for err in [
            repo.update_study_bookmark(None, kept.id, Some("x"), None)
                .await
                .unwrap_err(),
            repo.delete_study_bookmark(Some("other@example.org"), kept.id)
                .await
                .unwrap_err(),
        ] {
            assert!(matches!(
                err.downcast_ref::<RepositoryError>(),
                Some(RepositoryError::NotFound(BOOKMARK_NOT_FOUND))
            ));
        }
        assert_eq!(
            names(&repo.list_study_bookmarks(mine).await.unwrap()),
            vec![("Moja", 0)]
        );
    }

    #[tokio::test]
    async fn a_rename_and_a_move_change_only_what_they_name() {
        let repo = repo().await;
        let row = repo
            .insert_study_bookmark(None, "Kázeň", &place("HEB", 11, 4), None)
            .await
            .unwrap();
        let renamed = repo
            .update_study_bookmark(None, row.id, Some("Nedeľa"), None)
            .await
            .unwrap();
        assert_eq!(renamed.name, "Nedeľa");
        assert_eq!((renamed.book_code.as_str(), renamed.verse), ("HEB", 4));
        let moved = repo
            .update_study_bookmark(None, row.id, None, Some(&place("ROM", 8, 28)))
            .await
            .unwrap();
        assert_eq!(moved.name, "Nedeľa");
        assert_eq!(
            (moved.book_code.as_str(), moved.chapter, moved.verse),
            ("ROM", 8, 28)
        );
        assert_eq!(moved.created_at, row.created_at);
    }

    #[tokio::test]
    async fn a_delete_answers_the_row_and_closes_the_gap() {
        let repo = repo().await;
        let first = repo
            .insert_study_bookmark(None, "A", &place("HEB", 11, 1), None)
            .await
            .unwrap();
        repo.insert_study_bookmark(None, "B", &place("HEB", 11, 2), None)
            .await
            .unwrap();
        let deleted = repo.delete_study_bookmark(None, first.id).await.unwrap();
        assert_eq!(deleted.name, "A");
        assert_eq!(
            names(&repo.list_study_bookmarks(None).await.unwrap()),
            vec![("B", 0)]
        );
        let again = repo
            .delete_study_bookmark(None, first.id)
            .await
            .unwrap_err();
        assert!(matches!(
            again.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::NotFound(_))
        ));
    }
}
