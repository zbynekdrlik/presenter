//! The sermon writes of the editor's first steps (#851 spec §7.5, §11): create,
//! add an item, edit an item, delete. Each write loads the sermon, refuses a
//! stale `expected_revision` with 409, computes the new item list in memory and
//! commits it through `commit_sermon` (which re-checks the revision inside the
//! transaction). The answer is the full `SermonDetail`. The item upkeep writes
//! live in `maintain.rs` (P4d).

use std::time::Instant;

use chrono::{NaiveDate, Utc};
use presenter_core::bible::sermon::{
    default_sermon_title, default_service_date, normalize_edits, ItemEdits,
};
use presenter_core::{BiblePresentationId, BibleTranslation};
use presenter_persistence::{
    NewSermon, RepositoryError, SermonItemRecord, SermonSnapshot, SERMON_CONFLICT, SERMON_NOT_FOUND,
};
use uuid::Uuid;

use super::super::AppState;
use super::commit::{log_sermon_write, SermonChange, SermonLine};
use super::snapshot::ItemRange;
use super::view::SermonDetail;
use super::{SermonOp, SermonOrigin, SermonRefusal};
use crate::bible_remote;
use crate::live::LiveEvent;

const ITEM_NOT_FOUND: &str = "sermon item not found";

/// `POST /bible/sermons`: every field optional (spec §7.6 defaults).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SermonDraft {
    pub(crate) title: Option<String>,
    pub(crate) service_date: Option<NaiveDate>,
    pub(crate) default_translation: Option<String>,
    /// `None` = the Bible preference's secondary; `Some(None)` = no line.
    pub(crate) secondary_translation: Option<Option<String>>,
}

pub(super) fn check_revision(snapshot: &SermonSnapshot, expected: i64) -> anyhow::Result<()> {
    if snapshot.sermon.revision == expected {
        Ok(())
    } else {
        Err(RepositoryError::Conflict(SERMON_CONFLICT).into())
    }
}

pub(super) fn item_mut(
    items: &mut [SermonItemRecord],
    item_id: Uuid,
) -> anyhow::Result<&mut SermonItemRecord> {
    items
        .iter_mut()
        .find(|item| item.id == item_id)
        .ok_or_else(|| RepositoryError::NotFound(ITEM_NOT_FOUND).into())
}

/// An installed translation's own code; the NLT only when `allow_remote`.
pub(super) fn installed(
    translations: &[BibleTranslation],
    code: &str,
    allow_remote: bool,
) -> Result<String, SermonRefusal> {
    if !allow_remote && bible_remote::is_remote_translation(code) {
        return Err(SermonRefusal::new(
            "NLT sa do kázne neukladá — vyber iný preklad",
        ));
    }
    translations
        .iter()
        .find(|translation| translation.code.eq_ignore_ascii_case(code.trim()))
        .map(|translation| translation.code.clone())
        .ok_or_else(|| SermonRefusal::new(format!("Preklad {code} nie je nainštalovaný")))
}

impl AppState {
    pub(super) async fn load_sermon(
        &self,
        id: BiblePresentationId,
    ) -> anyhow::Result<SermonSnapshot> {
        Ok(self
            .repository
            .fetch_sermon(id)
            .await?
            .ok_or(RepositoryError::NotFound(SERMON_NOT_FOUND))?)
    }

    /// Commit, then answer the stored sermon with the write's warnings.
    pub(super) async fn commit_and_view(
        &self,
        change: SermonChange<'_>,
    ) -> anyhow::Result<SermonDetail> {
        let id = change.sermon.id;
        let committed = self.commit_sermon(change).await?;
        let mut detail = self
            .sermon_detail(id)
            .await?
            .ok_or(RepositoryError::NotFound(SERMON_NOT_FOUND))?;
        detail.warnings = committed.warnings;
        Ok(detail)
    }

    /// A new, empty sermon (revision 0) with the spec §7.6 defaults: the next
    /// Sunday, "Kázeň D. M. YYYY", the Bible preference's translations.
    pub(crate) async fn create_sermon(
        &self,
        draft: SermonDraft,
        today: NaiveDate,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let started = Instant::now();
        let preferences = self.get_bible_preferences().await?;
        let translations = self.list_bible_translations().await?;
        let default_translation = match draft.default_translation.as_deref() {
            Some(code) => installed(&translations, code, false)?,
            None => preferences
                .main_translation
                .as_deref()
                .and_then(|code| installed(&translations, code, false).ok())
                .or_else(|| {
                    translations
                        .iter()
                        .find(|t| !bible_remote::is_remote_translation(&t.code))
                        .map(|t| t.code.clone())
                })
                .ok_or_else(|| SermonRefusal::new("Žiadny preklad nie je nainštalovaný"))?,
        };
        let secondary = match draft.secondary_translation {
            Some(Some(code)) => Some(installed(&translations, &code, true)?),
            Some(None) => None,
            None => preferences
                .secondary_translation
                .as_deref()
                .and_then(|code| installed(&translations, code, true).ok()),
        };
        let service_date = draft
            .service_date
            .unwrap_or_else(|| default_service_date(today));
        let title = match draft
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            Some(title) => title.to_string(),
            None => default_sermon_title(service_date),
        };
        let sermon = self
            .repository
            .create_sermon(&NewSermon {
                title,
                service_date,
                default_translation_code: default_translation,
                secondary_translation_code: secondary,
                author_email: origin.author.clone(),
            })
            .await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: sermon.id.to_string(),
        });
        log_sermon_write(&SermonLine::new(
            SermonOp::Create,
            sermon.id,
            origin,
            started,
        ));
        self.sermon_detail(sermon.id)
            .await?
            .ok_or_else(|| RepositoryError::NotFound(SERMON_NOT_FOUND).into())
    }

    /// Add the database text of `range` as an item at `position` (default: the
    /// end).
    pub(crate) async fn add_sermon_item(
        &self,
        id: BiblePresentationId,
        expected_revision: i64,
        position: Option<usize>,
        range: &ItemRange,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        let item = self.snapshot_item(range).await?;
        let item_id = item.id;
        let position = position
            .unwrap_or(snapshot.items.len())
            .min(snapshot.items.len());
        snapshot.items.insert(position, item);
        self.commit_and_view(SermonChange {
            op: SermonOp::Add,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: Some(item_id),
        })
        .await
    }

    /// Replace one item's edits (validated and normalized against its snapshot).
    pub(crate) async fn set_sermon_item_edits(
        &self,
        id: BiblePresentationId,
        item_id: Uuid,
        expected_revision: i64,
        edits: &ItemEdits,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        let item = item_mut(&mut snapshot.items, item_id)?;
        item.edits = normalize_edits(&item.source, edits)
            .map_err(|err| SermonRefusal::new(err.to_string()))?;
        item.updated_at = Utc::now();
        self.commit_and_view(SermonChange {
            op: SermonOp::Edit,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: Some(item_id),
        })
        .await
    }

    /// Delete the sermon's presentation; the sermon and its items cascade.
    /// Only a sermon: a plain presentation is not found here.
    pub(crate) async fn delete_sermon(
        &self,
        id: BiblePresentationId,
        origin: &SermonOrigin,
    ) -> anyhow::Result<()> {
        let started = Instant::now();
        let snapshot = self.load_sermon(id).await?;
        self.repository.delete_bible_presentation(id).await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: id.to_string(),
        });
        let mut line = SermonLine::new(SermonOp::Delete, id, origin, started);
        line.revision = snapshot.sermon.revision;
        line.items = snapshot.items.len();
        log_sermon_write(&line);
        Ok(())
    }
}
