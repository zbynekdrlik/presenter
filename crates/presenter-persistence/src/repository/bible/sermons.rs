//! Sermons (#851 spec §7.1, §7.5). A sermon IS a Bible presentation (its slides
//! are what the operator, the tablet and the walls already play) plus a
//! `bible_sermons` row and its source items. The server generates the slides
//! from the items; [`Repository::save_sermon_change`] stores items and slides
//! in ONE transaction guarded by the sermon's `revision`, so a stale write from
//! a second device is a 409 and never half-applied.

use std::collections::HashMap;

use anyhow::Context;
use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
};
use tracing::instrument;
use uuid::Uuid;

use super::presentations::bible_slide_to_active_model;
use crate::entities::{bible_presentation, bible_sermon, bible_sermon_item, bible_slide};
use crate::repository::util::{parse_uuid, RepositoryError};
use crate::repository::Repository;
use presenter_core::bible::sermon::{ItemEdits, SourceVerse};
use presenter_core::{BiblePresentationId, BiblePresentationSlide};

/// The 409 of every stale sermon write (spec §7.5).
pub const SERMON_CONFLICT: &str = "sermon changed on another device";
pub const SERMON_NOT_FOUND: &str = "sermon not found";
const DATE_FORMAT: &str = "%Y-%m-%d";

/// A sermon to create: the presentation's name is `title`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSermon {
    pub title: String,
    pub service_date: NaiveDate,
    pub default_translation_code: String,
    pub secondary_translation_code: Option<String>,
    pub author_email: Option<String>,
}

/// The `bible_sermons` row with its presentation's name as the title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SermonRecord {
    pub id: BiblePresentationId,
    pub title: String,
    pub service_date: NaiveDate,
    pub default_translation_code: String,
    pub secondary_translation_code: Option<String>,
    pub author_email: Option<String>,
    pub revision: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One source item; its order is its position in the sermon's item list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SermonItemRecord {
    pub id: Uuid,
    pub translation_code: String,
    pub book_code: String,
    pub book_number: u16,
    pub book_name: String,
    pub chapter: u16,
    pub verse_start: u16,
    pub verse_end: u16,
    pub source: Vec<SourceVerse>,
    pub edits: ItemEdits,
    pub slide_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A sermon with its items in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SermonSnapshot {
    pub sermon: SermonRecord,
    pub items: Vec<SermonItemRecord>,
}

/// One row of the sermon list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SermonSummary {
    pub id: BiblePresentationId,
    pub title: String,
    pub service_date: NaiveDate,
    pub author_email: Option<String>,
    pub item_count: usize,
    pub slide_count: usize,
    pub revision: i64,
    pub updated_at: DateTime<Utc>,
}

/// The whole new state of one sermon write: meta, items in order (each with
/// its `slide_count`) and the generated slides.
#[derive(Debug, Clone, PartialEq)]
pub struct SermonWrite {
    pub expected_revision: i64,
    pub title: String,
    pub service_date: NaiveDate,
    pub secondary_translation_code: Option<String>,
    pub items: Vec<SermonItemRecord>,
    pub slides: Vec<BiblePresentationSlide>,
}

#[derive(Debug, FromQueryResult)]
struct ItemTotals {
    presentation_id: String,
    item_count: i64,
    slide_count: Option<i64>,
}

impl Repository {
    /// Create the presentation and its sermon row (revision 0) in one
    /// transaction.
    #[instrument(skip_all)]
    pub async fn create_sermon(&self, new: &NewSermon) -> anyhow::Result<SermonRecord> {
        let id = BiblePresentationId::new();
        let now = Utc::now();
        let txn = self.db.begin().await?;
        bible_presentation::Entity::insert(bible_presentation::ActiveModel {
            id: Set(id.to_string()),
            name: Set(new.title.clone()),
            created_at: Set(now.into()),
        })
        .exec(&txn)
        .await?;
        bible_sermon::Entity::insert(bible_sermon::ActiveModel {
            presentation_id: Set(id.to_string()),
            service_date: Set(new.service_date.format(DATE_FORMAT).to_string()),
            default_translation_code: Set(new.default_translation_code.clone()),
            secondary_translation_code: Set(new.secondary_translation_code.clone()),
            author_email: Set(new.author_email.clone()),
            revision: Set(0),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        })
        .exec(&txn)
        .await?;
        txn.commit().await?;
        Ok(SermonRecord {
            id,
            title: new.title.clone(),
            service_date: new.service_date,
            default_translation_code: new.default_translation_code.clone(),
            secondary_translation_code: new.secondary_translation_code.clone(),
            author_email: new.author_email.clone(),
            revision: 0,
            created_at: now,
            updated_at: now,
        })
    }

