//! #555 song-sync repository tests: identity, LWW apply, soft-delete, trash.
//! Add further `use` imports (ColumnTrait/QueryFilter/etc.) in the task that first needs
//! them — keep the file clippy-clean (`-D warnings` forbids unused imports) at every commit.
use crate::entities::presentation as presentation_entity;
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
