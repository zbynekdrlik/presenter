//! The guest transcript's store (#852 C3-persist, spec §12): plain writes and
//! reads over `listen_transcript`. Every rule that needs a clock (today's
//! service date, the 7-day cutoff) lives in the caller, which passes dates.
//! Rows convert both ways through one typed error (`repository-error-pattern.md`):
//! a value that does not fit the column / the wire type is
//! [`ListenTranscriptError`], never a silent wrap or a panic.

use super::Repository;
use crate::entities::listen_transcript;
use chrono::{NaiveDate, Utc};
use presenter_core::interpreter_tips::service_date_string;
use presenter_core::listen::ws::TranscriptSentence;
use sea_orm::{ColumnTrait, EntityTrait, NotSet, QueryFilter, QueryOrder, QuerySelect, Set};
use tracing::instrument;

/// A closed sentence and the local service date it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTranscriptSentence {
    pub service_date: NaiveDate,
    pub sentence: TranscriptSentence,
}

/// A transcript value that cannot cross the column ↔ wire boundary. An
/// internal data fault (the router's 500), typed so a caller can tell it from
/// a database error.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ListenTranscriptError {
    /// A sentence value above `i64::MAX` cannot be stored.
    #[error("transcript sentence {field} {value} does not fit the column")]
    TooLarge { field: &'static str, value: u64 },
    /// A stored row holds a negative value where the wire needs a `u64`.
    #[error("listen_transcript row {id}: {field} is negative ({value})")]
    Negative {
        id: i32,
        field: &'static str,
        value: i64,
    },
}

impl Repository {
    /// Insert closed sentences in ONE statement; returns how many. Nothing is
    /// written when any of them cannot be stored.
    #[instrument(skip_all, fields(rows = rows.len()))]
    pub async fn insert_listen_transcript(
        &self,
        rows: &[NewTranscriptSentence],
    ) -> anyhow::Result<u64> {
        if rows.is_empty() {
            return Ok(0);
        }
        let models = rows
            .iter()
            .map(active_model)
            .collect::<Result<Vec<_>, _>>()?;
        let _last = listen_transcript::Entity::insert_many(models)
            .exec(&self.db)
            .await?;
        Ok(u64::try_from(rows.len()).unwrap_or(u64::MAX))
    }