    /// The sermon with its items in order; `None` when `id` is no sermon.
    #[instrument(skip_all)]
    pub async fn fetch_sermon(
        &self,
        id: BiblePresentationId,
    ) -> anyhow::Result<Option<SermonSnapshot>> {
        let id_str = id.to_string();
        let Some(row) = bible_sermon::Entity::find_by_id(id_str.clone())
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        let title = bible_presentation::Entity::find_by_id(id_str.clone())
            .one(&self.db)
            .await?
            .map(|presentation| presentation.name)
            .ok_or(RepositoryError::NotFound(SERMON_NOT_FOUND))?;
        let items = bible_sermon_item::Entity::find()
            .filter(bible_sermon_item::Column::PresentationId.eq(id_str))
            .order_by_asc(bible_sermon_item::Column::ItemOrder)
            .all(&self.db)
            .await?
            .into_iter()
            .map(item_from_model)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Some(SermonSnapshot {
            sermon: sermon_from_model(row, title)?,
            items,
        }))
    }

    /// Every sermon, by service date, with its item and slide counts.
    #[instrument(skip_all)]
    pub async fn list_sermon_summaries(&self) -> anyhow::Result<Vec<SermonSummary>> {
        let rows = bible_sermon::Entity::find()
            .order_by_asc(bible_sermon::Column::ServiceDate)
            .order_by_asc(bible_sermon::Column::CreatedAt)
            .all(&self.db)
            .await?;
        let names: HashMap<String, String> = bible_presentation::Entity::find()
            .filter(
                bible_presentation::Column::Id
                    .is_in(rows.iter().map(|row| row.presentation_id.clone())),
            )
            .all(&self.db)
            .await?
            .into_iter()
            .map(|presentation| (presentation.id, presentation.name))
            .collect();
        let totals = item_totals(&self.db).await?;
        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            let (item_count, slide_count) =
                totals.get(&row.presentation_id).copied().unwrap_or((0, 0));
            let title = names.get(&row.presentation_id).cloned().unwrap_or_default();
            let sermon = sermon_from_model(row, title)?;
            summaries.push(SermonSummary {
                id: sermon.id,
                title: sermon.title,
                service_date: sermon.service_date,
                author_email: sermon.author_email,
                item_count,
                slide_count,
                revision: sermon.revision,
                updated_at: sermon.updated_at,
            });
        }
        Ok(summaries)
    }

    /// The sermons whose service date is `date`, oldest first.
    #[instrument(skip_all)]
    pub async fn sermon_ids_on(&self, date: NaiveDate) -> anyhow::Result<Vec<BiblePresentationId>> {
        let rows = bible_sermon::Entity::find()
            .filter(bible_sermon::Column::ServiceDate.eq(date.format(DATE_FORMAT).to_string()))
            .order_by_asc(bible_sermon::Column::CreatedAt)
            .all(&self.db)
            .await?;
        sermon_ids(rows)
    }

    /// The sermons whose service date is `date` or later, by date.
    #[instrument(skip_all)]
    pub async fn sermon_ids_from(
        &self,
        date: NaiveDate,
    ) -> anyhow::Result<Vec<BiblePresentationId>> {
        let rows = bible_sermon::Entity::find()
            .filter(bible_sermon::Column::ServiceDate.gte(date.format(DATE_FORMAT).to_string()))
            .order_by_asc(bible_sermon::Column::ServiceDate)
            .all(&self.db)
            .await?;
        sermon_ids(rows)
    }

    /// Whether the presentation is a sermon (its slides are generated).
    #[instrument(skip_all)]
    pub async fn is_sermon(&self, id: BiblePresentationId) -> anyhow::Result<bool> {
        Ok(bible_sermon::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .is_some())
    }

    /// The service date of every sermon, by presentation id (the
    /// `sermon_date` of presentation summaries and details).
    pub(super) async fn sermon_dates(&self) -> anyhow::Result<HashMap<String, NaiveDate>> {
        let mut dates = HashMap::new();
        for row in bible_sermon::Entity::find().all(&self.db).await? {
            let date = parse_date(&row.service_date)?;
            dates.insert(row.presentation_id, date);
        }
        Ok(dates)
    }

    /// The service date of one presentation; `None` for a plain presentation.
    pub(super) async fn sermon_date_of(
        &self,
        presentation_id: &str,
    ) -> anyhow::Result<Option<NaiveDate>> {
        bible_sermon::Entity::find_by_id(presentation_id.to_string())
            .one(&self.db)
            .await?
            .map(|row| parse_date(&row.service_date))
            .transpose()
    }

    /// Store one sermon write (spec §7.5) in ONE transaction: bump the revision
    /// only when it still equals `expected_revision` (else `Conflict` → 409, or
    /// `NotFound` → 404 when the sermon is gone), then the meta, the items and
    /// the slides. Returns the new revision.
    #[instrument(skip_all)]
    pub async fn save_sermon_change(
        &self,
        id: BiblePresentationId,
        write: &SermonWrite,
    ) -> anyhow::Result<i64> {
        let id_str = id.to_string();
        let now: DateTimeWithTimeZone = Utc::now().into();
        let txn = self.db.begin().await?;
        let bumped = bible_sermon::Entity::update_many()
            .col_expr(
                bible_sermon::Column::Revision,
                Expr::col(bible_sermon::Column::Revision).add(1),
            )
            .col_expr(bible_sermon::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                bible_sermon::Column::ServiceDate,
                Expr::value(write.service_date.format(DATE_FORMAT).to_string()),
            )
            .col_expr(
                bible_sermon::Column::SecondaryTranslationCode,
                Expr::value(write.secondary_translation_code.clone()),
            )
            .filter(bible_sermon::Column::PresentationId.eq(id_str.clone()))
            .filter(bible_sermon::Column::Revision.eq(write.expected_revision))
            .exec(&txn)
            .await?;
        if bumped.rows_affected == 0 {
            let exists = bible_sermon::Entity::find_by_id(id_str.clone())
                .one(&txn)
                .await?
                .is_some();
            let refusal = if exists {
                RepositoryError::Conflict(SERMON_CONFLICT)
            } else {
                RepositoryError::NotFound(SERMON_NOT_FOUND)
            };
            return Err(refusal.into());
        }
        bible_presentation::Entity::update_many()
            .col_expr(
                bible_presentation::Column::Name,
                Expr::value(write.title.clone()),
            )
            .filter(bible_presentation::Column::Id.eq(id_str.clone()))
            .exec(&txn)
            .await?;
        replace_items(&txn, &id_str, &write.items).await?;
        replace_slides(&txn, &id_str, &write.slides).await?;
        txn.commit().await?;
        Ok(write.expected_revision + 1)
    }
}

