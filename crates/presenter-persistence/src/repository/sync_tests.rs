//! #555 song-sync repository tests: identity, LWW apply, adopt-by-name, and
//! stage-layout marker remapping. Trash/tombstone/carryover tests live in
//! `sync_trash_tests.rs` (split when this file crossed the 1000-line cap);
//! shared helpers in `sync_test_support`.
//! Add further `use` imports (ColumnTrait/QueryFilter/etc.) in the task that first needs
//! them — keep the file clippy-clean (`-D warnings` forbids unused imports) at every commit.
use super::sync_test_support::{peer_song, repo, row, slide, updated_at_of};
use crate::Repository;
use presenter_core::{SlideContent, SlideText};
use sea_orm::EntityTrait;

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
async fn apply_sync_presentation_remaps_markers_without_a_position_fallback_collision() {
    // #558 round-3 T3: the POSITION fallback ignored `claimed`, so a marker
    // whose old content is AMBIGUOUS (shared by 2+ old slides, so it gets
    // no content match) could fall back to a position ANOTHER marker's
    // content match already claimed — a PK violation on
    // `slide_stage_layouts` (its primary key is `slide_id` alone; two
    // markers can never point at the same slide). Reproduces the exact
    // shape: OLD [verse(marked), chorus(marked), chorus] with the peer
    // reordering to [chorus, verse, chorus] — verse's content is unique so
    // it correctly claims its new slide; chorus's content is NOT unique
    // (two old chorus slides), so it falls back to its OLD position (1),
    // which the peer's reorder now gives to the very slide verse's content
    // match just claimed.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, local) = repo
        .create_presentation(
            lib.id,
            "Marked Song",
            Some(&[slide(0, "verse"), slide(1, "chorus"), slide(2, "chorus")]),
        )
        .await
        .unwrap();
    repo.set_slide_stage_layout(local.id, local.slides[0].id, "verse-layout")
        .await
        .unwrap();
    repo.set_slide_stage_layout(local.id, local.slides[1].id, "chorus-layout")
        .await
        .unwrap();

    let local_sync_id = row(&repo, local.id).await.sync_id;
    let peer_slides = vec![slide(0, "chorus"), slide(1, "verse"), slide(2, "chorus")];
    let incoming = crate::SyncPresentation {
        sync_id: local_sync_id,
        library_name: "Songs".to_string(),
        name: "Marked Song".to_string(),
        updated_at: chrono::Utc::now() + chrono::Duration::seconds(5),
        deleted_at: None,
        slides: peer_slides.clone(),
    };
    // Must not error — a PK violation here IS the regression.
    repo.apply_sync_presentation(&incoming)
        .await
        .expect("sync apply must not fail with a primary-key violation");

    let markers = repo.list_slide_stage_layouts(local.id).await.unwrap();
    let verse_new_id = peer_slides[1].id.to_string();
    assert_eq!(
        markers.get(&verse_new_id),
        Some(&"verse-layout".to_string()),
        "the verse marker follows its content to the new position, collision-free"
    );
    assert_eq!(
        markers.len(),
        1,
        "the ambiguous chorus marker's position fallback is already claimed by the verse \
         marker's content match — it is dropped, never double-inserted onto the same slide"
    );
}

#[tokio::test]
async fn apply_sync_presentation_remaps_a_marker_on_an_oversize_legacy_stored_slide() {
    // #558 round-3 T7: `markers_with_content` re-validated the OLD slide's
    // STORED text through `SlideText::new` — a legacy/wire-path row that
    // predates (or bypassed, e.g. deserialized straight off the wire) the
    // 4000-char limit then makes EVERY future sync apply of that marked
    // song fail FOREVER (the same oversize stored text is read back and
    // re-validated on every single apply attempt, never just once). FIX:
    // match by RAW column strings, never re-validated through
    // `SlideText::new`.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, local) = repo
        .create_presentation(lib.id, "Legacy Song", Some(&[slide(0, "short")]))
        .await
        .unwrap();
    repo.set_slide_stage_layout(local.id, local.slides[0].id, "fulltext")
        .await
        .unwrap();

    // Simulate a legacy/wire-path row: write an OVERSIZE worship_main
    // directly, bypassing SlideText::new's validation entirely (a normal
    // repository call would reject it before it ever reaches the DB).
    let oversize = "x".repeat(5000);
    use crate::entities::slide as slide_entity;
    use sea_orm::{ColumnTrait, QueryFilter};
    slide_entity::Entity::update_many()
        .col_expr(
            slide_entity::Column::WorshipMain,
            sea_orm::sea_query::Expr::value(oversize),
        )
        .filter(slide_entity::Column::Id.eq(local.slides[0].id.to_string()))
        .exec(&repo.db)
        .await
        .unwrap();

    let local_sync_id = row(&repo, local.id).await.sync_id;
    let incoming = crate::SyncPresentation {
        sync_id: local_sync_id,
        library_name: "Songs".to_string(),
        name: "Legacy Song".to_string(),
        updated_at: chrono::Utc::now() + chrono::Duration::seconds(5),
        deleted_at: None,
        slides: vec![slide(0, "short (edited by peer)")],
    };
    // Must not error — re-validating the oversize OLD stored text IS the regression.
    repo.apply_sync_presentation(&incoming)
        .await
        .expect("sync apply must not choke on an oversize legacy stored slide");

    let markers = repo.list_slide_stage_layouts(local.id).await.unwrap();
    assert_eq!(
        markers.len(),
        1,
        "the marker survives via position fallback (the slide's own content changed)"
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
