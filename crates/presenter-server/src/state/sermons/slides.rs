//! A sermon's slides edited from the operator console (#917): one kind of Bible
//! presentation for the operator. A sermon's slides are generated from its items,
//! so deleting a slide deletes the item that produced it, and a new slide order
//! orders the items by where each item's first named slide stands. The slide
//! text itself stays generated (`update_bible_slide` is still refused, `guard.rs`).

use std::collections::HashMap;

use presenter_core::{BiblePresentation, BiblePresentationId, BibleSlideId};
use presenter_persistence::{RepositoryError, SermonItemRecord};
use uuid::Uuid;

use super::super::AppState;
use super::commit::SermonChange;
use super::generate::stable_slide_id;
use super::{SermonOp, SermonOrigin};

const SLIDE_NOT_FOUND: &str = "slide not found in presentation";

/// Which item generated each slide (the stable ids of `generate.rs`).
fn owners(items: &[SermonItemRecord]) -> HashMap<BibleSlideId, Uuid> {
    items
        .iter()
        .flat_map(|item| {
            let count = usize::try_from(item.slide_count).unwrap_or(0);
            (0..count).map(move |n| (stable_slide_id(item.id, n), item.id))
        })
        .collect()
}

/// The item order a slide order asks for: items in the order their first slide
/// appears in `slide_ids`, then every item the list does not name, in its old
/// order (like `reorder_bible_slides` keeps unnamed slides).
pub(super) fn item_order(items: &[SermonItemRecord], slide_ids: &[BibleSlideId]) -> Vec<Uuid> {
    let owner_of = owners(items);
    let mut order: Vec<Uuid> = Vec::with_capacity(items.len());
    for slide_id in slide_ids {
        if let Some(item_id) = owner_of.get(slide_id) {
            if !order.contains(item_id) {
                order.push(*item_id);
            }
        }
    }
    for item in items {
        if !order.contains(&item.id) {
            order.push(item.id);
        }
    }
    order
}

impl AppState {
    async fn sermon_presentation(
        &self,
        id: BiblePresentationId,
    ) -> anyhow::Result<BiblePresentation> {
        Ok(self
            .repository
            .fetch_bible_presentation(id)
            .await?
            .ok_or(RepositoryError::NotFound("bible presentation not found"))?)
    }

    /// Delete the item that generated `slide_id` (one sermon write,
    /// `op=delete_item`). 404 when no item generated it.
    pub(crate) async fn delete_sermon_slide(
        &self,
        id: BiblePresentationId,
        slide_id: BibleSlideId,
        origin: &SermonOrigin,
    ) -> anyhow::Result<BiblePresentation> {
        let snapshot = self.load_sermon(id).await?;
        let item_id = *owners(&snapshot.items)
            .get(&slide_id)
            .ok_or(RepositoryError::NotFound(SLIDE_NOT_FOUND))?;
        self.delete_sermon_item(id, item_id, snapshot.sermon.revision, origin)
            .await?;
        self.sermon_presentation(id).await
    }

    /// Order the items as `slide_ids` orders their slides (one sermon write,
    /// `op=reorder`); an unchanged order writes nothing.
    pub(crate) async fn reorder_sermon_slides(
        &self,
        id: BiblePresentationId,
        slide_ids: &[BibleSlideId],
        origin: &SermonOrigin,
    ) -> anyhow::Result<BiblePresentation> {
        let mut snapshot = self.load_sermon(id).await?;
        let order = item_order(&snapshot.items, slide_ids);
        let unchanged = order
            .iter()
            .zip(&snapshot.items)
            .all(|(wanted, item)| *wanted == item.id);
        if !unchanged {
            let expected_revision = snapshot.sermon.revision;
            let mut items = Vec::with_capacity(order.len());
            for item_id in &order {
                if let Some(index) = snapshot.items.iter().position(|item| item.id == *item_id) {
                    items.push(snapshot.items.swap_remove(index));
                }
            }
            self.commit_sermon(SermonChange {
                op: SermonOp::Reorder,
                origin,
                expected_revision,
                sermon: snapshot.sermon,
                items,
                item_id: None,
            })
            .await?;
        }
        self.sermon_presentation(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use presenter_core::bible::sermon::ItemEdits;

    fn record(slides: u32) -> SermonItemRecord {
        SermonItemRecord {
            id: Uuid::new_v4(),
            translation_code: "slk-seb".to_string(),
            book_code: "HEB".to_string(),
            book_number: 58,
            book_name: "Hebrejom".to_string(),
            chapter: 11,
            verse_start: 1,
            verse_end: 1,
            source: Vec::new(),
            edits: ItemEdits::default(),
            slide_count: slides,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn items_follow_their_first_named_slide_and_unnamed_ones_keep_their_place() {
        let (a, b, c) = (record(2), record(1), record(1));
        let items = vec![a.clone(), b.clone(), c.clone()];
        // b's slide first, then a's SECOND slide; c is not named.
        let order = item_order(
            &items,
            &[
                stable_slide_id(b.id, 0),
                stable_slide_id(a.id, 1),
                BibleSlideId::new(),
            ],
        );
        assert_eq!(order, vec![b.id, a.id, c.id]);
    }

    #[test]
    fn an_empty_slide_list_keeps_the_order() {
        let items = vec![record(1), record(1)];
        let ids: Vec<Uuid> = items.iter().map(|i| i.id).collect();
        assert_eq!(item_order(&items, &[]), ids);
    }
}
