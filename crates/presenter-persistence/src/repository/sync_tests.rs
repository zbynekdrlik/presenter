//! #555 song-sync repository tests: identity, LWW apply, soft-delete, trash.
//! Add further `use` imports (ColumnTrait/QueryFilter/etc.) in the task that first needs
//! them — keep the file clippy-clean (`-D warnings` forbids unused imports) at every commit.
use crate::entities::{playlist_entry, presentation as presentation_entity};
use crate::Repository;
use presenter_core::{PresentationId, Slide, SlideContent, SlideText};
use sea_orm::EntityTrait;

async fn repo() -> Repository {
    Repository::connect_in_memory()
        .await
        .expect("in-memory repo")
}

fn slide(order: u32, main: &str) -> Slide {
    Slide::new(
        order,
        SlideContent::new(
            SlideText::new(main).unwrap(),
            SlideText::new("").unwrap(),
            SlideText::new("").unwrap(),
            None,
        ),
    )
}

/// Direct row read (test-only) — used before the sync read methods exist.
async fn row(repo: &Repository, id: PresentationId) -> presentation_entity::Model {
    presentation_entity::Entity::find_by_id(id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("presentation row exists")
}

async fn updated_at_of(repo: &Repository, id: PresentationId) -> chrono::DateTime<chrono::Utc> {
    row(repo, id).await.updated_at.into()
}

#[tokio::test]
async fn rename_bumps_updated_at() {
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, pres) = repo
        .create_presentation(lib.id, "Old", Some(&[slide(0, "a")]))
        .await
        .unwrap();
    let before = updated_at_of(&repo, pres.id).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    repo.rename_presentation(pres.id, "New").await.unwrap();
    let after = updated_at_of(&repo, pres.id).await;
    assert!(after > before, "rename must bump updated_at");
}

#[tokio::test]
async fn slide_content_edit_bumps_updated_at() {
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, pres) = repo
        .create_presentation(lib.id, "Song", Some(&[slide(0, "old")]))
        .await
        .unwrap();
    let slide_id = pres.slides[0].id;
    let before = updated_at_of(&repo, pres.id).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let new_content = SlideContent::new(
        SlideText::new("new").unwrap(),
        SlideText::new("").unwrap(),
        SlideText::new("").unwrap(),
        None,
    );
    repo.update_slide_content_with_metadata(pres.id, slide_id, &new_content, None)
        .await
        .unwrap();
    let after = updated_at_of(&repo, pres.id).await;
    assert!(after > before, "slide content edit must bump updated_at");
}

#[tokio::test]
async fn replace_slides_bumps_updated_at() {
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, pres) = repo
        .create_presentation(lib.id, "Song", Some(&[slide(0, "a")]))
        .await
        .unwrap();
    let before = updated_at_of(&repo, pres.id).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    repo.replace_presentation_slides(pres.id, &[slide(0, "a"), slide(1, "b")])
        .await
        .unwrap();
    let after = updated_at_of(&repo, pres.id).await;
    assert!(after > before, "structural slide ops must bump updated_at");
}

fn peer_song(sync_id: &str, name: &str, main: &str, minutes_ago: i64) -> crate::SyncPresentation {
    crate::SyncPresentation {
        sync_id: sync_id.to_string(),
        library_name: "Songs".to_string(),
        name: name.to_string(),
        updated_at: chrono::Utc::now() - chrono::Duration::minutes(minutes_ago),
        deleted_at: None,
        slides: vec![slide(0, main)],
    }
}

