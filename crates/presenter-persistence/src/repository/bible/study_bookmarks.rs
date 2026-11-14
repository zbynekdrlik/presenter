//! The study page's bookmarks (#915, reshaped by #951): plain rows of
//! `study_bookmarks`, one list per owner (`owner_email`, NULL = the shared list
//! of the LAN). Every rule (a name, a real place, the limit) is checked by the
//! caller; this layer only keeps the order and answers an unknown id — or
//! another owner's — with `NotFound` (404), so one list never reveals another's
//! rows.
//!
//! **The order is shown NEWEST FIRST (#951) without touching a stored row:** the
//! stored `position` keeps its #915 meaning (ascending = older, so the rows
//! already on SNV, 0..n oldest to newest, show newest first as they are), and
//! every list and index this layer speaks is TOP-FIRST: `list` reads
//! `position DESC`, an index 0 is the top, `renumber` writes `len - 1 - i`.

use std::collections::HashSet;

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

/// The refusal of an order that is not the caller's whole list (another device
/// added or deleted one meanwhile): 409, the page reloads.
pub const BOOKMARK_ORDER_STALE: &str = "Záložky sa medzitým zmenili — skús znova";

/// A place in a translation, as a bookmark keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkPlace {
    pub translation_code: String,
    pub book_code: String,
    pub book_name: String,
    pub chapter: u16,
    pub verse: u16,
}

/// One stored bookmark. `position` orders the owner's list (ascending = older).
/// `text` = the verse's text, read only by the list (else `None`).
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
    pub text: Option<String>,
}

/// A deleted bookmark and its index in the list as it was (top-first): Späť
/// puts it back there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedStudyBookmark {
    pub row: StudyBookmarkRecord,
    pub index: usize,
}

const COLUMNS: &str = "id, owner_email, name, translation_code, book_code, book_name, chapter, \
     verse, position, created_at, updated_at";

/// The verse's text in translation `{t}` (a column or `?`), one passage, served
/// by `idx_bible_passages_place` (translation, book code, chapter, verse start).
fn verse_text(translation: &str) -> String {
    format!(
        "(SELECT p.content FROM bible_passages p WHERE p.translation_code = {translation} \
         AND p.book_code = b.book_code AND p.chapter = b.chapter \
         AND p.verse_start <= b.verse AND p.verse_end >= b.verse \
         ORDER BY p.verse_start DESC LIMIT 1)"
    )
}

fn owner_value(owner: Option<&str>) -> Value {
    owner.map(str::to_string).into()
}

impl Repository {
    /// The owner's list, newest first, each row with its verse's text: the
    /// bookmark's own translation, else `fallback_translation` (the main one;
    /// the remote NLT has no text here). One query.
    #[instrument(skip_all)]
    pub async fn list_study_bookmarks(
        &self,
        owner: Option<&str>,
        fallback_translation: Option<&str>,
    ) -> anyhow::Result<Vec<StudyBookmarkRecord>> {
        let sql = format!(
            "SELECT {COLUMNS}, COALESCE({}, {}) AS text FROM study_bookmarks b \
             WHERE owner_email IS ? ORDER BY position DESC, id DESC",
            verse_text("b.translation_code"),
            verse_text("?"),
        );
        StudyBookmarkRecord::find_by_statement(Statement::from_sql_and_values(
            self.db.get_database_backend(),
            sql,
            [
                fallback_translation.map(str::to_string).into(),
                owner_value(owner),
            ],
        ))
        .all(&self.db)
        .await
        .context("bookmark: list")
    }

