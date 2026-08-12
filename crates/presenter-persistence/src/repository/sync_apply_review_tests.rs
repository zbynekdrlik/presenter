//! #646 adversarial-review regression tests for the #634 forced-tombstone
//! path in `sync_apply.rs`. Kept in its OWN sibling file rather than growing
//! `sync_apply.rs` (959 prod lines) or `sync_trash_tests.rs` (853) toward the
//! 1000-line hard cap — mirrors the `sync_trash_tests`/`search_trash_tests`
//! split precedent. Shared helpers live in `sync_test_support`.
use super::sync_test_support::{peer_song, repo, row, slide};
use crate::entities::{library, playlist_entry, slide_stage_layout};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::collections::HashSet;

#[tokio::test]
async fn forced_tombstone_pins_deleted_at_to_the_library_clock_and_updated_at_to_the_incoming_clock(
) {
    // #646 critical finding: the OLD code stamped BOTH `deleted_at` and
    // `updated_at` of a force-tombstoned row with
    // `presentation_updated_at.max(library_updated)` — a single, maxed
    // timestamp. That let `updated_at` jump PAST its true incoming value,
    // which could then LWW-beat the peer's own live copy of the same song.
    // Pin the fix: the two stamps are DISTINCT, each from its own honest
    // source.
    let repo = repo().await;
    repo.create_library("Old").await.unwrap();

    let initial = peer_song("peer-pin-stamps", "Pinned Song", "x", 30);
    let (outcome, id) = repo
        .apply_sync_presentation(&initial, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_id = id.expect("created presentation has an id");

    let new_lib = repo.create_library("New").await.unwrap();
    repo.delete_library(new_lib.id).await.unwrap();
    let new_lib_row = library::Entity::find_by_id(new_lib.id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("library row still exists, tombstoned");
    let library_tombstone_at: chrono::DateTime<chrono::Utc> = new_lib_row.updated_at.into();

    // Newer than the song's own clock (so the update applies) but OLDER
    // than "New"'s tombstone (so the tombstone wins) -- and DISTINCT from
    // the library's tombstone time, so a maxed stamp is provably wrong.
    let moved_updated_at = library_tombstone_at - chrono::Duration::minutes(10);
    let moved = crate::SyncPresentation {
        sync_id: "peer-pin-stamps".to_string(),
        library_name: "New".to_string(),
        library_sync_id: None,
        name: "Pinned Song".to_string(),
        updated_at: moved_updated_at,
        deleted_at: None,
        slides: vec![slide(0, "x")],
    };
    let (outcome, _) = repo
        .apply_sync_presentation(&moved, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Updated);

    let pres_row = row(&repo, pres_id).await;
    let stored_deleted_at: chrono::DateTime<chrono::Utc> =
        pres_row.deleted_at.expect("forced tombstone").into();
    let stored_updated_at: chrono::DateTime<chrono::Utc> = pres_row.updated_at.into();

    assert_eq!(
        stored_deleted_at, library_tombstone_at,
        "deleted_at must be the LIBRARY's own tombstone time, never max'd \
         against the presentation's clock"
    );
    assert_eq!(
        stored_updated_at, moved_updated_at,
        "updated_at must stay the INCOMING presentation's own clock, \
         unchanged -- never bumped to the (later) library tombstone time"
    );
    assert_ne!(
        stored_deleted_at, stored_updated_at,
        "sanity: the two stamps are genuinely distinct here -- a maxed \
         single stamp would make them equal"
    );
}

#[tokio::test]
async fn forced_tombstone_is_lww_neutral_and_never_beats_a_peers_live_copy() {
    // #646 critical finding, the actual data-loss mechanism: a forced
    // tombstone carrying the SAME clock a peer already holds LIVE must
    // never apply onto that peer's copy -- the strict `>` LWW gate treats
    // equal clocks as "not newer". Deletion reaches the peer only through
    // the library-level channel, never by this forced row directly beating
    // a live copy.
    let repo = repo().await;
    repo.create_library("Old").await.unwrap();
    repo.apply_sync_presentation(
        &peer_song("peer-lww-neutral", "Neutral Song", "x", 30),
        &HashSet::new(),
    )
    .await
    .unwrap();

    let new_lib = repo.create_library("New").await.unwrap();
    repo.delete_library(new_lib.id).await.unwrap();

    let moved_updated_at = chrono::Utc::now() - chrono::Duration::minutes(20);
    let moved = crate::SyncPresentation {
        sync_id: "peer-lww-neutral".to_string(),
        library_name: "New".to_string(),
        library_sync_id: None,
        name: "Neutral Song".to_string(),
        updated_at: moved_updated_at,
        deleted_at: None,
        slides: vec![slide(0, "x")],
    };
    let (outcome, id) = repo
        .apply_sync_presentation(&moved, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Updated);
    let pres_row = row(&repo, id.expect("update returns the existing id")).await;
    assert!(
        pres_row.deleted_at.is_some(),
        "sanity: forced tombstone applied locally"
    );

    // Simulate a PEER that still holds this exact song LIVE, at the SAME
    // clock we just forced it at -- "the library-level tombstone hasn't
    // reached this peer yet". (Calling the helper via its module path, not
    // by name, since `repo` is already shadowed by the local variable above.)
    let peer_repo = super::sync_test_support::repo().await;
    peer_repo.create_library("New").await.unwrap();
    peer_repo
        .apply_sync_presentation(
            &crate::SyncPresentation {
                sync_id: "peer-lww-neutral".to_string(),
                library_name: "New".to_string(),
                library_sync_id: None,
                name: "Neutral Song".to_string(),
                updated_at: moved_updated_at,
                deleted_at: None,
                slides: vec![slide(0, "x")],
            },
            &HashSet::new(),
        )
        .await
        .unwrap();

    // Apply OUR forced tombstone (as the peer would receive it via sync)
    // onto the peer's repo.
    let our_forced = crate::SyncPresentation {
        sync_id: "peer-lww-neutral".to_string(),
        library_name: "New".to_string(),
        library_sync_id: None,
        name: "Neutral Song".to_string(),
        updated_at: pres_row.updated_at.into(),
        deleted_at: pres_row.deleted_at.map(Into::into),
        slides: vec![slide(0, "x")],
    };
    let (peer_outcome, _) = peer_repo
        .apply_sync_presentation(&our_forced, &HashSet::new())
        .await
        .unwrap();

    assert_eq!(
        peer_outcome,
        crate::SyncApplyOutcome::SkippedNotNewer,
        "the forced tombstone's clock EQUALS the peer's own live copy's \
         clock -- the strict `>` LWW gate must skip it, never letting the \
         forced row beat a live copy directly"
    );
    let peer_pres_id = peer_repo
        .find_presentation_id_by_sync_id("peer-lww-neutral")
        .await
        .unwrap()
        .expect("peer holds the song");
    let peer_row = row(&peer_repo, peer_pres_id).await;
    assert!(
        peer_row.deleted_at.is_none(),
        "the peer's live copy must remain untouched -- data loss is \
         exactly a forced tombstone silently beating a live copy"
    );
}

#[tokio::test]
async fn forced_tombstone_cleans_playlist_entries_and_clears_stage_layout_markers() {
    // #646 finding 3: a force-tombstoned song must be cleaned the SAME way
    // `delete_library`/`delete_presentation` clean a real local delete --
    // playlist references removed, stage-layout markers cleared (never
    // remapped onto the new slides of a row that is now hidden).
    let repo = repo().await;
    repo.create_library("Old").await.unwrap();

    let initial = peer_song("peer-cascade", "Cascade Song", "verse one", 30);
    let (outcome, id) = repo
        .apply_sync_presentation(&initial, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_id = id.expect("created presentation has an id");

    let playlist = repo.create_playlist("Sunday", false).await.unwrap();
    playlist_entry::Entity::insert(playlist_entry::ActiveModel {
        id: sea_orm::Set(uuid::Uuid::new_v4().to_string()),
        playlist_id: sea_orm::Set(playlist.id.to_string()),
        entry_type: sea_orm::Set("presentation".to_string()),
        presentation_id: sea_orm::Set(Some(pres_id.to_string())),
        position: sea_orm::Set(0),
        midi_note: sea_orm::Set(None),
        label: sea_orm::Set(None),
    })
    .exec(&repo.db)
    .await
    .unwrap();

    let (_, _, presentation) = repo
        .fetch_presentation_detail(pres_id)
        .await
        .unwrap()
        .expect("presentation exists");
    let slide_id = presentation.slides[0].id;
    repo.set_slide_stage_layout(pres_id, slide_id, "fulltext")
        .await
        .unwrap();
    assert!(
        repo.get_slide_stage_layout(slide_id)
            .await
            .unwrap()
            .is_some(),
        "sanity: marker set"
    );

    let new_lib = repo.create_library("New").await.unwrap();
    repo.delete_library(new_lib.id).await.unwrap();

    let moved = crate::SyncPresentation {
        sync_id: "peer-cascade".to_string(),
        library_name: "New".to_string(),
        library_sync_id: None,
        name: "Cascade Song".to_string(),
        updated_at: chrono::Utc::now() - chrono::Duration::minutes(5),
        deleted_at: None,
        slides: vec![slide(0, "verse one")],
    };
    let (outcome, _) = repo
        .apply_sync_presentation(&moved, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Updated);
    let pres_row = row(&repo, pres_id).await;
    assert!(pres_row.deleted_at.is_some(), "sanity: force-tombstoned");

    let remaining_entries = playlist_entry::Entity::find()
        .filter(playlist_entry::Column::PresentationId.eq(pres_id.to_string()))
        .all(&repo.db)
        .await
        .unwrap();
    assert!(
        remaining_entries.is_empty(),
        "playlist entries referencing the force-tombstoned song must be removed"
    );

    let remaining_markers = slide_stage_layout::Entity::find()
        .filter(slide_stage_layout::Column::PresentationId.eq(pres_id.to_string()))
        .all(&repo.db)
        .await
        .unwrap();
    assert!(
        remaining_markers.is_empty(),
        "stage-layout markers for the force-tombstoned song must be \
         cleared, never remapped onto its replacement slides"
    );
}

#[tokio::test]
async fn genuine_incoming_tombstone_cleans_playlist_entries() {
    // #649: a GENUINE peer-initiated tombstone (the peer deleted the song
    // for real -- no library cascade involved, unlike the #646 forced case
    // above) must clean `playlist_entry` rows the same way a local delete
    // (`delete_presentation`) does. Before the fix, `write_synced_row` only
    // ran the playlist-entry cleanup when `forced_delete.is_some()`, so this
    // ordinary case -- matched by `sync_id`, `incoming.deleted_at: Some(_)`,
    // no library cascade -- left the reference behind.
    let repo = repo().await;
    repo.create_library("Songs").await.unwrap();

    let initial = peer_song("peer-genuine-tombstone", "Genuine Song", "verse one", 30);
    let (outcome, id) = repo
        .apply_sync_presentation(&initial, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_id = id.expect("created presentation has an id");

    let playlist = repo.create_playlist("Sunday", false).await.unwrap();
    playlist_entry::Entity::insert(playlist_entry::ActiveModel {
        id: sea_orm::Set(uuid::Uuid::new_v4().to_string()),
        playlist_id: sea_orm::Set(playlist.id.to_string()),
        entry_type: sea_orm::Set("presentation".to_string()),
        presentation_id: sea_orm::Set(Some(pres_id.to_string())),
        position: sea_orm::Set(0),
        midi_note: sea_orm::Set(None),
        label: sea_orm::Set(None),
    })
    .exec(&repo.db)
    .await
    .unwrap();

    // The peer's own, real deletion of this song -- same library, no
    // cascade, `deleted_at: Some(_)` -- strictly newer than the initial
    // apply so it's accepted.
    let tombstone = crate::SyncPresentation {
        sync_id: "peer-genuine-tombstone".to_string(),
        library_name: "Songs".to_string(),
        library_sync_id: None,
        name: "Genuine Song".to_string(),
        updated_at: chrono::Utc::now() - chrono::Duration::minutes(5),
        deleted_at: Some(chrono::Utc::now() - chrono::Duration::minutes(5)),
        slides: vec![slide(0, "verse one")],
    };
    let (outcome, _) = repo
        .apply_sync_presentation(&tombstone, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Updated);
    let pres_row = row(&repo, pres_id).await;
    assert!(
        pres_row.deleted_at.is_some(),
        "sanity: genuinely tombstoned"
    );

    let remaining_entries = playlist_entry::Entity::find()
        .filter(playlist_entry::Column::PresentationId.eq(pres_id.to_string()))
        .all(&repo.db)
        .await
        .unwrap();
    assert!(
        remaining_entries.is_empty(),
        "playlist entries referencing a genuinely (non-forced) tombstoned \
         song must be removed, same as a local delete"
    );
}
