//! #555/#558 song-sync trash tests: soft delete, tombstone apply semantics
//! (round-3 Decision B), trash list/restore/prune, and re-import trash
//! carryover (round-3 Decision A). Split from `sync_tests.rs` when that file
//! crossed the 1000-line hard cap — the tests moved verbatim; shared helpers
//! live in `sync_test_support`.
use super::sync_test_support::{peer_song, repo, row, slide, updated_at_of};
use crate::entities::{library, playlist_entry, presentation as presentation_entity};
use sea_orm::EntityTrait;

#[tokio::test]
async fn adopt_by_name_never_adopts_a_trashed_candidate() {
    // S4 regression: the adopt-by-name match had no `deleted_at IS NULL`
    // filter, so a peer row whose sync_id is unknown could silently adopt
    // (and thereby un-delete) a LOCALLY TRASHED song sharing its name. A
    // trashed candidate must never be eligible for adoption — the peer's
    // song must be created as a brand new live row instead.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, local) = repo
        .create_presentation(lib.id, "Shared Song", Some(&[slide(0, "local text")]))
        .await
        .unwrap();
    repo.delete_presentation(local.id).await.unwrap();
    let local_sync_id_before = row(&repo, local.id).await.sync_id;

    let peer = crate::SyncPresentation {
        sync_id: "peer-identity-trashed-case".to_string(),
        library_name: "Songs".to_string(),
        library_sync_id: None,
        name: "Shared Song".to_string(),
        updated_at: chrono::Utc::now() + chrono::Duration::seconds(5),
        deleted_at: None,
        slides: vec![slide(0, "peer text")],
    };
    let (outcome, _id) = repo
        .apply_sync_presentation(&peer, &std::collections::HashSet::new())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        crate::SyncApplyOutcome::Created,
        "a trashed local row must never be adopted-by-name"
    );

    let local_row_after = row(&repo, local.id).await;
    assert!(
        local_row_after.deleted_at.is_some(),
        "the trashed local row stays trashed"
    );
    assert_eq!(
        local_row_after.sync_id, local_sync_id_before,
        "the trashed local row's own identity is untouched"
    );
}

#[tokio::test]
async fn apply_never_adopts_a_tombstone_by_name_onto_a_live_same_name_song() {
    // #558 round-3 Decision B (fixes T1): two SITES can independently hold
    // DIFFERENT songs that happen to share a NAME (different sync_ids) —
    // e.g. each site imported its own "Shared Song". Trashing one site's
    // copy must never reach across and trash the OTHER site's unrelated
    // same-named song via adopt-by-name. A peer TOMBSTONE whose sync_id is
    // unknown must therefore NEVER attempt adopt-by-name at all — it
    // creates its OWN new trashed row instead, leaving any live
    // same-named local song completely untouched.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, local) = repo
        .create_presentation(lib.id, "Shared Song", Some(&[slide(0, "local text")]))
        .await
        .unwrap();
    let local_row_before = row(&repo, local.id).await;

    // Peer's tombstone for a DIFFERENT song sharing the SAME name, unknown
    // sync_id, clearly newer than the local row.
    let tombstone_at = chrono::Utc::now() + chrono::Duration::seconds(5);
    let peer_tombstone = crate::SyncPresentation {
        sync_id: "peer-tombstone-unknown".to_string(),
        library_name: "Songs".to_string(),
        library_sync_id: None,
        name: "Shared Song".to_string(),
        updated_at: tombstone_at,
        deleted_at: Some(tombstone_at),
        slides: vec![slide(0, "peer text")],
    };
    let (outcome, _id) = repo
        .apply_sync_presentation(&peer_tombstone, &std::collections::HashSet::new())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        crate::SyncApplyOutcome::Created,
        "a tombstone with an unknown sync_id must create its OWN row, never adopt-by-name"
    );

    let local_row_after = row(&repo, local.id).await;
    assert_eq!(
        local_row_before, local_row_after,
        "the live local song must be completely untouched"
    );

    let new_trashed = repo
        .fetch_sync_presentation("peer-tombstone-unknown")
        .await
        .unwrap()
        .expect("the peer's tombstone landed as its own new row");
    assert_eq!(new_trashed.deleted_at, Some(tombstone_at));
    assert_eq!(new_trashed.slides[0].content.main.value(), "peer text");

    let trash = repo.list_trashed_presentations().await.unwrap();
    assert!(
        trash.iter().any(|t| t.sync_id == "peer-tombstone-unknown"),
        "the new row shows up in trash"
    );
    assert!(
        !trash
            .iter()
            .any(|t| t.name == "Shared Song" && t.sync_id != "peer-tombstone-unknown"),
        "the local live song must not have been trashed under this identity"
    );
}