#[tokio::test]
async fn apply_creates_unknown_updates_newer_skips_older() {
    let repo = repo().await;

    // Unknown → created (library auto-created too).
    let outcome = repo
        .apply_sync_presentation(&peer_song("sid-1", "Peer Song", "v1", 10))
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);

    // Newer peer edit → updated, and the PEER timestamp is stored verbatim.
    let newer = peer_song("sid-1", "Peer Song", "v2", 5);
    let outcome = repo.apply_sync_presentation(&newer).await.unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Updated);
    let manifest = repo.list_sync_manifest().await.unwrap();
    let row = manifest.iter().find(|r| r.sync_id == "sid-1").unwrap();
    assert_eq!(
        row.updated_at, newer.updated_at,
        "apply stores the peer's clock, never now() (no echo)"
    );

    // Older peer state → skipped, content untouched.
    let outcome = repo
        .apply_sync_presentation(&peer_song("sid-1", "Peer Song", "stale", 60))
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::SkippedNotNewer);
    let full = repo
        .fetch_sync_presentation("sid-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(full.slides[0].content.main.value(), "v2");
}

#[tokio::test]
async fn apply_adopts_by_name_preserving_the_local_row_id() {
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, local) = repo
        .create_presentation(lib.id, "Shared Song", Some(&[slide(0, "local text")]))
        .await
        .unwrap();
    let local_row_before = row(&repo, local.id).await;

    // Peer holds the same-named song under a DIFFERENT sync_id, newer edit.
    let peer = crate::SyncPresentation {
        sync_id: "peer-identity".to_string(),
        library_name: "Songs".to_string(),
        name: "Shared Song".to_string(),
        updated_at: chrono::Utc::now() + chrono::Duration::seconds(5),
        deleted_at: None,
        slides: vec![slide(0, "peer text")],
    };
    let outcome = repo.apply_sync_presentation(&peer).await.unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::AdoptedByName);

    let local_row_after = row(&repo, local.id).await;
    assert_eq!(
        local_row_before.id, local_row_after.id,
        "local presentation id survives (playlist refs intact)"
    );
    assert_eq!(
        local_row_after.sync_id, "peer-identity",
        "the peer's sync_id is adopted"
    );
    let full = repo
        .fetch_sync_presentation("peer-identity")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(full.slides[0].content.main.value(), "peer text");
}

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
        name: "Shared Song".to_string(),
        updated_at: chrono::Utc::now() + chrono::Duration::seconds(5),
        deleted_at: None,
        slides: vec![slide(0, "peer text")],
    };
    let outcome = repo.apply_sync_presentation(&peer).await.unwrap();
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
async fn adopt_by_name_never_guesses_among_multiple_live_candidates() {
    // S4 regression: `.one()` with no ORDER BY picks an ARBITRARY row when
    // 2+ live candidates share the same name in the same library. Adoption
    // must happen ONLY when exactly one live candidate exists — an
    // ambiguous match must fall through to create, never guess.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, first) = repo
        .create_presentation(lib.id, "Shared Song", Some(&[slide(0, "a")]))
        .await
        .unwrap();
    let (_, _, second) = repo
        .create_presentation(lib.id, "Shared Song", Some(&[slide(0, "b")]))
        .await
        .unwrap();
    let first_sync_id_before = row(&repo, first.id).await.sync_id;
    let second_sync_id_before = row(&repo, second.id).await.sync_id;

    let peer = crate::SyncPresentation {
        sync_id: "peer-identity-ambiguous-case".to_string(),
        library_name: "Songs".to_string(),
        name: "Shared Song".to_string(),
        updated_at: chrono::Utc::now() + chrono::Duration::seconds(5),
        deleted_at: None,
        slides: vec![slide(0, "peer text")],
    };
    let outcome = repo.apply_sync_presentation(&peer).await.unwrap();
    assert_eq!(
        outcome,
        crate::SyncApplyOutcome::Created,
        "an ambiguous name match (2+ live candidates) must never guess"
    );

    assert_eq!(
        row(&repo, first.id).await.sync_id,
        first_sync_id_before,
        "neither ambiguous candidate is touched"
    );
    assert_eq!(
        row(&repo, second.id).await.sync_id,
        second_sync_id_before,
        "neither ambiguous candidate is touched"
    );
}