async fn replace_items<C: ConnectionTrait>(
    txn: &C,
    presentation_id: &str,
    items: &[SermonItemRecord],
) -> anyhow::Result<()> {
    bible_sermon_item::Entity::delete_many()
        .filter(bible_sermon_item::Column::PresentationId.eq(presentation_id.to_string()))
        .exec(txn)
        .await?;
    for (order, item) in items.iter().enumerate() {
        bible_sermon_item::Entity::insert(item_to_active(item, presentation_id, order)?)
            .exec(txn)
            .await?;
    }
    Ok(())
}

async fn replace_slides<C: ConnectionTrait>(
    txn: &C,
    presentation_id: &str,
    slides: &[BiblePresentationSlide],
) -> anyhow::Result<()> {
    bible_slide::Entity::delete_many()
        .filter(bible_slide::Column::PresentationId.eq(presentation_id.to_string()))
        .exec(txn)
        .await?;
    for (order, slide) in slides.iter().enumerate() {
        let mut normalized = slide.clone();
        normalized.order = u32::try_from(order).context("too many sermon slides")?;
        bible_slide::Entity::insert(bible_slide_to_active_model(&normalized, presentation_id)?)
            .exec(txn)
            .await?;
    }
    Ok(())
}

/// Items and summed slide counts per sermon.
async fn item_totals<C: ConnectionTrait>(
    db: &C,
) -> anyhow::Result<HashMap<String, (usize, usize)>> {
    let rows = bible_sermon_item::Entity::find()
        .select_only()
        .column(bible_sermon_item::Column::PresentationId)
        .column_as(
            Expr::col(bible_sermon_item::Column::Id).count(),
            "item_count",
        )
        .column_as(
            Expr::col(bible_sermon_item::Column::SlideCount).sum(),
            "slide_count",
        )
        .group_by(bible_sermon_item::Column::PresentationId)
        .into_model::<ItemTotals>()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let items = usize::try_from(row.item_count).unwrap_or(0);
            let slides = usize::try_from(row.slide_count.unwrap_or(0)).unwrap_or(0);
            (row.presentation_id, (items, slides))
        })
        .collect())
}