#[tokio::test]
async fn apply_skips_an_unknown_tombstone_older_than_the_prune_horizon() {
    // Decision B, second half: an unknown sync_id whose tombstone predates
    // PRUNE_HORIZON is presumed already pruned elsewhere — apply must skip
    // it, never manufacture a fresh trashed row for content nobody asked to
    // resurrect (mirrors S7's guard, exercised here at the
    // apply_sync_presentation level directly, not just via the manifest
    // pre-filter).
    let repo = repo().await;
    let stale_at = chrono::Utc::now() - crate::PRUNE_HORIZON - chrono::Duration::days(1);
    let stale_tombstone = crate::SyncPresentation {
        sync_id: "ancient-tombstone".to_string(),
        library_name: "Songs".to_string(),
        library_sync_id: None,
        name: "Long Gone".to_string(),
        updated_at: stale_at,
        deleted_at: Some(stale_at),
        slides: vec![slide(0, "x")],
    };
    let (outcome, _id) = repo
        .apply_sync_presentation(&stale_tombstone, &std::collections::HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::SkippedNotNewer);
    assert!(
        repo.fetch_sync_presentation("ancient-tombstone")
            .await
            .unwrap()
            .is_none(),
        "an already-pruned tombstone must never be resurrected as a new row"
    );
}

#[tokio::test]
async fn apply_skips_a_past_horizon_tombstone_for_an_unknown_library_without_creating_it() {
    // #558 round-4 U4: `apply_unknown_sync_id`'s horizon-gate skip ran
    // AFTER the library for the incoming peer `library_name` was already
    // ensured/created (unconditionally, at the top of
    // `apply_sync_presentation`) — so even a SKIPPED (never-write) apply
    // committed a phantom, permanently-empty library row for a library
    // this instance otherwise has no reason to know about. FIX: evaluate
    // the tombstone/horizon decision BEFORE creating any library; only
    // ensure-library once the apply is committed to actually writing a
    // row.
    let repo = repo().await;
    let stale_at = chrono::Utc::now() - crate::PRUNE_HORIZON - chrono::Duration::days(1);
    let stale_tombstone = crate::SyncPresentation {
        sync_id: "ancient-unknown-library-tombstone".to_string(),
        library_name: "Never Heard Of It".to_string(),
        library_sync_id: None,
        name: "Long Gone".to_string(),
        updated_at: stale_at,
        deleted_at: Some(stale_at),
        slides: vec![slide(0, "x")],
    };
    let (outcome, _id) = repo
        .apply_sync_presentation(&stale_tombstone, &std::collections::HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::SkippedNotNewer);

    let libraries = repo.fetch_libraries().await.unwrap();
    assert!(
        !libraries.iter().any(|l| l.name == "Never Heard Of It"),
        "a skipped past-horizon tombstone must never create a phantom library"
    );
}

#[tokio::test]
async fn apply_carries_a_peer_delete_and_restore() {
    let repo = repo().await;
    let created = peer_song("sid-del", "Doomed Peer", "x", 30);
    repo.apply_sync_presentation(&created, &std::collections::HashSet::new())
        .await
        .unwrap();

    // Peer deleted it later → local goes to trash.
    let mut deleted = peer_song("sid-del", "Doomed Peer", "x", 20);
    deleted.deleted_at = Some(deleted.updated_at);
    repo.apply_sync_presentation(&deleted, &std::collections::HashSet::new())
        .await
        .unwrap();
    let trash = repo.list_trashed_presentations().await.unwrap();
    assert!(trash.iter().any(|t| t.sync_id == "sid-del"));

    // Peer restored it even later → local leaves the trash.
    let restored = peer_song("sid-del", "Doomed Peer", "x", 10);
    repo.apply_sync_presentation(&restored, &std::collections::HashSet::new())
        .await
        .unwrap();
    let trash = repo.list_trashed_presentations().await.unwrap();
    assert!(!trash.iter().any(|t| t.sync_id == "sid-del"));
}

