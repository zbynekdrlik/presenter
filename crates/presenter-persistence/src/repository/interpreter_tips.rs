//! Šepkár tips repository (#852 spec §9.2): plain CRUD over
//! `interpreter_tips`. Every rule that needs a clock (the active window, the
//! duplicate window, "today") lives in the caller, which passes the dates.
//! Refusals are typed (`repository-error-pattern.md`): an unknown id is
//! `NotFound` (404), restoring a tip of another day is `Conflict` (409).

use super::util::RepositoryError;
use super::Repository;
use crate::entities::interpreter_tip;
use chrono::{DateTime, NaiveDate, Utc};
use presenter_core::interpreter_tips::{service_date_string, TipDto};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, NotSet, QueryFilter, QueryOrder,
    Set, TransactionTrait,
};
use tracing::instrument;

/// A validated tip plus its audit fields, ready to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInterpreterTip {
    pub service_date: NaiveDate,
    pub source_term: Option<String>,
    pub target_term: Option<String>,
    pub note: Option<String>,
    pub sender_name: String,
    pub sender_ip: String,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}

const TIP_NOT_FOUND: &str = "Tip neexistuje.";

impl Repository {
    #[instrument(skip_all)]
    pub async fn insert_interpreter_tip(&self, tip: NewInterpreterTip) -> anyhow::Result<TipDto> {
        let inserted = interpreter_tip::ActiveModel {
            id: NotSet,
            service_date: Set(service_date_string(tip.service_date)),
            source_term: Set(tip.source_term),
            target_term: Set(tip.target_term),
            note: Set(tip.note),
            sender_name: Set(tip.sender_name),
            sender_ip: Set(tip.sender_ip),
            user_agent: Set(tip.user_agent),
            created_at: Set(tip.created_at.into()),
            dismissed_at: Set(None),
        }
        .insert(&self.db)
        .await?;
        Ok(tip_from_model(inserted))
    }

    /// Every tip of one service date, newest first — ordered by id, which
    /// follows insertion, never by a stored timestamp string.
    pub async fn list_interpreter_tips_on(
        &self,
        service_date: NaiveDate,
    ) -> anyhow::Result<Vec<TipDto>> {
        let models = interpreter_tip::Entity::find()
            .filter(interpreter_tip::Column::ServiceDate.eq(service_date_string(service_date)))
            .order_by_desc(interpreter_tip::Column::Id)
            .all(&self.db)
            .await?;
        Ok(models.into_iter().map(tip_from_model).collect())
    }

    /// Hide one tip. Idempotent: a hidden tip keeps its first `dismissed_at`.
    #[instrument(skip_all)]
    pub async fn dismiss_interpreter_tip(
        &self,
        id: i64,
        at: DateTime<Utc>,
    ) -> anyhow::Result<TipDto> {
        let model = self.interpreter_tip_by_id(id).await?;
        if model.dismissed_at.is_some() {
            return Ok(tip_from_model(model));
        }
        let mut active = model.into_active_model();
        active.dismissed_at = Set(Some(at.into()));
        Ok(tip_from_model(active.update(&self.db).await?))
    }

    /// Undo a dismiss (the interpreter's 5 s "Späť"). A tip of another
    /// service date is `Conflict`.
    #[instrument(skip_all)]
    pub async fn restore_interpreter_tip(
        &self,
        id: i64,
        today: NaiveDate,
    ) -> anyhow::Result<TipDto> {
        let model = self.interpreter_tip_by_id(id).await?;
        if model.service_date != service_date_string(today) {
            return Err(
                RepositoryError::Conflict("Tento tip už nie je z dnešnej bohoslužby.").into(),
            );
        }
        if model.dismissed_at.is_none() {
            return Ok(tip_from_model(model));
        }
        let mut active = model.into_active_model();
        active.dismissed_at = Set(None);
        Ok(tip_from_model(active.update(&self.db).await?))
    }