    /// A new bookmark at `index` of the owner's list (top-first; the TOP when
    /// `None`, the end when past it); the list is numbered again in the same
    /// transaction.
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
        let mut ids: Vec<i64> = ids_in(&txn, owner)
            .await?
            .into_iter()
            .filter(|row_id| *row_id != id)
            .collect();
        let at = index.unwrap_or(0).min(ids.len());
        ids.insert(at, id);
        renumber(&txn, &ids).await?;
        let row = find_in(&txn, owner, id).await?;
        txn.commit().await.context("bookmark: commit")?;
        Ok(row)
    }

    /// The owner's list in a new order: `ids` top-first, the WHOLE list once
    /// each. Any other set is a stale view of the list → `Conflict` (409).
    #[instrument(skip_all)]
    pub async fn reorder_study_bookmarks(
        &self,
        owner: Option<&str>,
        ids: &[i64],
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await.context("bookmark: begin")?;
        let held: HashSet<i64> = ids_in(&txn, owner).await?.into_iter().collect();
        let asked: HashSet<i64> = ids.iter().copied().collect();
        if asked.len() != ids.len() || asked != held {
            return Err(RepositoryError::Conflict(BOOKMARK_ORDER_STALE).into());
        }
        renumber(&txn, ids).await?;
        txn.commit().await.context("bookmark: commit")?;
        Ok(())
    }

    /// Delete some of the owner's bookmarks in ONE transaction; answers each with
    /// its index in the list as it was (ascending), so Späť restores them in
    /// that order to their exact places. An id not in the list deletes nothing
    /// (`NotFound`, 404).
    #[instrument(skip_all)]
    pub async fn delete_study_bookmarks(
        &self,
        owner: Option<&str>,
        ids: &[i64],
    ) -> anyhow::Result<Vec<DeletedStudyBookmark>> {
        let backend = self.db.get_database_backend();
        let txn = self.db.begin().await.context("bookmark: begin")?;
        let held = ids_in(&txn, owner).await?;
        let gone: HashSet<i64> = ids.iter().copied().collect();
        let mut deleted = Vec::with_capacity(gone.len());
        for (index, id) in held.iter().enumerate() {
            if gone.contains(id) {
                let row = find_in(&txn, owner, *id).await?;
                deleted.push(DeletedStudyBookmark { row, index });
            }
        }
        if deleted.len() != gone.len() {
            return Err(RepositoryError::NotFound(BOOKMARK_NOT_FOUND).into());
        }
        for item in &deleted {
            txn.execute(Statement::from_sql_and_values(
                backend,
                "DELETE FROM study_bookmarks WHERE id = ?",
                [item.row.id.into()],
            ))
            .await
            .context("bookmark: delete")?;
        }
        let kept: Vec<i64> = held.into_iter().filter(|id| !gone.contains(id)).collect();
        renumber(&txn, &kept).await?;
        txn.commit().await.context("bookmark: commit")?;
        Ok(deleted)
    }
}

/// The owner's ids, top-first.
async fn ids_in(db: &impl ConnectionTrait, owner: Option<&str>) -> anyhow::Result<Vec<i64>> {
    #[derive(FromQueryResult)]
    struct IdRow {
        id: i64,
    }
    let rows = IdRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT id FROM study_bookmarks WHERE owner_email IS ? ORDER BY position DESC, id DESC",
        [owner_value(owner)],
    ))
    .all(db)
    .await
    .context("bookmark: ids")?;
    Ok(rows.into_iter().map(|row| row.id).collect())
}

async fn find_in(
    db: &impl ConnectionTrait,
    owner: Option<&str>,
    id: i64,
) -> anyhow::Result<StudyBookmarkRecord> {
    let row = StudyBookmarkRecord::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        format!(
            "SELECT {COLUMNS}, NULL AS text FROM study_bookmarks WHERE id = ? AND owner_email IS ?"
        ),
        [id.into(), owner_value(owner)],
    ))
    .one(db)
    .await
    .context("bookmark: read")?;
    Ok(row.ok_or(RepositoryError::NotFound(BOOKMARK_NOT_FOUND))?)
}

