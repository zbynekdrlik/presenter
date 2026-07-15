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
    // Real ProPresenter libraries contain DUPLICATED .pro files, so two
    // presentations can carry the SAME embedded protobuf UUID. The sync_id
    // unique index must survive that: the first occurrence keeps the raw UUID,
    // every later duplicate gets a DERIVED — deterministic, not random — id so
    // both instances still converge on identical identities.
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
        first_row.sync_id, "DUP-UUID",
        "first occurrence keeps the raw UUID"
    );
    assert_ne!(
        second_row.sync_id, "DUP-UUID",
        "duplicate gets a distinct id"
    );
    assert_eq!(
        second_row.sync_id,
        uuid::Uuid::new_v5(
            &presenter_core::SYNC_ID_NAMESPACE,
            "DUP-UUID/Songs/Copy Of Original".as_bytes(),
        )
        .to_string(),
        "the derived id is deterministic (both instances compute the same)"
    );

    // Cross-library duplicate: another library importing the same file also derives.
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
        elsewhere_row.sync_id,
        uuid::Uuid::new_v5(
            &presenter_core::SYNC_ID_NAMESPACE,
            "DUP-UUID/Other/Original".as_bytes(),
        )
        .to_string(),
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