    /// Hide the given tips in one transaction. Returns how many were still
    /// shown; a missing or already hidden id is skipped.
    #[instrument(skip_all)]
    pub async fn dismiss_interpreter_tips(
        &self,
        ids: &[i64],
        at: DateTime<Utc>,
    ) -> anyhow::Result<u64> {
        let keys: Vec<i32> = ids
            .iter()
            .filter_map(|id| i32::try_from(*id).ok())
            .collect();
        if keys.is_empty() {
            return Ok(0);
        }
        let txn = self.db.begin().await?;
        let models = interpreter_tip::Entity::find()
            .filter(interpreter_tip::Column::Id.is_in(keys))
            .filter(interpreter_tip::Column::DismissedAt.is_null())
            .all(&txn)
            .await?;
        let count = models.len() as u64;
        for model in models {
            let mut active = model.into_active_model();
            active.dismissed_at = Set(Some(at.into()));
            active.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(count)
    }

    /// Delete every tip whose service date is before `cutoff` (retention).
    #[instrument(skip_all)]
    pub async fn prune_interpreter_tips_before(&self, cutoff: NaiveDate) -> anyhow::Result<u64> {
        let result = interpreter_tip::Entity::delete_many()
            .filter(interpreter_tip::Column::ServiceDate.lt(service_date_string(cutoff)))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    async fn interpreter_tip_by_id(&self, id: i64) -> anyhow::Result<interpreter_tip::Model> {
        // An out-of-i32 id can never be a row: refuse, never wrap into another
        // row (stream-graphics.md i32/i64 rule).
        let key = i32::try_from(id).map_err(|_| RepositoryError::NotFound(TIP_NOT_FOUND))?;
        interpreter_tip::Entity::find_by_id(key)
            .one(&self.db)
            .await?
            .ok_or(RepositoryError::NotFound(TIP_NOT_FOUND).into())
    }
}

fn tip_from_model(model: interpreter_tip::Model) -> TipDto {
    TipDto {
        id: i64::from(model.id),
        service_date: model.service_date,
        source_term: model.source_term,
        target_term: model.target_term,
        note: model.note,
        sender_name: model.sender_name,
        created_at: model.created_at.with_timezone(&Utc),
        dismissed_at: model.dismissed_at.map(|at| at.with_timezone(&Utc)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).expect("date")
    }

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .expect("RFC 3339")
            .with_timezone(&Utc)
    }

    fn new_tip(day: NaiveDate, created: &str, note: &str) -> NewInterpreterTip {
        NewInterpreterTip {
            service_date: day,
            source_term: None,
            target_term: None,
            note: Some(note.to_string()),
            sender_name: "Peter".to_string(),
            sender_ip: "203.0.113.9".to_string(),
            user_agent: Some("Mozilla/5.0 (iPhone)".to_string()),
            created_at: at(created),
        }
    }

    async fn repo() -> Repository {
        Repository::connect_in_memory()
            .await
            .expect("in-memory repository")
    }

    fn is_not_found(err: &anyhow::Error) -> bool {
        matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::NotFound(_))
        )
    }

    #[tokio::test]
    async fn insert_returns_the_dto_and_a_day_lists_newest_first() {
        let repo = repo().await;
        let first = repo
            .insert_interpreter_tip(new_tip(date(10, 18), "2026-10-18T17:00:00Z", "a"))
            .await
            .expect("insert");
        assert_eq!(first.service_date, "2026-10-18");
        assert_eq!(first.note.as_deref(), Some("a"));
        assert_eq!(first.created_at, at("2026-10-18T17:00:00Z"));
        assert_eq!(first.dismissed_at, None);
        let second = repo
            .insert_interpreter_tip(new_tip(date(10, 18), "2026-10-18T17:01:00Z", "b"))
            .await
            .expect("insert");
        repo.insert_interpreter_tip(new_tip(date(10, 17), "2026-10-17T17:00:00Z", "yesterday"))
            .await
            .expect("insert");
        let today = repo
            .list_interpreter_tips_on(date(10, 18))
            .await
            .expect("list");
        let ids: Vec<i64> = today.iter().map(|tip| tip.id).collect();
        assert_eq!(
            ids,
            vec![second.id, first.id],
            "only that day, newest first"
        );
    }

    #[tokio::test]
    async fn dismiss_is_idempotent_and_keeps_the_first_time() {
        let repo = repo().await;
        let tip = repo
            .insert_interpreter_tip(new_tip(date(10, 18), "2026-10-18T17:00:00Z", "a"))
            .await
            .expect("insert");
        let first = repo
            .dismiss_interpreter_tip(tip.id, at("2026-10-18T17:05:00Z"))
            .await
            .expect("dismiss");
        assert_eq!(first.dismissed_at, Some(at("2026-10-18T17:05:00Z")));
        let again = repo
            .dismiss_interpreter_tip(tip.id, at("2026-10-18T17:09:00Z"))
            .await
            .expect("dismiss again");
        assert_eq!(again.dismissed_at, Some(at("2026-10-18T17:05:00Z")));
    }

    #[tokio::test]
    async fn restore_clears_the_dismiss_and_refuses_another_day() {
        let repo = repo().await;
        let tip = repo
            .insert_interpreter_tip(new_tip(date(10, 18), "2026-10-18T17:00:00Z", "a"))
            .await
            .expect("insert");
        repo.dismiss_interpreter_tip(tip.id, at("2026-10-18T17:05:00Z"))
            .await
            .expect("dismiss");
        let restored = repo
            .restore_interpreter_tip(tip.id, date(10, 18))
            .await
            .expect("restore");
        assert_eq!(restored.dismissed_at, None);
        let err = repo
            .restore_interpreter_tip(tip.id, date(10, 19))
            .await
            .expect_err("another day");
        assert!(
            matches!(
                err.downcast_ref::<RepositoryError>(),
                Some(RepositoryError::Conflict(_))
            ),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn unknown_or_out_of_range_ids_are_not_found() {
        let repo = repo().await;
        let now = at("2026-10-18T17:00:00Z");
        for id in [999_999, i64::from(i32::MAX) + 1, -1] {
            assert!(is_not_found(
                &repo
                    .dismiss_interpreter_tip(id, now)
                    .await
                    .expect_err("dismiss")
            ));
            assert!(is_not_found(
                &repo
                    .restore_interpreter_tip(id, date(10, 18))
                    .await
                    .expect_err("restore")
            ));
        }
    }

    #[tokio::test]
    async fn dismiss_many_counts_only_tips_still_shown() {
        let repo = repo().await;
        let a = repo
            .insert_interpreter_tip(new_tip(date(10, 18), "2026-10-18T17:00:00Z", "a"))
            .await
            .expect("insert");
        let b = repo
            .insert_interpreter_tip(new_tip(date(10, 18), "2026-10-18T17:01:00Z", "b"))
            .await
            .expect("insert");
        repo.dismiss_interpreter_tip(a.id, at("2026-10-18T17:02:00Z"))
            .await
            .expect("dismiss");
        let count = repo
            .dismiss_interpreter_tips(&[a.id, b.id, 999_999], at("2026-10-18T17:03:00Z"))
            .await
            .expect("dismiss many");
        assert_eq!(count, 1, "a was already hidden, 999999 does not exist");
        assert_eq!(
            repo.dismiss_interpreter_tips(&[], at("2026-10-18T17:04:00Z"))
                .await
                .expect("empty"),
            0
        );
        let today = repo
            .list_interpreter_tips_on(date(10, 18))
            .await
            .expect("list");
        assert!(today.iter().all(|tip| tip.dismissed_at.is_some()));
    }

    #[tokio::test]
    async fn prune_deletes_days_before_the_cutoff_only() {
        let repo = repo().await;
        repo.insert_interpreter_tip(new_tip(date(9, 17), "2026-09-17T17:00:00Z", "old"))
            .await
            .expect("insert");
        repo.insert_interpreter_tip(new_tip(date(9, 18), "2026-09-18T17:00:00Z", "kept"))
            .await
            .expect("insert");
        let pruned = repo
            .prune_interpreter_tips_before(date(9, 18))
            .await
            .expect("prune");
        assert_eq!(pruned, 1);
        assert!(repo
            .list_interpreter_tips_on(date(9, 17))
            .await
            .expect("list")
            .is_empty());
        assert_eq!(
            repo.list_interpreter_tips_on(date(9, 18))
                .await
                .expect("list")
                .len(),
            1
        );
    }
}
