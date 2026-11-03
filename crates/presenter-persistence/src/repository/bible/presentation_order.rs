//! #874: the Prepared tab's order and its archive. The active list is
//! ordered by `position` (smallest first); an archived presentation has
//! `archived_at` set and is out of that list (tablet, verse console, AI tools
//! never offer it), but every read by id still finds it.

use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
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

/// The position that puts a presentation at the top of the active list: one
/// above the smallest active position (0 when the list is empty).
pub(super) async fn top_position<C: ConnectionTrait>(conn: &C) -> anyhow::Result<i32> {
    let smallest: Option<Option<i32>> = bible_presentation::Entity::find()
        .select_only()
        .column_as(bible_presentation::Column::Position.min(), "smallest")
        .filter(bible_presentation::Column::ArchivedAt.is_null())
        .into_tuple()
        .one(conn)
        .await?;
    Ok(smallest.flatten().map_or(0, |p| p.saturating_sub(1)))
}

/// True when `ids` is exactly `active` (same ids, each once), in any order.
fn same_set(active: &[String], ids: &[String]) -> bool {
    let mut a = active.to_vec();
    let mut b = ids.to_vec();
    a.sort();
    b.sort();
    let before = b.len();
    b.dedup();
    before == b.len() && a == b
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
    /// (`Conflict` → 409), never a partial reorder.
    #[instrument(skip_all)]
    pub async fn reorder_bible_presentations(
        &self,
        ids: &[BiblePresentationId],
    ) -> anyhow::Result<()> {
        let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
        let txn = self.db.begin().await?;
        let active: Vec<String> = bible_presentation::Entity::find()
            .select_only()
            .column(bible_presentation::Column::Id)
            .filter(bible_presentation::Column::ArchivedAt.is_null())
            .into_tuple()
            .all(&txn)
            .await?;
        if !same_set(&active, &ids) {
            return Err(RepositoryError::Conflict(PRESENTATION_ORDER_CONFLICT).into());
        }
        for (index, id) in ids.iter().enumerate() {
            let position = i32::try_from(index).unwrap_or(i32::MAX);
            bible_presentation::Entity::update_many()
                .col_expr(bible_presentation::Column::Position, Expr::value(position))
                .filter(bible_presentation::Column::Id.eq(id.as_str()))
                .exec(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Archive a presentation. Archiving an archived one changes nothing (its
    /// archive time stays).
    #[instrument(skip_all)]
    pub async fn archive_bible_presentation(&self, id: BiblePresentationId) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let model = bible_presentation::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
            .ok_or(RepositoryError::NotFound(NOT_FOUND))?;
        if model.archived_at.is_none() {
            bible_presentation::Entity::update_many()
                .col_expr(
                    bible_presentation::Column::ArchivedAt,
                    Expr::value(Utc::now().to_rfc3339()),
                )
                .filter(bible_presentation::Column::Id.eq(model.id))
                .exec(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Bring a presentation back from the archive, to the top of the active
    /// list. Restoring an active one changes nothing.
    #[instrument(skip_all)]
    pub async fn restore_bible_presentation(&self, id: BiblePresentationId) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;
        let model = bible_presentation::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
            .ok_or(RepositoryError::NotFound(NOT_FOUND))?;
        if model.archived_at.is_some() {
            let position = top_position(&txn).await?;
            bible_presentation::Entity::update_many()
                .col_expr(
                    bible_presentation::Column::ArchivedAt,
                    Expr::value(Option::<String>::None),
                )
                .col_expr(bible_presentation::Column::Position, Expr::value(position))
                .filter(bible_presentation::Column::Id.eq(model.id))
                .exec(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::same_set;

    fn v(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_reorder_needs_exactly_the_active_set() {
        assert!(same_set(&v(&["a", "b", "c"]), &v(&["c", "a", "b"])));
        assert!(same_set(&v(&[]), &v(&[])));
        assert!(!same_set(&v(&["a", "b"]), &v(&["a"])));
        assert!(!same_set(&v(&["a", "b"]), &v(&["a", "b", "x"])));
        assert!(!same_set(&v(&["a", "b"]), &v(&["a", "a"])));
        assert!(!same_set(&v(&["a", "b", "c"]), &v(&["a", "b", "b", "c"])));
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