#[tokio::test]
async fn apply_sync_tombstone_clears_stage_layout_markers_instead_of_remapping_them() {
    // R3 regression: applying a peer's TOMBSTONE went through the same
    // remap-by-position path as a normal edit, so a trashed song's
    // stage-layout markers survived (remapped) across the trash boundary —
    // unlike a LOCAL delete, which clears them (`delete_presentation`).
    // Restoring the song later then flips the stage to a stale layout the
    // operator never re-applied. FIX: when the incoming row is a
    // tombstone, clear markers in the same transaction, mirroring
    // `delete_presentation`'s behavior, instead of remapping them.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, local) = repo
        .create_presentation(
            lib.id,
            "Marked Song",
            Some(&[slide(0, "verse 1"), slide(1, "verse 2")]),
        )
        .await
        .unwrap();
    repo.set_slide_stage_layout(local.id, local.slides[1].id, "fulltext")
        .await
        .unwrap();
    let markers_before = repo.list_slide_stage_layouts(local.id).await.unwrap();
    assert_eq!(markers_before.len(), 1, "sanity: the marker is set");

    let local_sync_id = row(&repo, local.id).await.sync_id;
    let tombstone_at = chrono::Utc::now() + chrono::Duration::seconds(5);
    let tombstone = crate::SyncPresentation {
        sync_id: local_sync_id,
        library_name: "Songs".to_string(),
        library_sync_id: None,
        name: "Marked Song".to_string(),
        updated_at: tombstone_at,
        deleted_at: Some(tombstone_at),
        slides: vec![slide(0, "verse 1"), slide(1, "verse 2")],
    };
    repo.apply_sync_presentation(&tombstone, &std::collections::HashSet::new())
        .await
        .unwrap();

    let markers_after_tombstone = repo.list_slide_stage_layouts(local.id).await.unwrap();
    assert!(
        markers_after_tombstone.is_empty(),
        "a synced tombstone must CLEAR stage-layout markers, mirroring a local delete — \
         never remap-and-preserve them across the trash boundary"
    );

    // A later restore must not resurrect a marker either (mirrors
    // restore_presentation not reinstating anything it never held).
    repo.restore_presentation(local.id).await.unwrap();
    let markers_after_restore = repo.list_slide_stage_layouts(local.id).await.unwrap();
    assert!(
        markers_after_restore.is_empty(),
        "restore must not bring back a cleared marker"
    );
}

#[tokio::test]
async fn trash_lists_restores_and_prunes() {
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, fresh) = repo
        .create_presentation(lib.id, "Fresh Trash", Some(&[slide(0, "a")]))
        .await
        .unwrap();
    let (_, _, old) = repo
        .create_presentation(lib.id, "Old Trash", Some(&[slide(0, "b")]))
        .await
        .unwrap();
    repo.delete_presentation(fresh.id).await.unwrap();
    repo.delete_presentation(old.id).await.unwrap();

    // Trash lists both.
    let trash = repo.list_trashed_presentations().await.unwrap();
    assert_eq!(trash.len(), 2);

    // Restore brings one back into the libraries and bumps its clock.
    let before = updated_at_of(&repo, fresh.id).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    repo.restore_presentation(fresh.id).await.unwrap();
    let restored = row(&repo, fresh.id).await;
    assert!(restored.deleted_at.is_none(), "restore clears the marker");
    let after = updated_at_of(&repo, fresh.id).await;
    assert!(after > before, "restore bumps updated_at (it must sync)");
    let libs = repo.fetch_libraries().await.unwrap();
    assert!(libs
        .iter()
        .any(|l| l.presentations.iter().any(|p| p.name == "Fresh Trash")));

    // Age the other row past PRUNE_HORIZON, then prune keeps only fresh trash.
    use sea_orm::{ColumnTrait, QueryFilter};
    let old_stamp =
        (chrono::Utc::now() - crate::PRUNE_HORIZON - chrono::Duration::days(1)).to_rfc3339();
    presentation_entity::Entity::update_many()
        .col_expr(
            presentation_entity::Column::DeletedAt,
            sea_orm::sea_query::Expr::value(old_stamp),
        )
        .filter(presentation_entity::Column::Id.eq(old.id.to_string()))
        .exec(&repo.db)
        .await
        .unwrap();
    let removed = repo
        .prune_deleted_presentations(crate::PRUNE_HORIZON)
        .await
        .unwrap();
    assert_eq!(
        removed, 1,
        "only the row older than PRUNE_HORIZON is pruned"
    );
    assert!(
        presentation_entity::Entity::find_by_id(old.id.to_string())
            .one(&repo.db)
            .await
            .unwrap()
            .is_none(),
        "pruned row is gone for good"
    );
}