    /// The newest `limit` sentences of one service date, oldest first (by row
    /// id, which follows insertion — never by a stored timestamp string).
    pub async fn newest_listen_transcript_on(
        &self,
        service_date: NaiveDate,
        limit: u64,
    ) -> anyhow::Result<Vec<TranscriptSentence>> {
        let mut models = listen_transcript::Entity::find()
            .filter(listen_transcript::Column::ServiceDate.eq(service_date_string(service_date)))
            .order_by_desc(listen_transcript::Column::Id)
            .limit(limit)
            .all(&self.db)
            .await?;
        models.reverse();
        Ok(models
            .into_iter()
            .map(sentence_from_model)
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// The highest stored sentence id of any day (`None` when empty): new
    /// ids continue after it, so a stored id is never reused.
    pub async fn max_listen_transcript_sentence_id(&self) -> anyhow::Result<Option<u64>> {
        let newest = listen_transcript::Entity::find()
            .order_by_desc(listen_transcript::Column::SentenceId)
            .one(&self.db)
            .await?;
        let Some(model) = newest else {
            return Ok(None);
        };
        Ok(Some(non_negative(
            model.id,
            "sentence_id",
            model.sentence_id,
        )?))
    }

    /// Delete every sentence whose service date is before `cutoff` (retention).
    #[instrument(skip_all)]
    pub async fn prune_listen_transcript_before(&self, cutoff: NaiveDate) -> anyhow::Result<u64> {
        let result = listen_transcript::Entity::delete_many()
            .filter(listen_transcript::Column::ServiceDate.lt(service_date_string(cutoff)))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }
}

fn active_model(
    row: &NewTranscriptSentence,
) -> Result<listen_transcript::ActiveModel, ListenTranscriptError> {
    let sentence = &row.sentence;
    Ok(listen_transcript::ActiveModel {
        id: NotSet,
        mic_id: Set(sentence.mic_id.clone()),
        sentence_id: Set(fits("id", sentence.id)?),
        text: Set(sentence.text.clone()),
        start_ms: Set(fits("start_ms", sentence.start_ms)?),
        end_ms: Set(fits("end_ms", sentence.end_ms)?),
        closed_at: Set(sentence.closed_at.into()),
        language: Set(sentence.language.clone()),
        service_date: Set(service_date_string(row.service_date)),
    })
}

fn fits(field: &'static str, value: u64) -> Result<i64, ListenTranscriptError> {
    i64::try_from(value).map_err(|_| ListenTranscriptError::TooLarge { field, value })
}

fn non_negative(id: i32, field: &'static str, value: i64) -> Result<u64, ListenTranscriptError> {
    u64::try_from(value).map_err(|_| ListenTranscriptError::Negative { id, field, value })
}

fn sentence_from_model(
    model: listen_transcript::Model,
) -> Result<TranscriptSentence, ListenTranscriptError> {
    Ok(TranscriptSentence {
        id: non_negative(model.id, "sentence_id", model.sentence_id)?,
        start_ms: non_negative(model.id, "start_ms", model.start_ms)?,
        end_ms: non_negative(model.id, "end_ms", model.end_ms)?,
        mic_id: model.mic_id,
        text: model.text,
        closed_at: model.closed_at.with_timezone(&Utc),
        language: model.language,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone};
    use sea_orm::{ConnectionTrait, DbBackend, Statement};

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).expect("date")
    }

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 22, 9, 30, second)
            .single()
            .expect("time")
    }

    fn row(day: u32, id: u64, text: &str) -> NewTranscriptSentence {
        NewTranscriptSentence {
            service_date: date(day),
            sentence: TranscriptSentence {
                id,
                mic_id: "guest".to_string(),
                text: text.to_string(),
                start_ms: id * 1000,
                end_ms: id * 1000 + 900,
                closed_at: at(u32::try_from(id % 60).expect("second")),
                language: (id % 2 == 0).then(|| "en".to_string()),
            },
        }
    }

    async fn repo() -> Repository {
        Repository::connect_in_memory().await.expect("repository")
    }

    #[tokio::test]
    async fn an_inserted_sentence_reads_back_unchanged() {
        let repo = repo().await;
        let written = vec![row(22, 7, "Grace and peace."), row(22, 8, "Amen.")];
        assert_eq!(repo.insert_listen_transcript(&written).await.unwrap(), 2);
        let read = repo
            .newest_listen_transcript_on(date(22), 500)
            .await
            .unwrap();
        let expected: Vec<TranscriptSentence> =
            written.into_iter().map(|row| row.sentence).collect();
        assert_eq!(read, expected, "every field, oldest first");
        assert_eq!(repo.insert_listen_transcript(&[]).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn the_newest_n_of_one_date_oldest_first() {
        let repo = repo().await;
        let rows: Vec<_> = (1..=6).map(|id| row(22, id, &format!("s{id}"))).collect();
        repo.insert_listen_transcript(&rows).await.unwrap();
        repo.insert_listen_transcript(&[row(21, 99, "yesterday"), row(23, 100, "tomorrow")])
            .await
            .unwrap();
        let read = repo.newest_listen_transcript_on(date(22), 4).await.unwrap();
        let ids: Vec<u64> = read.iter().map(|sentence| sentence.id).collect();
        assert_eq!(ids, vec![3, 4, 5, 6], "the newest 4 of that date only");
        assert!(repo
            .newest_listen_transcript_on(date(20), 4)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn the_max_sentence_id_spans_every_date() {
        let repo = repo().await;
        assert_eq!(
            repo.max_listen_transcript_sentence_id().await.unwrap(),
            None
        );
        repo.insert_listen_transcript(&[row(21, 40, "old"), row(22, 3, "new")])
            .await
            .unwrap();
        assert_eq!(
            repo.max_listen_transcript_sentence_id().await.unwrap(),
            Some(40)
        );
    }

    #[tokio::test]
    async fn retention_deletes_only_dates_before_the_cutoff_and_nothing_else() {
        let repo = repo().await;
        repo.insert_listen_transcript(&[row(14, 1, "a"), row(15, 2, "b"), row(16, 3, "c")])
            .await
            .unwrap();
        repo.set_app_setting("listen.mics", "[]").await.unwrap();
        assert_eq!(
            repo.prune_listen_transcript_before(date(15)).await.unwrap(),
            1
        );
        assert!(repo
            .newest_listen_transcript_on(date(14), 10)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            repo.newest_listen_transcript_on(date(15), 10)
                .await
                .unwrap()
                .len(),
            1,
            "the cutoff date itself is kept"
        );
        assert_eq!(
            repo.get_app_setting("listen.mics")
                .await
                .unwrap()
                .as_deref(),
            Some("[]"),
            "no other table is touched"
        );
    }

    #[tokio::test]
    async fn a_value_that_does_not_fit_is_a_typed_refusal_and_writes_nothing() {
        let repo = repo().await;
        let mut big = row(22, 2, "big");
        big.sentence.end_ms = u64::MAX;
        let err = repo
            .insert_listen_transcript(&[row(22, 1, "fine"), big])
            .await
            .unwrap_err();
        assert_eq!(
            err.downcast_ref::<ListenTranscriptError>(),
            Some(&ListenTranscriptError::TooLarge {
                field: "end_ms",
                value: u64::MAX
            })
        );
        assert!(repo
            .newest_listen_transcript_on(date(22), 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_negative_stored_value_is_a_typed_refusal() {
        let repo = repo().await;
        repo.insert_listen_transcript(&[row(22, 1, "a")])
            .await
            .unwrap();
        repo.db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "UPDATE listen_transcript SET start_ms = -5".to_string(),
            ))
            .await
            .unwrap();
        let err = repo
            .newest_listen_transcript_on(date(22), 10)
            .await
            .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<ListenTranscriptError>(),
            Some(ListenTranscriptError::Negative {
                field: "start_ms",
                value: -5,
                ..
            })
        ));
    }
}