#[tokio::test]
async fn apply_carries_a_peer_delete_and_restore() {
    let repo = repo().await;
    let created = peer_song("sid-del", "Doomed Peer", "x", 30);
    repo.apply_sync_presentation(&created).await.unwrap();

    // Peer deleted it later → local goes to trash.
    let mut deleted = peer_song("sid-del", "Doomed Peer", "x", 20);
    deleted.deleted_at = Some(deleted.updated_at);
    repo.apply_sync_presentation(&deleted).await.unwrap();
    let trash = repo.list_trashed_presentations().await.unwrap();
    assert!(trash.iter().any(|t| t.sync_id == "sid-del"));

    // Peer restored it even later → local leaves the trash.
    let restored = peer_song("sid-del", "Doomed Peer", "x", 10);
    repo.apply_sync_presentation(&restored).await.unwrap();
    let trash = repo.list_trashed_presentations().await.unwrap();
    assert!(!trash.iter().any(|t| t.sync_id == "sid-del"));
}

#[tokio::test]
async fn apply_sync_presentation_remaps_stage_layout_markers_by_position() {
    // S9 regression: apply replaces slides WHOLESALE, carrying the PEER's
    // slide ids — every #515 stage-layout marker (keyed by slide_id) was
    // orphaned by this, silently wiping the losing site's markers for
    // ~every song on initial convergence. Fix: remap markers by slide
    // POSITION (old index -> new slide id at that same index); only markers
    // whose position no longer exists in the new slide list are dropped.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, local) = repo
        .create_presentation(
            lib.id,
            "Marked Song",
            Some(&[
                slide(0, "verse 1"),
                slide(1, "verse 2"),
                slide(2, "verse 3"),
            ]),
        )
        .await
        .unwrap();
    // Mark slide at position 1 with a stage layout.
    repo.set_slide_stage_layout(local.id, local.slides[1].id, "fulltext")
        .await
        .unwrap();

    let local_sync_id = row(&repo, local.id).await.sync_id;
    let peer_slides = vec![
        slide(0, "verse 1"),
        slide(1, "verse 2 (edited by peer)"),
        slide(2, "verse 3"),
    ];
    let incoming = crate::SyncPresentation {
        sync_id: local_sync_id,
        library_name: "Songs".to_string(),
        name: "Marked Song".to_string(),
        updated_at: chrono::Utc::now() + chrono::Duration::seconds(5),
        deleted_at: None,
        slides: peer_slides.clone(),
    };
    repo.apply_sync_presentation(&incoming).await.unwrap();

    let markers = repo.list_slide_stage_layouts(local.id).await.unwrap();
    assert_eq!(
        markers.len(),
        1,
        "the marker survives the sync apply (not silently wiped)"
    );
    let new_slide_id_at_position_1 = peer_slides[1].id.to_string();
    assert_eq!(
        markers.get(&new_slide_id_at_position_1),
        Some(&"fulltext".to_string()),
        "the marker is remapped to the peer's slide id at the SAME position"
    );
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
        name: "Marked Song".to_string(),
        updated_at: tombstone_at,
        deleted_at: Some(tombstone_at),
        slides: vec![slide(0, "verse 1"), slide(1, "verse 2")],
    };
    repo.apply_sync_presentation(&tombstone).await.unwrap();

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
async fn apply_sync_presentation_remaps_stage_layout_markers_by_content_on_a_pure_reorder() {
    // R4 regression: the position-keyed remap reattaches a marker to the
    // WRONG slide when the peer's version is a pure REORDER (no edits) —
    // the marker followed the old INDEX, not the verse it was actually on.
    // FIX: match old slide to new slide by CONTENT identity first (main /
    // translation / stage / group all equal); only fall back to position
    // when content doesn't settle it (e.g. the marked slide's text was
    // itself edited — see the position-based test above, which must keep
    // passing unchanged).
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, local) = repo
        .create_presentation(
            lib.id,
            "Reordered Song",
            Some(&[
                slide(0, "verse 1"),
                slide(1, "verse 2"),
                slide(2, "verse 3"),
            ]),
        )
        .await
        .unwrap();
    // Mark "verse 2", currently at position 1.
    repo.set_slide_stage_layout(local.id, local.slides[1].id, "fulltext")
        .await
        .unwrap();

    let local_sync_id = row(&repo, local.id).await.sync_id;
    // Peer reorders the SAME (unedited) slides: verse2 now leads.
    let peer_slides = vec![
        slide(0, "verse 2"),
        slide(1, "verse 1"),
        slide(2, "verse 3"),
    ];
    let incoming = crate::SyncPresentation {
        sync_id: local_sync_id,
        library_name: "Songs".to_string(),
        name: "Reordered Song".to_string(),
        updated_at: chrono::Utc::now() + chrono::Duration::seconds(5),
        deleted_at: None,
        slides: peer_slides.clone(),
    };
    repo.apply_sync_presentation(&incoming).await.unwrap();

    let markers = repo.list_slide_stage_layouts(local.id).await.unwrap();
    assert_eq!(markers.len(), 1, "the marker survives a pure reorder");
    let verse2_new_id = peer_slides[0].id.to_string();
    let verse1_new_id = peer_slides[1].id.to_string();
    assert_eq!(
        markers.get(&verse2_new_id),
        Some(&"fulltext".to_string()),
        "the marker follows VERSE 2 by content, even though it moved to position 0"
    );
    assert!(
        !markers.contains_key(&verse1_new_id),
        "the marker must NOT stay pinned to position 1 (that's verse 1 now)"
    );
}

