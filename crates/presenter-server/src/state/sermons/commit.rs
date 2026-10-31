//! The one write path of a sermon (#851 spec §7.5): generate the slides from
//! the new item list, store items + slides in ONE transaction guarded by the
//! expected revision, publish `BibleSlidesChanged`, log one INFO line.

use std::time::{Duration, Instant};

use chrono::NaiveDate;
use presenter_core::BiblePresentationId;
use presenter_persistence::{
    RepositoryError, SermonItemRecord, SermonRecord, SermonWrite, SERMON_NOT_FOUND,
};
use uuid::Uuid;

use super::super::AppState;
use super::{SermonOp, SermonOrigin};
use crate::live::LiveEvent;

/// What a committed write produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommittedSermon {
    pub(crate) slide_count: usize,
    /// Operator-facing warnings of the generation (an unavailable NLT).
    pub(crate) warnings: Vec<String>,
}

/// The new state of one sermon, computed in memory before the commit.
pub(crate) struct SermonChange<'a> {
    pub(crate) op: SermonOp,
    pub(crate) origin: &'a SermonOrigin,
    pub(crate) expected_revision: i64,
    /// The sermon with its new title, date and secondary translation.
    pub(crate) sermon: SermonRecord,
    /// The new item list, in order; `slide_count` is filled in here.
    pub(crate) items: Vec<SermonItemRecord>,
    /// The item the write is about (the log line's `item_id`).
    pub(crate) item_id: Option<Uuid>,
}

/// The fields of one `presenter::bible::sermon` line (spec §13).
struct SermonLine<'a> {
    op: SermonOp,
    sermon_id: BiblePresentationId,
    item_id: Option<Uuid>,
    revision: i64,
    items: usize,
    slides: usize,
    origin: &'a SermonOrigin,
    elapsed: Duration,
    warning: Option<&'a str>,
}

fn log_sermon_write(line: &SermonLine<'_>) {
    let item_id = line
        .item_id
        .map_or_else(|| "-".to_string(), |id| id.to_string());
    tracing::info!(
        target: "presenter::bible::sermon",
        op = line.op.as_str(),
        sermon_id = %line.sermon_id,
        item_id = %item_id,
        revision = line.revision,
        items = line.items,
        slides = line.slides,
        author = line.origin.author.as_deref().unwrap_or("-"),
        client_ip = line.origin.client_ip.as_str(),
        ms = u64::try_from(line.elapsed.as_millis()).unwrap_or(u64::MAX),
        warning = line.warning.unwrap_or("-"),
        "sermon write"
    );
}

fn is_conflict(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<RepositoryError>(),
        Some(RepositoryError::Conflict(_))
    )
}

impl AppState {
    /// Generate, store (409 when `expected_revision` is stale), publish, log.
    pub(crate) async fn commit_sermon(
        &self,
        change: SermonChange<'_>,
    ) -> anyhow::Result<CommittedSermon> {
        let started = Instant::now();
        let generated = self
            .generate_sermon_slides(&change.sermon, &change.items)
            .await?;
        let mut items = change.items;
        for (item, count) in items.iter_mut().zip(&generated.slide_counts) {
            item.slide_count = *count;
        }
        let write = SermonWrite {
            expected_revision: change.expected_revision,
            title: change.sermon.title.clone(),
            service_date: change.sermon.service_date,
            secondary_translation_code: change.sermon.secondary_translation_code.clone(),
            items,
            slides: generated.slides,
        };
        let revision = self
            .repository
            .save_sermon_change(change.sermon.id, &write)
            .await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: change.sermon.id.to_string(),
        });
        log_sermon_write(&SermonLine {
            op: change.op,
            sermon_id: change.sermon.id,
            item_id: change.item_id,
            revision,
            items: write.items.len(),
            slides: write.slides.len(),
            origin: change.origin,
            elapsed: started.elapsed(),
            warning: generated.warnings.first().map(String::as_str),
        });
        Ok(CommittedSermon {
            slide_count: write.slides.len(),
            warnings: generated.warnings,
        })
    }

    /// Generate the slides of the sermon's CURRENT items again, at its current
    /// revision (the character-limit pass).
    async fn regenerate_current(
        &self,
        id: BiblePresentationId,
        origin: &SermonOrigin,
    ) -> anyhow::Result<CommittedSermon> {
        let snapshot = self
            .repository
            .fetch_sermon(id)
            .await?
            .ok_or(RepositoryError::NotFound(SERMON_NOT_FOUND))?;
        self.commit_sermon(SermonChange {
            op: SermonOp::Regenerate,
            origin,
            expected_revision: snapshot.sermon.revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: None,
        })
        .await
    }

    /// Spec §7.4: a new character limit regenerates the slides of every
    /// sermon dated `today` or later (a few rows), retrying once on a revision
    /// conflict. A failure is a WARN and never fails the preference write.
    pub(crate) async fn regenerate_future_sermons(&self, today: NaiveDate) {
        let ids = match self.repository.sermon_ids_from(today).await {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!(
                    target: "presenter::bible::sermon",
                    error = %err,
                    "character limit changed: future sermons not listed"
                );
                return;
            }
        };
        let origin = SermonOrigin::system();
        let mut slides = 0;
        for id in &ids {
            let first = self.regenerate_current(*id, &origin).await;
            let result = match first {
                Err(err) if is_conflict(&err) => self.regenerate_current(*id, &origin).await,
                other => other,
            };
            match result {
                Ok(committed) => {
                    slides += committed.slide_count;
                    for warning in &committed.warnings {
                        tracing::warn!(
                            target: "presenter::bible::sermon",
                            sermon_id = %id,
                            warning = warning.as_str(),
                            "character limit changed: sermon regenerated with a warning"
                        );
                    }
                }
                Err(err) => tracing::warn!(
                    target: "presenter::bible::sermon",
                    sermon_id = %id,
                    error = %err,
                    "character limit changed: sermon not regenerated"
                ),
            }
        }
        tracing::info!(
            target: "presenter::bible::sermon",
            sermons = ids.len(),
            slides,
            "character limit changed: future sermons regenerated"
        );
    }
}