#[tokio::test]
async fn restore_presentation_refuses_when_the_parent_library_is_tombstoned() {
    // #636 regression: restore_presentation used to clear the presentation's
    // OWN deleted_at unconditionally, with no check on its parent library. A
    // song "restored" under a still-tombstoned library becomes reachable by
    // nothing (fetch_libraries excludes the library) AND is now LIVE, so the
    // next tombstone-prune cascade hard-deletes it -- restoring it
    // accomplished nothing durable and actively made things worse.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, song) = repo
        .create_presentation(lib.id, "Trapped Song", Some(&[slide(0, "a")]))
        .await
        .unwrap();
    // delete_library soft-deletes the library AND its live presentations in
    // one transaction -- the realistic path that traps a song under a
    // tombstoned library.
    repo.delete_library(lib.id).await.unwrap();

    let err = repo
        .restore_presentation(song.id)
        .await
        .expect_err("restore must refuse while the parent library is tombstoned");
    assert!(
        matches!(
            err.downcast_ref::<crate::RepositoryError>(),
            Some(crate::RepositoryError::Conflict(_))
        ),
        "expected RepositoryError::Conflict, got: {err}"
    );

    let still_trashed = row(&repo, song.id).await;
    assert!(
        still_trashed.deleted_at.is_some(),
        "a refused restore must leave the presentation exactly as tombstoned \
         as before -- no partial mutation"
    );
}

#[tokio::test]
async fn reimport_preserves_a_trashed_songs_tombstone() {
    // S2 regression: upsert_library inserted EVERY incoming row with
    // deleted_at: None and updated_at: now() unconditionally — so
    // re-importing a library (e.g. re-running the Import Data workflow)
    // resurrected any song the user had TRASHED, and the fresh "now()"
    // stamp then LWW-wins over the peer's real tombstone, propagating the
    // resurrection to the other instance too. A re-import must restore
    // CONTENT but never clear an existing tombstone nor manufacture a newer
    // edit-time for an already-trashed song.
    let repo = repo().await;
    let presentation = presenter_core::Presentation::new("Doomed", vec![slide(0, "v1")]).unwrap();
    let library =
        presenter_core::Library::new("Songs".to_string(), vec![presentation.clone()]).unwrap();
    repo.upsert_library(&library).await.unwrap();

    repo.delete_presentation(presentation.id).await.unwrap();
    let trashed_before = row(&repo, presentation.id).await;
    assert!(
        trashed_before.deleted_at.is_some(),
        "sanity: the song is trashed before the re-import"
    );
    let deleted_at_before = trashed_before.deleted_at;
    let updated_at_before = trashed_before.updated_at;

    // Re-import the SAME library content (same .pro files, unchanged).
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    repo.upsert_library(&library).await.unwrap();

    let after = row(&repo, presentation.id).await;
    assert_eq!(
        after.deleted_at, deleted_at_before,
        "re-import must not resurrect a trashed song"
    );
    assert_eq!(
        after.updated_at, updated_at_before,
        "re-import must not manufacture a newer edit-time for a trashed song"
    );
}