#[tokio::test]
async fn manifest_lists_live_and_trashed_content_fetch_returns_slides() {
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, live) = repo
        .create_presentation(
            lib.id,
            "Live Song",
            Some(&[slide(0, "alpha"), slide(1, "beta")]),
        )
        .await
        .unwrap();
    let (_, _, trashed) = repo
        .create_presentation(lib.id, "Gone Song", Some(&[slide(0, "x")]))
        .await
        .unwrap();
    repo.delete_presentation(trashed.id).await.unwrap();

    let manifest = repo.list_sync_manifest().await.unwrap();
    assert_eq!(manifest.len(), 2, "manifest carries live AND trashed songs");
    let gone = manifest.iter().find(|r| r.name == "Gone Song").unwrap();
    assert!(gone.deleted_at.is_some(), "trashed row keeps its marker");
    let live_row = manifest.iter().find(|r| r.name == "Live Song").unwrap();
    assert!(live_row.deleted_at.is_none());

    let full = repo
        .fetch_sync_presentation(&live_row.sync_id)
        .await
        .unwrap()
        .expect("content fetch by sync_id");
    assert_eq!(full.library_name, "Songs");
    assert_eq!(full.slides.len(), 2);
    assert_eq!(full.slides[0].content.main.value(), "alpha");

    let _ = row(&repo, live.id).await; // row helper stays used across tasks
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

    // Age the other row 31 days back, then prune keeps only fresh trash.
    use sea_orm::{ColumnTrait, QueryFilter};
    let old_stamp = (chrono::Utc::now() - chrono::Duration::days(31)).to_rfc3339();
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
        .prune_deleted_presentations(chrono::Duration::days(30))
        .await
        .unwrap();
    assert_eq!(removed, 1, "only the 31-day-old row is pruned");
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
async fn reimport_preserves_trash_when_a_same_name_twin_shifts_the_song_to_a_derived_id() {
    // R1 regression: a previously-UNIQUE trashed song ("Amazing Grace",
    // name-derived sync_id) loses that very identity the moment a same-name
    // twin joins a LATER import — the cardinality shift means BOTH
    // occurrences now derive fresh sync_ids (#558 S3's content-pure rule),
    // so `old_trash_state.get(&new_sync_id)` misses the OLD (name-derived)
    // key entirely and the trashed song comes back LIVE with a fresh
    // `updated_at` — which then LWW-wins and propagates the resurrection to
    // the peer. FIX: key the old-state map by BOTH sync_id AND
    // (library_name, presentation_name); fall back to the name key on a
    // sync_id miss before defaulting to live/new.
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
    let deleted_at_before = trashed_before.deleted_at;
    let updated_at_before = trashed_before.updated_at;

    // Re-import: the SAME "Amazing Grace" content, plus a brand-new
    // same-name twin. Both now collide on the raw name-derived id, so BOTH
    // derive fresh sync_ids (S3) — the exact cardinality shift R1 flags.
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
        reimported_row.deleted_at.is_some(),
        "the original song's tombstone must survive even though its sync_id shifted"
    );
    assert_eq!(
        reimported_row.deleted_at, deleted_at_before,
        "the tombstone's own timestamp survives unchanged"
    );
    assert_eq!(
        reimported_row.updated_at, updated_at_before,
        "re-import must not manufacture a newer edit-time for the trashed row"
    );
    assert!(
        twin_row.deleted_at.is_none(),
        "the brand-new twin must NOT inherit the other row's tombstone"
    );
}