fn sermon_ids(rows: Vec<bible_sermon::Model>) -> anyhow::Result<Vec<BiblePresentationId>> {
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        ids.push(BiblePresentationId::from_uuid(parse_uuid(
            &row.presentation_id,
        )?));
    }
    Ok(ids)
}

fn parse_date(raw: &str) -> anyhow::Result<NaiveDate> {
    NaiveDate::parse_from_str(raw, DATE_FORMAT)
        .with_context(|| format!("invalid sermon service_date '{raw}' in the database"))
}

fn sermon_from_model(model: bible_sermon::Model, title: String) -> anyhow::Result<SermonRecord> {
    Ok(SermonRecord {
        id: BiblePresentationId::from_uuid(parse_uuid(&model.presentation_id)?),
        title,
        service_date: parse_date(&model.service_date)?,
        default_translation_code: model.default_translation_code,
        secondary_translation_code: model.secondary_translation_code,
        author_email: model.author_email,
        revision: model.revision,
        created_at: model.created_at.with_timezone(&Utc),
        updated_at: model.updated_at.with_timezone(&Utc),
    })
}

fn stored_u16(value: i32, column: &str) -> anyhow::Result<u16> {
    u16::try_from(value).with_context(|| format!("sermon item {column} {value} out of range"))
}

fn item_from_model(model: bible_sermon_item::Model) -> anyhow::Result<SermonItemRecord> {
    Ok(SermonItemRecord {
        id: parse_uuid(&model.id)?,
        translation_code: model.translation_code,
        book_code: model.book_code,
        book_number: stored_u16(model.book_number, "book_number")?,
        book_name: model.book_name,
        chapter: stored_u16(model.chapter, "chapter")?,
        verse_start: stored_u16(model.verse_start, "verse_start")?,
        verse_end: stored_u16(model.verse_end, "verse_end")?,
        source: serde_json::from_str(&model.source_json).context("sermon item source_json")?,
        edits: serde_json::from_str(&model.edits_json).context("sermon item edits_json")?,
        slide_count: u32::try_from(model.slide_count).context("sermon item slide_count")?,
        created_at: model.created_at.with_timezone(&Utc),
        updated_at: model.updated_at.with_timezone(&Utc),
    })
}

fn item_to_active(
    item: &SermonItemRecord,
    presentation_id: &str,
    order: usize,
) -> anyhow::Result<bible_sermon_item::ActiveModel> {
    Ok(bible_sermon_item::ActiveModel {
        id: Set(item.id.to_string()),
        presentation_id: Set(presentation_id.to_string()),
        item_order: Set(i32::try_from(order).context("too many sermon items")?),
        translation_code: Set(item.translation_code.clone()),
        book_code: Set(item.book_code.clone()),
        book_number: Set(i32::from(item.book_number)),
        book_name: Set(item.book_name.clone()),
        chapter: Set(i32::from(item.chapter)),
        verse_start: Set(i32::from(item.verse_start)),
        verse_end: Set(i32::from(item.verse_end)),
        source_json: Set(serde_json::to_string(&item.source)?),
        edits_json: Set(serde_json::to_string(&item.edits)?),
        slide_count: Set(i32::try_from(item.slide_count).context("sermon item slide_count")?),
        created_at: Set(item.created_at.into()),
        updated_at: Set(item.updated_at.into()),
    })
}

#[cfg(test)]
mod tests;
