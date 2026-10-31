//! The item upkeep writes (#851 spec §7.5, §11): change an item's translation,
//! refresh its text, delete it, reorder the items, change the sermon's title,
//! date or secondary translation, regenerate the slides. Same shape as
//! `write.rs`: load, 409 on a stale revision, compute, `commit_and_view`.

use chrono::NaiveDate;
use presenter_core::bible::sermon::{retain_unchanged_edits, ItemEdits};
use presenter_core::BiblePresentationId;
use presenter_persistence::{RepositoryError, SermonItemRecord, SERMON_CONFLICT};
use uuid::Uuid;

use super::super::AppState;
use super::commit::SermonChange;
use super::snapshot::ItemRange;
use super::view::SermonDetail;
use super::write::{check_revision, installed, item_mut};
use super::{SermonOp, SermonOrigin, SermonRefusal};

/// `PATCH /bible/sermons/{id}`: `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SermonPatch {
    pub(crate) title: Option<String>,
    pub(crate) service_date: Option<NaiveDate>,
    pub(crate) secondary_translation: Option<Option<String>>,
}

fn non_empty_title(title: &str) -> Result<String, SermonRefusal> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(SermonRefusal::new("Názov kázne nesmie byť prázdny"));
    }
    Ok(trimmed.to_string())
}

impl AppState {
    /// Re-snapshot one item in another translation; its edits are cleared
    /// (the editor confirms first, spec §7.5).
    pub(crate) async fn change_sermon_item_translation(
        &self,
        id: BiblePresentationId,
        item_id: Uuid,
        expected_revision: i64,
        translation: &str,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        self.resnapshot_item(id, item_id, expected_revision, Some(translation), origin)
            .await
    }

    /// "Obnoviť text": re-snapshot one item; the edits of verses whose text is
    /// unchanged stay, the rest are cleared.
    pub(crate) async fn refresh_sermon_item_source(
        &self,
        id: BiblePresentationId,
        item_id: Uuid,
        expected_revision: i64,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        self.resnapshot_item(id, item_id, expected_revision, None, origin)
            .await
    }

    async fn resnapshot_item(
        &self,
        id: BiblePresentationId,
        item_id: Uuid,
        expected_revision: i64,
        translation: Option<&str>,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        let item = item_mut(&mut snapshot.items, item_id)?;
        let range = ItemRange {
            translation: translation
                .unwrap_or(item.translation_code.as_str())
                .to_string(),
            book_code: item.book_code.clone(),
            chapter: item.chapter,
            verse_start: item.verse_start,
            verse_end: item.verse_end,
        };
        let fresh = self.snapshot_item(&range).await?;
        let edits = match translation {
            Some(_) => ItemEdits::default(),
            None => retain_unchanged_edits(&item.source, &fresh.source, &item.edits),
        };
        *item = SermonItemRecord {
            id: item.id,
            edits,
            created_at: item.created_at,
            ..fresh
        };
        let op = match translation {
            Some(_) => SermonOp::Translation,
            None => SermonOp::Refresh,
        };
        self.commit_and_view(SermonChange {
            op,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: Some(item_id),
        })
        .await
    }

    pub(crate) async fn delete_sermon_item(
        &self,
        id: BiblePresentationId,
        item_id: Uuid,
        expected_revision: i64,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        item_mut(&mut snapshot.items, item_id)?;
        snapshot.items.retain(|item| item.id != item_id);
        self.commit_and_view(SermonChange {
            op: SermonOp::DeleteItem,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: Some(item_id),
        })
        .await
    }

    /// The items in the order of `item_ids`, which must be exactly the stored
    /// set (a different set is a stale view: 409, spec §7.5).
    pub(crate) async fn reorder_sermon_items(
        &self,
        id: BiblePresentationId,
        expected_revision: i64,
        item_ids: &[Uuid],
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        let mut reordered = Vec::with_capacity(snapshot.items.len());
        for item_id in item_ids {
            let Some(index) = snapshot.items.iter().position(|item| item.id == *item_id) else {
                return Err(RepositoryError::Conflict(SERMON_CONFLICT).into());
            };
            reordered.push(snapshot.items.swap_remove(index));
        }
        if !snapshot.items.is_empty() {
            return Err(RepositoryError::Conflict(SERMON_CONFLICT).into());
        }
        self.commit_and_view(SermonChange {
            op: SermonOp::Reorder,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: reordered,
            item_id: None,
        })
        .await
    }

    /// Title (= the presentation name), service date, secondary translation.
    pub(crate) async fn update_sermon(
        &self,
        id: BiblePresentationId,
        expected_revision: i64,
        patch: SermonPatch,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let mut snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        if let Some(title) = patch.title.as_deref() {
            snapshot.sermon.title = non_empty_title(title)?;
        }
        if let Some(date) = patch.service_date {
            snapshot.sermon.service_date = date;
        }
        if let Some(secondary) = patch.secondary_translation {
            let translations = self.list_bible_translations().await?;
            snapshot.sermon.secondary_translation_code = match secondary {
                Some(code) => Some(installed(&translations, &code, true)?),
                None => None,
            };
        }
        self.commit_and_view(SermonChange {
            op: SermonOp::Update,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: None,
        })
        .await
    }

    /// `POST /bible/sermons/{id}/regenerate`: the same items, the slides
    /// generated again (retries the secondary line after an NLT failure).
    pub(crate) async fn regenerate_sermon(
        &self,
        id: BiblePresentationId,
        expected_revision: i64,
        origin: &SermonOrigin,
    ) -> anyhow::Result<SermonDetail> {
        let snapshot = self.load_sermon(id).await?;
        check_revision(&snapshot, expected_revision)?;
        self.commit_and_view(SermonChange {
            op: SermonOp::Regenerate,
            origin,
            expected_revision,
            sermon: snapshot.sermon,
            items: snapshot.items,
            item_id: None,
        })
        .await
    }
}