#[tokio::test]
async fn upsert_library_prefers_domain_sync_id_and_derives_the_rest() {
    let repo = repo().await;
    let with_uuid = presenter_core::Presentation::new("Imported", vec![slide(0, "a")])
        .unwrap()
        .with_sync_id("PRO-UUID-123");
    let without_uuid = presenter_core::Presentation::new("Handmade", vec![slide(0, "b")]).unwrap();
    let library = presenter_core::Library::new(
        "Songs".to_string(),
        vec![with_uuid.clone(), without_uuid.clone()],
    )
    .unwrap();
    repo.upsert_library(&library).await.unwrap();

    let imported = row(&repo, with_uuid.id).await;
    assert_eq!(imported.sync_id, "PRO-UUID-123", ".pro UUID wins");

    let handmade = row(&repo, without_uuid.id).await;
    assert_eq!(
        handmade.sync_id,
        presenter_core::sync_id_for_name("Songs", "Handmade"),
        "no .pro UUID → deterministic name-based identity"
    );
}

#[tokio::test]
async fn upsert_library_deduplicates_repeated_pro_uuids_deterministically() {
    // S3 FIX (this test's assertions changed on purpose — see #558 review):
    // the OLD dedupe let the "first occurrence in this library's own scan"
    // keep the raw UUID (a mutable `used` HashSet seeded from the DB, order
    // + import-history dependent) while every LATER duplicate derived. That
    // is exactly the history dependence #558 flags: WHICH song is "first"
    // is an accident of processing order, not of content. The fixed rule is
    // content-pure: a raw UUID is kept ONLY when it occurs exactly ONCE
    // across the whole current import scan; EVERY occurrence of an in-scan
    // duplicate derives UUIDv5(raw/library/name) — no exception for the
    // first one.
    let repo = repo().await;
    let first = presenter_core::Presentation::new("Original", vec![slide(0, "a")])
        .unwrap()
        .with_sync_id("DUP-UUID");
    let second = presenter_core::Presentation::new("Copy Of Original", vec![slide(0, "b")])
        .unwrap()
        .with_sync_id("DUP-UUID");
    let library =
        presenter_core::Library::new("Songs".to_string(), vec![first.clone(), second.clone()])
            .unwrap();
    repo.upsert_library(&library)
        .await
        .expect("duplicate .pro UUIDs within one import must not violate the unique index");

    let first_row = row(&repo, first.id).await;
    let second_row = row(&repo, second.id).await;
    assert_eq!(
        first_row.sync_id,
        uuid::Uuid::new_v5(
            &presenter_core::SYNC_ID_NAMESPACE,
            "DUP-UUID/Songs/Original".as_bytes(),
        )
        .to_string(),
        "an in-scan duplicate ALWAYS derives, even the first occurrence — no first-wins"
    );
    assert_eq!(
        second_row.sync_id,
        uuid::Uuid::new_v5(
            &presenter_core::SYNC_ID_NAMESPACE,
            "DUP-UUID/Songs/Copy Of Original".as_bytes(),
        )
        .to_string(),
        "the other duplicate derives too — deterministic, both instances compute the same"
    );
    assert_ne!(first_row.sync_id, second_row.sync_id);

    // Cross-library: since NEITHER Songs row kept the raw "DUP-UUID" (both
    // derived away above), a different library's presentation carrying the
    // SAME raw uuid is now content-pure unique-in-scan AND does not conflict
    // with any foreign DB row — so it keeps the raw value.
    let elsewhere = presenter_core::Presentation::new("Original", vec![slide(0, "c")])
        .unwrap()
        .with_sync_id("DUP-UUID");
    let other_lib =
        presenter_core::Library::new("Other".to_string(), vec![elsewhere.clone()]).unwrap();
    repo.upsert_library(&other_lib)
        .await
        .expect("cross-library duplicate must not violate the unique index");
    let elsewhere_row = row(&repo, elsewhere.id).await;
    assert_eq!(
        elsewhere_row.sync_id, "DUP-UUID",
        "no foreign row holds the raw id anymore, so it is free to keep it"
    );
}