#[tokio::test]
async fn reimport_resurrects_a_trashed_song_when_a_same_name_twin_shifts_its_sync_id() {
    // #558 round-3 DESIGN SIMPLIFICATION (Decision A): the by-name
    // trash-carryover fallback (R1) was DELETED — round-3 review found it
    // unfixable by patching (T2/T4/T5/T6 were four independent failures of
    // the same mechanism: sibling-key leakage, name recycling, scan-order
    // dependence, old-map name collisions). The simplified, final rule:
    // trash carryover on re-import keys on `sync_id` ONLY. If a trashed
    // song's sync_id SHIFTS on re-import (this corner-of-a-corner case: a
    // same-name twin joins the scan, so BOTH occurrences now derive fresh
    // sync_ids under S3's cardinality-sensitive content-pure rule), the
    // song comes back LIVE — that is now CORRECT behavior, not a
    // regression. It composes safely with sync: the peer still holds the
    // OLD sync_id as a fresh tombstone, which Decision B
    // (`sync_apply.rs`) applies as its OWN new trashed row rather than
    // reaching for any existing local row by name — both sites converge to
    // new-id-live + old-id-trashed. A brand-new same-name twin must never
    // inherit anyone's tombstone either.
    let repo = repo().await;
    let original =
        presenter_core::Presentation::new("Amazing Grace", vec![slide(0, "v1")]).unwrap();
    let library =
        presenter_core::Library::new("Songs".to_string(), vec![original.clone()]).unwrap();
    repo.upsert_library(&library).await.unwrap();
    repo.delete_presentation(original.id).await.unwrap();

    let trashed_before = row(&repo, original.id).await;
    assert!(
        trashed_before.deleted_at.is_some(),
        "sanity: trashed before the re-import"
    );
    let updated_at_before = trashed_before.updated_at;

    // Re-import: the SAME "Amazing Grace" content, plus a brand-new
    // same-name twin. Both now collide on the raw name-derived id, so BOTH
    // derive fresh sync_ids (S3) — the exact cardinality shift that used to
    // be R1's regression and is now the documented, accepted outcome.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let reimported =
        presenter_core::Presentation::new("Amazing Grace", vec![slide(0, "v1")]).unwrap();
    let twin =
        presenter_core::Presentation::new("Amazing Grace", vec![slide(0, "v1 (twin)")]).unwrap();
    let library2 =
        presenter_core::Library::new("Songs".to_string(), vec![reimported.clone(), twin.clone()])
            .unwrap();
    repo.upsert_library(&library2).await.unwrap();

    let reimported_row = row(&repo, reimported.id).await;
    let twin_row = row(&repo, twin.id).await;

    assert!(
        reimported_row.deleted_at.is_none(),
        "a sync_id SHIFT on re-import is treated as a fresh identity by design \
         (Decision A) — the song comes back LIVE rather than resurrecting under a \
         key it no longer holds"
    );
    assert!(
        reimported_row.updated_at > updated_at_before,
        "a fresh identity gets a fresh updated_at, exactly like any brand-new row"
    );
    assert!(
        twin_row.deleted_at.is_none(),
        "the brand-new twin must NOT inherit any tombstone"
    );
    assert_ne!(
        reimported_row.sync_id, twin_row.sync_id,
        "sanity: the cardinality shift produced two distinct derived identities"
    );
}

#[tokio::test]
async fn soft_delete_hides_the_song_but_keeps_the_row() {
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, pres) = repo
        .create_presentation(lib.id, "Doomed", Some(&[slide(0, "x")]))
        .await
        .unwrap();

    // Reference it from a playlist to prove the entry is removed on delete.
    let playlist = repo.create_playlist("Sunday", false).await.unwrap();
    playlist_entry::Entity::insert(playlist_entry::ActiveModel {
        id: sea_orm::Set(uuid::Uuid::new_v4().to_string()),
        playlist_id: sea_orm::Set(playlist.id.to_string()),
        entry_type: sea_orm::Set("presentation".to_string()),
        presentation_id: sea_orm::Set(Some(pres.id.to_string())),
        position: sea_orm::Set(0),
        midi_note: sea_orm::Set(None),
        label: sea_orm::Set(None),
    })
    .exec(&repo.db)
    .await
    .unwrap();

    repo.delete_presentation(pres.id).await.unwrap();

    // Hidden from every listing…
    let libs = repo.fetch_libraries().await.unwrap();
    assert!(
        !libs
            .iter()
            .any(|l| l.presentations.iter().any(|p| p.name == "Doomed")),
        "soft-deleted song must not appear in libraries"
    );
    assert!(
        repo.fetch_presentation_detail(pres.id)
            .await
            .unwrap()
            .is_none(),
        "detail fetch must treat a trashed song as absent"
    );

    // …but the row survives, marked.
    let model = row(&repo, pres.id).await;
    assert!(model.deleted_at.is_some(), "row keeps a deleted_at marker");

    // And its playlist entries are gone.
    use sea_orm::{ColumnTrait, QueryFilter};
    let remaining = playlist_entry::Entity::find()
        .filter(playlist_entry::Column::PresentationId.eq(pres.id.to_string()))
        .all(&repo.db)
        .await
        .unwrap();
    assert!(
        remaining.is_empty(),
        "playlist entries referencing the song are removed"
    );
}

