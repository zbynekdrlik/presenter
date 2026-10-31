//! Sermons (#851 spec §7.1, §7.5). A sermon IS a Bible presentation (its slides
//! are what the operator, the tablet and the walls already play) plus a
//! `bible_sermons` row and its source items. The server generates the slides
//! from the items; `save_sermon_change` (P2b) stores items and slides in ONE
//! transaction guarded by the sermon's `revision`, so a stale write from a
//! second device is a 409 and never half-applied.

use std::collections::HashMap;

use anyhow::Context;
use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait};
use tracing::instrument;
use uuid::Uuid;

use crate::entities::{bible_presentation, bible_sermon, bible_sermon_item};
use crate::repository::util::{parse_uuid, RepositoryError};
use crate::repository::Repository;
use presenter_core::bible::sermon::{ItemEdits, SourceVerse};
use presenter_core::BiblePresentationId;

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

#[cfg(test)]
mod tests;