#[tokio::test]
async fn upsert_library_dedup_is_independent_of_presentation_list_order() {
    // S3 regression: dedup must be a PURE function of import CONTENT — never
    // of the ORDER presentations happen to appear in the list (which is
    // filename-sort, an accident of the underlying directory listing, not a
    // property either site can rely on matching). Import the SAME two
    // duplicate-UUID presentations in FORWARD and REVERSED list order into
    // two separate databases and require IDENTICAL final sync_id
    // assignments either way.
    async fn build(
        order: [&str; 2],
    ) -> (
        Repository,
        presenter_core::PresentationId,
        presenter_core::PresentationId,
    ) {
        let repo = repo().await;
        let original = presenter_core::Presentation::new("Original", vec![slide(0, "a")])
            .unwrap()
            .with_sync_id("ORDER-DUP");
        let copy = presenter_core::Presentation::new("Copy Of Original", vec![slide(0, "b")])
            .unwrap()
            .with_sync_id("ORDER-DUP");
        let by_name: std::collections::HashMap<&str, presenter_core::Presentation> = [
            ("Original", original.clone()),
            ("Copy Of Original", copy.clone()),
        ]
        .into();
        let ordered: Vec<_> = order.iter().map(|name| by_name[name].clone()).collect();
        let library = presenter_core::Library::new("Songs".to_string(), ordered).unwrap();
        repo.upsert_library(&library).await.unwrap();
        (repo, original.id, copy.id)
    }

    let (forward_repo, forward_original, forward_copy) =
        build(["Original", "Copy Of Original"]).await;
    let (reversed_repo, reversed_original, reversed_copy) =
        build(["Copy Of Original", "Original"]).await;

    let forward_original_sid = row(&forward_repo, forward_original).await.sync_id;
    let forward_copy_sid = row(&forward_repo, forward_copy).await.sync_id;
    let reversed_original_sid = row(&reversed_repo, reversed_original).await.sync_id;
    let reversed_copy_sid = row(&reversed_repo, reversed_copy).await.sync_id;

    assert_eq!(
        forward_original_sid, reversed_original_sid,
        "\"Original\"'s assigned identity must not depend on list order"
    );
    assert_eq!(
        forward_copy_sid, reversed_copy_sid,
        "\"Copy Of Original\"'s assigned identity must not depend on list order"
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
async fn create_presentation_persists_sync_id_and_updated_at() {
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, pres) = repo
        .create_presentation(lib.id, "New Song", Some(&[slide(0, "verse")]))
        .await
        .unwrap();
    let model = row(&repo, pres.id).await;
    assert!(!model.sync_id.is_empty(), "create must assign a sync_id");
    assert!(model.deleted_at.is_none(), "a new song is not trashed");
    // updated_at is NOT NULL (the entity type guarantees it deserialized).
    let _ = model.updated_at;
}
