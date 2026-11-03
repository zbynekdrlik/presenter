//! #874: the Prepared tab's order and its archive. The active list is
//! ordered by `position` (smallest first); an archived presentation has
//! `archived_at` set and is out of that list (tablet, verse console, AI tools
//! never offer it), but every read by id still finds it.
//!
//! Every write transaction starts with its WRITE (sermon-prep rule): two
//! deferred readers upgrading to writers deadlock into SQLITE_BUSY (a 500);
//! a writer first takes the lock at once and the other waits (busy_timeout).

use chrono::Utc;
use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    TransactionTrait,
};
use tracing::instrument;

use crate::entities::bible_presentation;
use crate::repository::util::RepositoryError;
use crate::repository::Repository;
use presenter_core::{BiblePresentationId, BiblePresentationSummary};

/// A reorder named a different set than the active list holds now.
pub const PRESENTATION_ORDER_CONFLICT: &str =
    "zoznam prezentácií sa medzitým zmenil — obnovte stránku a skúste znova";

const NOT_FOUND: &str = "bible presentation not found";

/// One above the smallest position of the OTHER active presentations (0 when
/// there is none), computed by SQLite inside the UPDATE itself.
fn top_of_others(id: &str) -> SimpleExpr {
    Expr::cust_with_values(
        r#"(SELECT COALESCE(MIN(b."position"), 1) - 1 FROM "bible_presentations" AS b WHERE b."archived_at" IS NULL AND b."id" <> ?)"#,
        [id.to_string()],
    )
}

/// Move the presentation `id` to the top of the active list: one statement,
/// so no read precedes the write.
pub(super) async fn move_to_top<C: ConnectionTrait>(conn: &C, id: &str) -> anyhow::Result<()> {
    bible_presentation::Entity::update_many()
        .col_expr(bible_presentation::Column::Position, top_of_others(id))
        .filter(bible_presentation::Column::Id.eq(id))
        .exec(conn)
        .await?;
    Ok(())
}

/// True when the ids are pairwise distinct.
fn distinct(ids: &[String]) -> bool {
    let mut sorted = ids.to_vec();
    sorted.sort();
    sorted.windows(2).all(|pair| pair[0] != pair[1])
}

impl Repository {
    /// The archived presentations, the most recently archived first.
    #[instrument(skip_all)]
    pub async fn list_archived_bible_presentation_summaries(
        &self,
    ) -> anyhow::Result<Vec<BiblePresentationSummary>> {
        let models = bible_presentation::Entity::find()
            .filter(bible_presentation::Column::ArchivedAt.is_not_null())
            .order_by_desc(bible_presentation::Column::ArchivedAt)
            .order_by_asc(bible_presentation::Column::Name)
            .all(&self.db)
            .await?;
        self.summaries_of(models).await
    }

    /// Put the active list into the order of `ids`, which must name every
    /// active presentation exactly once — anything else is a stale view
    /// (`Conflict` → 409) and the transaction rolls back: never a partial
    /// reorder. The positions are written FIRST and the set checked after.
    #[instrument(skip_all)]
    pub async fn reorder_bible_presentations(
        &self,
        ids: &[BiblePresentationId],
    ) -> anyhow::Result<()> {
        let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
        let conflict =
            || anyhow::Error::from(RepositoryError::Conflict(PRESENTATION_ORDER_CONFLICT));
        if !distinct(&ids) {
            return Err(conflict());
        }
        let txn = self.db.begin().await?;
        for (index, id) in ids.iter().enumerate() {
            let position = i32::try_from(index).unwrap_or(i32::MAX);
            let result = bible_presentation::Entity::update_many()
                .col_expr(bible_presentation::Column::Position, Expr::value(position))
                .filter(bible_presentation::Column::Id.eq(id.as_str()))
                .filter(bible_presentation::Column::ArchivedAt.is_null())
                .exec(&txn)
                .await?;
            if result.rows_affected != 1 {
                // Unknown or archived: dropping `txn` rolls the writes back.
                return Err(conflict());
            }
        }
        let active = bible_presentation::Entity::find()
            .filter(bible_presentation::Column::ArchivedAt.is_null())
            .count(&txn)
            .await?;
        if active != ids.len() as u64 {
            return Err(conflict());
        }
        txn.commit().await?;
        Ok(())
    }