#[tokio::test]
async fn ensure_library_picks_the_most_recent_tombstone_when_several_share_a_name() {
    // #580 hardening: the partial unique index only guards LIVE rows
    // (idx_libraries_name_live_unique), so repeated delete/recreate cycles
    // of the same-named library leave MULTIPLE tombstoned rows behind.
    // ensure_library's tombstone lookup must pick the MOST RECENT one (by
    // updated_at) to compare against — picking an arbitrary/older one would
    // LWW-compare the incoming presentation against the wrong tombstone
    // entirely, e.g. wrongly reviving a long-superseded library.
    let repo = repo().await;

    let v1 = repo.create_library("Songs").await.unwrap();
    repo.delete_library(v1.id).await.unwrap();
    let v1_row_before = library::Entity::find_by_id(v1.id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("v1 row still exists, tombstoned");
    let t1: chrono::DateTime<chrono::Utc> = v1_row_before.updated_at.into();

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let v2 = repo.create_library("Songs").await.unwrap();
    repo.delete_library(v2.id).await.unwrap();
    let v2_row_before = library::Entity::find_by_id(v2.id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("v2 row still exists, tombstoned");
    let t2: chrono::DateTime<chrono::Utc> = v2_row_before.updated_at.into();
    assert!(t2 > t1, "sanity: v2 was tombstoned strictly after v1");

    // A live peer presentation whose updated_at sits STRICTLY BETWEEN the two
    // tombstones: newer than v1's (would wrongly revive v1 if v1 were picked)
    // but OLDER than v2's (correctly stays tombstoned when v2 -- the actually
    // relevant, most recent tombstone -- is picked).
    let between = t1 + (t2 - t1) / 2;
    let peer = crate::SyncPresentation {
        sync_id: "peer-p2-multi-tombstone".to_string(),
        library_name: "Songs".to_string(),
        library_sync_id: None,
        name: "P2".to_string(),
        updated_at: between,
        deleted_at: None,
        slides: vec![slide(0, "x")],
    };
    let (outcome, id) = repo
        .apply_sync_presentation(&peer, &std::collections::HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_row = row(&repo, id.expect("created presentation has an id")).await;

    assert_eq!(
        pres_row.library_id,
        v2.id.to_string(),
        "P2 must attach to v2 -- the MOST RECENT tombstone -- never v1 nor a \
         freshly-minted third library"
    );

    let v1_row_after = library::Entity::find_by_id(v1.id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        v1_row_after.deleted_at.is_some(),
        "v1 is untouched -- still tombstoned"
    );
    let v2_row_after = library::Entity::find_by_id(v2.id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        v2_row_after.deleted_at.is_some(),
        "v2 stays tombstoned -- P2 is OLDER than v2's own tombstone, so no revival"
    );
    // #634 regression: P2 attaches to a library that stays tombstoned, so P2
    // itself must ALSO be written as tombstoned -- a LIVE presentation
    // parented to a dead library is invisible (excluded from
    // fetch_libraries) and gets hard-deleted by the next tombstone-prune
    // cascade with zero prior trace it ever existed.
    assert!(
        pres_row.deleted_at.is_some(),
        "P2 must be written as tombstoned -- it attaches to v2, which stays \
         tombstoned; a live row under a dead library is silent data loss (#634)"
    );

    use sea_orm::{ColumnTrait, QueryFilter};
    let all_songs = library::Entity::find()
        .filter(library::Column::Name.eq("Songs".to_string()))
        .all(&repo.db)
        .await
        .unwrap();
    assert_eq!(
        all_songs.len(),
        2,
        "exactly v1 and v2 -- no third library was minted"
    );
}

#[tokio::test]
async fn ensure_library_boundary_is_strict_greater_than_not_greater_or_equal() {
    // #594 finding 4: the LWW boundary in `ensure_library` (sync_apply.rs:357)
    // is a STRICT `>` (mirrors `sync_should_apply`'s own strict tie-break),
    // documented as deliberate at :311 but previously untested -- mutating it
    // to `>=` would pass every existing test (exact equality is near
    // unreachable in practice, but the boundary itself was unproven). An
    // incoming presentation whose `updated_at` is EXACTLY EQUAL to the
    // tombstoned library's own must NOT revive it.
    let repo = repo().await;

    let lib = repo.create_library("Songs").await.unwrap();
    repo.delete_library(lib.id).await.unwrap();
    let lib_row = library::Entity::find_by_id(lib.id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("library row still exists, tombstoned");
    let tombstoned_at: chrono::DateTime<chrono::Utc> = lib_row.updated_at.into();

    let peer_tied = crate::SyncPresentation {
        sync_id: "peer-p2-tied-with-tombstone".to_string(),
        library_name: "Songs".to_string(),
        library_sync_id: None,
        name: "P2".to_string(),
        updated_at: tombstoned_at,
        deleted_at: None,
        slides: vec![slide(0, "x")],
    };
    let (outcome, id) = repo
        .apply_sync_presentation(&peer_tied, &std::collections::HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_row = row(&repo, id.expect("created presentation has an id")).await;
    assert_eq!(
        pres_row.library_id,
        lib.id.to_string(),
        "P2 (tied exactly with the tombstone's own updated_at) still attaches \
         to the existing tombstoned library"
    );

    let lib_row_after = library::Entity::find_by_id(lib.id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        lib_row_after.deleted_at.is_some(),
        "an EXACTLY EQUAL updated_at must not revive the library -- the LWW \
         boundary is strict `>`, not `>=`"
    );
    // #634 regression: same reasoning as the multi-tombstone test above --
    // P2 stays attached to a library that stays tombstoned, so P2 must be
    // written as tombstoned too, never silently live under a dead parent.
    assert!(
        pres_row.deleted_at.is_some(),
        "P2 must be written as tombstoned -- the library it attaches to \
         stays tombstoned; a live row under a dead library is silent data \
         loss (#634)"
    );
}

#[tokio::test]
async fn ensure_library_tie_breaks_identical_updated_at_tombstones_by_sync_id() {
    // #594 finding 3: the `order_by_desc(SyncId)` secondary key added
    // alongside `order_by_desc(UpdatedAt)` in `ensure_library` (bd044c4e) is
    // what makes two peers pick the SAME "most recent" tombstone when their
    // `updated_at` values happen to be IDENTICAL (e.g. two rapid
    // delete/recreate cycles landing on the same wall-clock millisecond).
    // `ensure_library_picks_the_most_recent_tombstone_when_several_share_a_name`
    // above never exercises this branch -- it deliberately sleeps 5ms so the
    // two tombstones get DIFFERENT `updated_at` values. This test writes two
    // tombstoned "Songs" rows with an IDENTICAL `updated_at` and distinct,
    // known `sync_id`s directly (bypassing the sleep), so only the SyncId
    // DESC tie-break decides the winner -- deterministic, independent of
    // insertion order.
    let repo = repo().await;
    let tied_at: chrono::DateTime<chrono::Utc> = chrono::Utc::now();

    let make_tombstoned_row = |id: String, sync_id: &str| library::ActiveModel {
        id: sea_orm::Set(id),
        name: sea_orm::Set("Songs".to_string()),
        search_name: sea_orm::Set("songs".to_string()),
        created_at: sea_orm::Set(tied_at.into()),
        updated_at: sea_orm::Set(tied_at.into()),
        sync_id: sea_orm::Set(sync_id.to_string()),
        deleted_at: sea_orm::Set(Some(tied_at.into())),
    };
    let low_id = uuid::Uuid::new_v4().to_string();
    let high_id = uuid::Uuid::new_v4().to_string();
    // Inserted with the row that must LOSE the tie-break FIRST and the row
    // that must WIN it SECOND. SQLite's `ORDER BY UpdatedAt DESC` alone (no
    // secondary sort) breaks a tie by returning matching rows in their
    // natural/insertion order -- so if the `order_by_desc(SyncId)` line were
    // ever deleted from `ensure_library`, an insertion-order fallback would
    // pick whichever row was inserted FIRST. Inserting the loser (`low_id`)
    // first means that fallback would wrongly return `low_id`, failing the
    // assertion below -- proving the SyncId tie-break, not insertion order,
    // is what selects `high_id`. (An earlier version of this test inserted
    // the winner first, which coincidentally still passed with the tie-break
    // line deleted -- a vacuous oracle caught by review.)
    library::Entity::insert(make_tombstoned_row(low_id.clone(), "aaaaaaaa-tie-low"))
        .exec(&repo.db)
        .await
        .unwrap();
    library::Entity::insert(make_tombstoned_row(high_id.clone(), "zzzzzzzz-tie-high"))
        .exec(&repo.db)
        .await
        .unwrap();

    // A live peer presentation newer than the tied updated_at -- must attach
    // to whichever tombstone wins the SyncId DESC tie-break: "zzzzzzzz...".
    let peer = crate::SyncPresentation {
        sync_id: "peer-p2-tied-tombstones".to_string(),
        library_name: "Songs".to_string(),
        library_sync_id: None,
        name: "P2".to_string(),
        updated_at: tied_at + chrono::Duration::seconds(5),
        deleted_at: None,
        slides: vec![slide(0, "x")],
    };
    let (outcome, id) = repo
        .apply_sync_presentation(&peer, &std::collections::HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_row = row(&repo, id.expect("created presentation has an id")).await;

    assert_eq!(
        pres_row.library_id, high_id,
        "identical updated_at must tie-break on SyncId DESC -- the row with \
         the lexicographically LATER sync_id ('zzzzzzzz...') wins, never an \
         arbitrary/insertion-order pick"
    );

    let low_row_after = library::Entity::find_by_id(low_id)
        .one(&repo.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        low_row_after.deleted_at.is_some(),
        "the losing tombstone (lower sync_id) is untouched -- still tombstoned"
    );
}

#[tokio::test]
async fn moving_an_existing_song_onto_a_tombstoned_library_tombstones_the_song_too() {
    // #634 regression, step-1 path (existing local row matched by sync_id,
    // UPDATED) -- the tombstone tests above exercise the SAME `ensure_library`
    // bug via step-3 (brand-new sync_id, `apply_unknown_sync_id`). This proves
    // the fix also covers an UPDATE: the peer moved an already-known
    // presentation to a library that, locally, only exists tombstoned.
    let repo = repo().await;
    repo.create_library("Old").await.unwrap();

    let initial = crate::SyncPresentation {
        sync_id: "peer-move-test".to_string(),
        library_name: "Old".to_string(),
        library_sync_id: None,
        name: "Moved Song".to_string(),
        updated_at: chrono::Utc::now() - chrono::Duration::minutes(10),
        deleted_at: None,
        slides: vec![slide(0, "x")],
    };
    let (outcome, id) = repo
        .apply_sync_presentation(&initial, &std::collections::HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_id = id.expect("created presentation has an id");

    let new_lib = repo.create_library("New").await.unwrap();
    repo.delete_library(new_lib.id).await.unwrap();

    // Peer moved the song to "New": newer than the song's own last-known
    // updated_at (so LWW allows the update to apply) but OLDER than "New"'s
    // own tombstone time (so the tombstone wins -- "New" stays dead).
    let moved = crate::SyncPresentation {
        sync_id: "peer-move-test".to_string(),
        library_name: "New".to_string(),
        library_sync_id: None,
        name: "Moved Song".to_string(),
        updated_at: chrono::Utc::now() - chrono::Duration::minutes(5),
        deleted_at: None,
        slides: vec![slide(0, "x")],
    };
    let (outcome, _) = repo
        .apply_sync_presentation(&moved, &std::collections::HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Updated);

    let pres_row = row(&repo, pres_id).await;
    assert_eq!(
        pres_row.library_id,
        new_lib.id.to_string(),
        "the song reattaches to 'New' even though 'New' stays tombstoned"
    );
    assert!(
        pres_row.deleted_at.is_some(),
        "the song must be written as tombstoned -- it now lives under a \
         library that stays tombstoned; leaving it LIVE is silent data \
         loss (#634)"
    );

    let new_lib_row = library::Entity::find_by_id(new_lib.id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        new_lib_row.deleted_at.is_some(),
        "'New' stays tombstoned -- the move is older than its own tombstone"
    );
}
