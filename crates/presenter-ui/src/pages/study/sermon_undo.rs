//! Späť on a toast (spec §7.6), in the sermon's queue: an open editor follows,
//! and a refusal keeps Späť on screen, because the item exists only there.

use leptos::prelude::*;
use presenter_core::bible::sermon::ItemEdits;

use super::add_to_sermon::add_with_retry;
use super::sermon_editor::{editor_for, end_turn, take_turn, Editor, SAVE_FIRST};
use super::{sermon_error, Study, Undo};
use crate::api::sermons::{self, is_conflict, SermonDetail, SermonItem, SermonPassage};
use crate::api::ApiError;
use crate::state::sermon_edit::retain_unchanged;

async fn delete_with_retry(sermon: &str, item: &str) -> Result<SermonDetail, ApiError> {
    let held = sermons::get_sermon(sermon).await?;
    match sermons::delete_item(sermon, item, held.revision).await {
        Err(err) if is_conflict(&err) => {
            let fresh = sermons::get_sermon(sermon).await?;
            sermons::delete_item(sermon, item, fresh.revision).await
        }
        other => other,
    }
}

/// Späť on "Pridané": the added item goes again, in the sermon's queue, so an
/// editor open on it follows. A refusal keeps Späť on screen.
pub(super) fn undo_added(s: Study, sermon: String, item: String) {
    let e = editor_for(s, &sermon);
    leptos::task::spawn_local(async move {
        let offer_again = |message: String, item: String| {
            s.tell_undo(
                message,
                Undo::Added {
                    sermon: e.id(),
                    item,
                },
            );
        };
        if !take_turn(e).await {
            offer_again(SAVE_FIRST.to_string(), item);
            return;
        }
        match delete_with_retry(&e.id(), &item).await {
            Ok(detail) => {
                e.apply(detail);
                s.tell("Odstránené z kázne".to_string());
            }
            Err(err) => offer_again(sermon_error(&err), item),
        }
        end_turn(e);
    });
}

/// The deleted item's edits on the item that came back. The server snapshots
/// today's text, so only the verses still reading the same take theirs.
fn restore_edits(e: Editor, removed: &SermonItem, new_id: &str, edits: &ItemEdits) {
    let Some(back) = e.item(new_id) else {
        return;
    };
    let kept = retain_unchanged(&removed.source, &back.source, edits);
    if kept != *edits {
        e.s.tell("Text sa medzitým zmenil — úpravy zmenených veršov sa nevrátili".to_string());
    }
    if kept != ItemEdits::default() {
        e.edit(new_id, |draft| draft.restore(kept));
    }
}

/// Späť on "Odstránené", in the sermon's queue: the item at the same position
/// with its edits. A refusal keeps Späť on screen: the item exists only there.
pub(super) fn undo_removed(
    s: Study,
    sermon: String,
    item: SermonItem,
    position: usize,
    edits: ItemEdits,
) {
    let e = editor_for(s, &sermon);
    leptos::task::spawn_local(async move {
        let offer_again = |message: String, item: SermonItem, edits: ItemEdits| {
            s.tell_undo(
                message,
                Undo::Removed {
                    sermon: e.id(),
                    item: Box::new(item),
                    position,
                    edits,
                },
            );
        };
        if !take_turn(e).await {
            offer_again(SAVE_FIRST.to_string(), item, edits);
            return;
        }
        let passage = SermonPassage {
            translation: item.translation.clone(),
            book_code: item.book_code.clone(),
            chapter: item.chapter,
            verse_start: item.verse_start,
            verse_end: item.verse_end,
        };
        match add_with_retry(&e.id(), Some(position), &passage).await {
            Ok((detail, new_id)) => {
                e.apply(detail);
                // A normal edit from here: it retries like any other.
                restore_edits(e, &item, &new_id, &edits);
                end_turn(e);
            }
            Err(err) => {
                end_turn(e);
                offer_again(sermon_error(&err), item, edits);
            }
        }
    });
}