/// Store a top-first order: the top gets the highest position.
async fn renumber(txn: &DatabaseTransaction, ids: &[i64]) -> anyhow::Result<()> {
    let backend = txn.get_database_backend();
    let len = ids.len();
    for (index, id) in ids.iter().enumerate() {
        let position = i64::try_from(len - 1 - index).context("bookmark: position")?;
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
    use presenter_core::bible::BibleIngestionBatch;
    use presenter_core::{BiblePassage, BibleReference, BibleTranslation};

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

    /// Hebrews 11:1-3 in `code`, verse 2+3 as ONE passage (a range).
    async fn seed(repo: &Repository, code: &str, word: &str) {
        let translation = BibleTranslation::new(code, code, "sk");
        let passages = [(1, 1), (2, 3)]
            .into_iter()
            .map(|(start, end)| {
                let reference =
                    BibleReference::new_with_code("Hebrejom", "HEB", 58, 11, start, end).unwrap();
                BiblePassage::new(reference, translation.clone(), format!("{word} {start}"))
            })
            .collect();
        let batch = BibleIngestionBatch::new(translation, passages).unwrap();
        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();
    }

    fn names(rows: &[StudyBookmarkRecord]) -> Vec<&str> {
        rows.iter().map(|row| row.name.as_str()).collect()
    }

    async fn add(repo: &Repository, owner: Option<&str>, name: &str) -> i64 {
        repo.insert_study_bookmark(owner, name, &place("HEB", 11, 1), None)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn a_new_bookmark_goes_on_top_or_to_its_index() {
        let repo = repo().await;
        let owner = Some("preacher@example.org");
        add(&repo, owner, "Prvá").await;
        add(&repo, owner, "Druhá").await;
        repo.insert_study_bookmark(owner, "Stred", &place("JHN", 3, 16), Some(1))
            .await
            .unwrap();
        repo.insert_study_bookmark(owner, "Ďaleko", &place("GEN", 1, 1), Some(99))
            .await
            .unwrap();
        let rows = repo.list_study_bookmarks(owner, None).await.unwrap();
        assert_eq!(names(&rows), vec!["Druhá", "Stred", "Prvá", "Ďaleko"]);
        // Stored ascending = older: the top holds the highest position.
        let positions: Vec<i64> = rows.iter().map(|row| row.position).collect();
        assert_eq!(positions, vec![3, 2, 1, 0]);
        assert_eq!(rows[1].book_code, "JHN");
        assert_eq!((rows[1].chapter, rows[1].verse), (3, 16));
        assert_eq!(rows[1].owner_email.as_deref(), owner);
    }

    #[tokio::test]
    async fn rows_stored_oldest_first_by_915_show_newest_first() {
        // SNV's rows: positions 0..n in the order they were added (#915).
        let repo = repo().await;
        for (position, name) in ["10:1", "10:26", "12:1"].into_iter().enumerate() {
            repo.db
                .execute(Statement::from_string(
                    repo.db.get_database_backend(),
                    format!(
                        "INSERT INTO study_bookmarks (owner_email, name, translation_code, \
                         book_code, book_name, chapter, verse, position, created_at, updated_at) \
                         VALUES (NULL, '{name}', 'slk-seb', '1SA', 'x', 1, 1, {position}, 't', 't')"
                    ),
                ))
                .await
                .unwrap();
        }
        let rows = repo.list_study_bookmarks(None, None).await.unwrap();
        assert_eq!(names(&rows), vec!["12:1", "10:26", "10:1"]);
        add(&repo, None, "nová").await;
        let rows = repo.list_study_bookmarks(None, None).await.unwrap();
        assert_eq!(names(&rows), vec!["nová", "12:1", "10:26", "10:1"]);
    }

    #[tokio::test]
    async fn the_list_carries_the_verse_text_of_its_translation_else_the_fallback() {
        let repo = repo().await;
        seed(&repo, "slk-seb", "seb").await;
        seed(&repo, "slk-roh", "roh").await;
        let at = |translation: &str, verse| BookmarkPlace {
            translation_code: translation.to_string(),
            ..place("HEB", 11, verse)
        };
        for (name, place) in [
            ("seb1", at("slk-seb", 1)),
            ("roh3", at("slk-roh", 3)),
            ("nlt2", at("eng-nlt", 2)),
            ("seb-none", at("slk-seb", 9)),
        ] {
            repo.insert_study_bookmark(None, name, &place, Some(99))
                .await
                .unwrap();
        }
        let text = |rows: &[StudyBookmarkRecord]| -> Vec<Option<String>> {
            rows.iter().map(|row| row.text.clone()).collect()
        };
        let rows = repo
            .list_study_bookmarks(None, Some("slk-seb"))
            .await
            .unwrap();
        assert_eq!(names(&rows), vec!["seb1", "roh3", "nlt2", "seb-none"]);
        // A verse inside a range passage (2-3) reads that passage.
        assert_eq!(
            text(&rows),
            vec![
                Some("seb 1".to_string()),
                Some("roh 2".to_string()),
                Some("seb 2".to_string()),
                None,
            ]
        );
        // No main translation: the NLT has no text.
        let rows = repo.list_study_bookmarks(None, None).await.unwrap();
        assert_eq!(rows[2].text, None);
    }

    #[tokio::test]
    async fn each_owner_sees_only_its_own_list_and_the_lan_has_one_of_its_own() {
        let repo = repo().await;
        let mine = Some("preacher@example.org");
        let kept = add(&repo, mine, "Moja").await;
        add(&repo, None, "Spoločná").await;
        assert_eq!(
            names(&repo.list_study_bookmarks(mine, None).await.unwrap()),
            vec!["Moja"]
        );
        assert_eq!(
            names(&repo.list_study_bookmarks(None, None).await.unwrap()),
            vec!["Spoločná"]
        );
        assert!(repo
            .list_study_bookmarks(Some("other@example.org"), None)
            .await
            .unwrap()
            .is_empty());
        // Another owner's id is not found, and not reorderable.
        let err = repo
            .delete_study_bookmarks(Some("other@example.org"), &[kept])
            .await
            .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::NotFound(BOOKMARK_NOT_FOUND))
        ));
        let err = repo
            .reorder_study_bookmarks(None, &[kept])
            .await
            .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::Conflict(BOOKMARK_ORDER_STALE))
        ));
        assert_eq!(
            names(&repo.list_study_bookmarks(mine, None).await.unwrap()),
            vec!["Moja"]
        );
    }

    #[tokio::test]
    async fn a_reorder_takes_the_whole_list_top_first_or_nothing() {
        let repo = repo().await;
        let a = add(&repo, None, "A").await;
        let b = add(&repo, None, "B").await;
        let c = add(&repo, None, "C").await;
        repo.reorder_study_bookmarks(None, &[a, c, b])
            .await
            .unwrap();
        assert_eq!(
            names(&repo.list_study_bookmarks(None, None).await.unwrap()),
            vec!["A", "C", "B"]
        );
        for stale in [vec![a, b], vec![a, b, c, c], vec![a, b, 999], vec![]] {
            let err = repo
                .reorder_study_bookmarks(None, &stale)
                .await
                .unwrap_err();
            assert!(
                matches!(
                    err.downcast_ref::<RepositoryError>(),
                    Some(RepositoryError::Conflict(BOOKMARK_ORDER_STALE))
                ),
                "{stale:?}"
            );
        }
        assert_eq!(
            names(&repo.list_study_bookmarks(None, None).await.unwrap()),
            vec!["A", "C", "B"]
        );
    }

    #[tokio::test]
    async fn a_delete_answers_the_old_indices_and_putting_them_back_restores_the_order() {
        let repo = repo().await;
        let mut ids = Vec::new();
        for name in ["E", "D", "C", "B", "A"] {
            ids.push(add(&repo, None, name).await);
        }
        // Shown: A B C D E. Delete A (top), C and E (bottom).
        let (a, c, e) = (ids[4], ids[2], ids[0]);
        let deleted = repo.delete_study_bookmarks(None, &[e, a, c]).await.unwrap();
        let answered: Vec<(&str, usize)> = deleted
            .iter()
            .map(|item| (item.row.name.as_str(), item.index))
            .collect();
        assert_eq!(answered, vec![("A", 0), ("C", 2), ("E", 4)]);
        assert_eq!(
            names(&repo.list_study_bookmarks(None, None).await.unwrap()),
            vec!["B", "D"]
        );
        for item in &deleted {
            repo.insert_study_bookmark(
                None,
                &item.row.name,
                &place("HEB", 11, 1),
                Some(item.index),
            )
            .await
            .unwrap();
        }
        assert_eq!(
            names(&repo.list_study_bookmarks(None, None).await.unwrap()),
            vec!["A", "B", "C", "D", "E"]
        );
    }

    #[tokio::test]
    async fn a_delete_with_an_unknown_id_deletes_nothing() {
        let repo = repo().await;
        let a = add(&repo, None, "A").await;
        let err = repo
            .delete_study_bookmarks(None, &[a, 999])
            .await
            .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::NotFound(_))
        ));
        assert_eq!(
            names(&repo.list_study_bookmarks(None, None).await.unwrap()),
            vec!["A"]
        );
    }
}