    /// Archive a presentation. Archiving an archived one changes nothing (its
    /// archive time stays).
    #[instrument(skip_all)]
    pub async fn archive_bible_presentation(&self, id: BiblePresentationId) -> anyhow::Result<()> {
        let result = bible_presentation::Entity::update_many()
            .col_expr(
                bible_presentation::Column::ArchivedAt,
                Expr::value(Utc::now().to_rfc3339()),
            )
            .filter(bible_presentation::Column::Id.eq(id.to_string()))
            .filter(bible_presentation::Column::ArchivedAt.is_null())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            self.require_presentation(id).await?;
        }
        Ok(())
    }

    /// Bring a presentation back from the archive, to the top of the active
    /// list (one statement). Restoring an active one changes nothing.
    #[instrument(skip_all)]
    pub async fn restore_bible_presentation(&self, id: BiblePresentationId) -> anyhow::Result<()> {
        let id_str = id.to_string();
        let result = bible_presentation::Entity::update_many()
            .col_expr(
                bible_presentation::Column::ArchivedAt,
                Expr::value(Option::<String>::None),
            )
            .col_expr(bible_presentation::Column::Position, top_of_others(&id_str))
            .filter(bible_presentation::Column::Id.eq(id_str.as_str()))
            .filter(bible_presentation::Column::ArchivedAt.is_not_null())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            self.require_presentation(id).await?;
        }
        Ok(())
    }

    /// `NotFound` unless the presentation exists.
    async fn require_presentation(&self, id: BiblePresentationId) -> anyhow::Result<()> {
        bible_presentation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or(RepositoryError::NotFound(NOT_FOUND))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::distinct;

    fn v(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_reorder_names_each_id_once() {
        assert!(distinct(&v(&["a", "b", "c"])));
        assert!(distinct(&v(&[])));
        assert!(!distinct(&v(&["a", "a"])));
        assert!(!distinct(&v(&["a", "b", "b", "c"])));
    }
}

#[cfg(test)]
mod repository_tests {
    use super::PRESENTATION_ORDER_CONFLICT;
    use crate::repository::util::RepositoryError;
    use crate::repository::Repository;
    use presenter_core::BiblePresentationId;

    async fn repo() -> Repository {
        Repository::connect_in_memory()
            .await
            .expect("in-memory repo")
    }

    async fn names(repo: &Repository) -> Vec<String> {
        let list = repo.list_bible_presentation_summaries().await.unwrap();
        list.into_iter().map(|s| s.name).collect()
    }

    async fn archived_names(repo: &Repository) -> Vec<String> {
        let list = repo
            .list_archived_bible_presentation_summaries()
            .await
            .unwrap();
        list.into_iter().map(|s| s.name).collect()
    }

    #[tokio::test]
    async fn a_new_presentation_goes_to_the_top() {
        let repo = repo().await;
        repo.create_bible_presentation("A").await.unwrap();
        repo.create_bible_presentation("C").await.unwrap();
        repo.create_bible_presentation("B").await.unwrap();
        assert_eq!(names(&repo).await, vec!["B", "C", "A"]);
    }

    #[tokio::test]
    async fn a_reorder_persists_the_given_order() {
        let repo = repo().await;
        let a = repo.create_bible_presentation("A").await.unwrap();
        let b = repo.create_bible_presentation("B").await.unwrap();
        let c = repo.create_bible_presentation("C").await.unwrap();
        repo.reorder_bible_presentations(&[a.id, c.id, b.id])
            .await
            .unwrap();
        assert_eq!(names(&repo).await, vec!["A", "C", "B"]);
    }

    #[tokio::test]
    async fn a_reorder_with_a_different_set_is_a_conflict_and_changes_nothing() {
        let repo = repo().await;
        let a = repo.create_bible_presentation("A").await.unwrap();
        let b = repo.create_bible_presentation("B").await.unwrap();
        let before = names(&repo).await;
        for ids in [
            vec![a.id],
            vec![a.id, b.id, BiblePresentationId::new()],
            vec![a.id, a.id],
        ] {
            let err = repo.reorder_bible_presentations(&ids).await.unwrap_err();
            assert!(
                matches!(
                    err.downcast_ref::<RepositoryError>(),
                    Some(RepositoryError::Conflict(PRESENTATION_ORDER_CONFLICT))
                ),
                "{err}"
            );
        }
        assert_eq!(names(&repo).await, before);
    }

    #[tokio::test]
    async fn an_archived_presentation_leaves_the_list_and_a_reorder_must_not_name_it() {
        let repo = repo().await;
        let a = repo.create_bible_presentation("A").await.unwrap();
        let b = repo.create_bible_presentation("B").await.unwrap();
        repo.archive_bible_presentation(a.id).await.unwrap();
        assert_eq!(names(&repo).await, vec!["B"]);
        assert_eq!(archived_names(&repo).await, vec!["A"]);
        // Still readable by id (a slide trigger, the AI tools).
        assert!(repo.fetch_bible_presentation(a.id).await.unwrap().is_some());
        assert!(repo
            .reorder_bible_presentations(&[b.id, a.id])
            .await
            .is_err());
        repo.reorder_bible_presentations(&[b.id]).await.unwrap();
    }

    #[tokio::test]
    async fn a_restored_presentation_comes_back_at_the_top() {
        let repo = repo().await;
        let a = repo.create_bible_presentation("A").await.unwrap();
        repo.create_bible_presentation("B").await.unwrap();
        repo.create_bible_presentation("C").await.unwrap();
        repo.archive_bible_presentation(a.id).await.unwrap();
        repo.restore_bible_presentation(a.id).await.unwrap();
        assert_eq!(names(&repo).await, vec!["A", "C", "B"]);
        assert!(archived_names(&repo).await.is_empty());
        // Restoring an active one changes nothing.
        repo.restore_bible_presentation(a.id).await.unwrap();
        assert_eq!(names(&repo).await, vec!["A", "C", "B"]);
    }

    #[tokio::test]
    async fn the_archive_lists_the_most_recently_archived_first() {
        let repo = repo().await;
        let a = repo.create_bible_presentation("A").await.unwrap();
        let b = repo.create_bible_presentation("B").await.unwrap();
        repo.archive_bible_presentation(a.id).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        repo.archive_bible_presentation(b.id).await.unwrap();
        assert_eq!(archived_names(&repo).await, vec!["B", "A"]);
        // Archiving again keeps the first archive time.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        repo.archive_bible_presentation(a.id).await.unwrap();
        assert_eq!(archived_names(&repo).await, vec!["B", "A"]);
    }

    /// A file database with a real connection pool: concurrent writers on
    /// separate connections, as on SNV (the in-memory one shares one cache).
    async fn file_repo() -> (Repository, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "presenter-874-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.join("order.db").display());
        let repo = Repository::connect(&crate::repository::DatabaseSettings::new(url))
            .await
            .unwrap();
        (repo, dir)
    }

    #[tokio::test]
    async fn concurrent_writes_wait_for_each_other_instead_of_failing() {
        let (repo, dir) = file_repo().await;
        let seed = repo.create_bible_presentation("Seed").await.unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..8 {
            let repo = repo.clone();
            tasks.spawn(async move {
                repo.create_bible_presentation(&format!("P{i}"))
                    .await
                    .map(|_| ())
            });
        }
        let r = repo.clone();
        tasks.spawn(async move { r.archive_bible_presentation(seed.id).await });
        let r = repo.clone();
        tasks.spawn(async move { r.restore_bible_presentation(seed.id).await });
        while let Some(result) = tasks.join_next().await {
            result.unwrap().unwrap();
        }
        let list = repo.list_bible_presentation_summaries().await.unwrap();
        assert_eq!(list.len(), 9);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn concurrent_creates_get_distinct_places() {
        let (repo, dir) = file_repo().await;
        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..6 {
            let repo = repo.clone();
            tasks.spawn(async move { repo.create_bible_presentation(&format!("P{i}")).await });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap().unwrap();
        }
        use crate::entities::bible_presentation;
        use sea_orm::EntityTrait;
        let mut positions: Vec<i32> = bible_presentation::Entity::find()
            .all(&repo.db)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.position)
            .collect();
        positions.sort_unstable();
        positions.dedup();
        assert_eq!(positions.len(), 6, "{positions:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn archive_and_restore_of_an_unknown_id_are_not_found() {
        let repo = repo().await;
        for err in [
            repo.archive_bible_presentation(BiblePresentationId::new())
                .await
                .unwrap_err(),
            repo.restore_bible_presentation(BiblePresentationId::new())
                .await
                .unwrap_err(),
        ] {
            assert!(matches!(
                err.downcast_ref::<RepositoryError>(),
                Some(RepositoryError::NotFound(_))
            ));
        }
    }
}
