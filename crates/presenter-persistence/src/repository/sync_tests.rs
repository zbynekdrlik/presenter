//! #555 song-sync repository tests: identity, LWW apply, soft-delete, trash.
//! Add further `use` imports (ColumnTrait/QueryFilter/etc.) in the task that first needs
//! them — keep the file clippy-clean (`-D warnings` forbids unused imports) at every commit.
use crate::entities::{playlist_entry, presentation as presentation_entity};
use crate::Repository;
use presenter_core::{PresentationId, Slide, SlideContent, SlideText};
use sea_orm::EntityTrait;

async fn repo() -> Repository {
    Repository::connect_in_memory().await.expect("in-memory repo")
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

#[tokio::test]
async fn upsert_library_prefers_domain_sync_id_and_derives_the_rest() {
    let repo = repo().await;
    let with_uuid = presenter_core::Presentation::new("Imported", vec![slide(0, "a")])
        .unwrap()
        .with_sync_id("PRO-UUID-123");
    let without_uuid =
        presenter_core::Presentation::new("Handmade", vec![slide(0, "b")]).unwrap();
    let library =
        presenter_core::Library::new("Songs".to_string(), vec![with_uuid.clone(), without_uuid.clone()])
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
        repo.fetch_presentation_detail(pres.id).await.unwrap().is_none(),
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
