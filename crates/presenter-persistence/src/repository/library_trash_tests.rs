//! #644 library-restore tests: soft-delete/list/restore, the 404-vs-409
//! split, the cascade-scoped restore (the crux of the settled semantics —
//! only the presentations THIS library's own deletion cascaded come back),
//! and the live-name collision guard.
use super::sync_test_support::{repo, slide};
use crate::entities::{library, presentation as presentation_entity};
use crate::RepositoryError;
use sea_orm::EntityTrait;

async fn library_row(repo: &crate::Repository, id: presenter_core::LibraryId) -> library::Model {
    library::Entity::find_by_id(id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("library row exists")
}

async fn presentation_row(
    repo: &crate::Repository,
    id: presenter_core::PresentationId,
) -> presentation_entity::Model {
    presentation_entity::Entity::find_by_id(id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("presentation row exists")
}

#[tokio::test]
async fn trash_lists_a_deleted_library_and_restore_clears_it_and_bumps_the_clock() {
    let repo = repo().await;
    let lib = repo.create_library("Doomed Library").await.unwrap();
    repo.delete_library(lib.id).await.unwrap();

    let trash = repo.list_trashed_libraries().await.unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].name, "Doomed Library");

    let before = library_row(&repo, lib.id).await.updated_at;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    repo.restore_library(lib.id).await.unwrap();

    let restored = library_row(&repo, lib.id).await;
    assert!(
        restored.deleted_at.is_none(),
        "restore clears the tombstone"
    );
    assert!(
        restored.updated_at > before,
        "restore bumps updated_at so it wins LWW on the peer's next pull"
    );

    let trash_after = repo.list_trashed_libraries().await.unwrap();
    assert!(trash_after.is_empty(), "restored library leaves the trash");

    let libs = repo.fetch_libraries().await.unwrap();
    assert!(
        libs.iter().any(|l| l.name == "Doomed Library"),
        "the restored library is reachable again"
    );
}

#[tokio::test]
async fn restore_library_returns_not_found_when_the_library_row_is_entirely_missing() {
    let repo = repo().await;
    let missing_id = presenter_core::LibraryId::new();

    let err = repo
        .restore_library(missing_id)
        .await
        .expect_err("restoring a nonexistent library must refuse");
    assert!(
        matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::NotFound(_))
        ),
        "expected RepositoryError::NotFound, got: {err}"
    );
}

#[tokio::test]
async fn restore_library_returns_conflict_when_the_library_is_not_trashed() {
    let repo = repo().await;
    let lib = repo.create_library("Live Library").await.unwrap();

    let err = repo
        .restore_library(lib.id)
        .await
        .expect_err("restoring a LIVE library must refuse");
    assert!(
        matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::Conflict(_))
        ),
        "expected RepositoryError::Conflict, got: {err}"
    );
}

#[tokio::test]
async fn restore_library_restores_only_the_presentations_its_own_cascade_tombstoned() {
    // #644 crux case: `delete_library` cascades every LIVE presentation into
    // the SAME tombstone as the library. A presentation trashed
    // INDEPENDENTLY (its own delete_presentation call, at a different
    // instant) must stay trashed when the library is later restored — only
    // the cascaded one comes back.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, cascaded) = repo
        .create_presentation(lib.id, "Cascaded Song", Some(&[slide(0, "a")]))
        .await
        .unwrap();
    let (_, _, independently_trashed) = repo
        .create_presentation(lib.id, "Independently Trashed Song", Some(&[slide(0, "b")]))
        .await
        .unwrap();

    // Trash one song on its own, BEFORE the library is deleted.
    repo.delete_presentation(independently_trashed.id)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    // delete_library tombstones the library AND the still-live "Cascaded
    // Song" together, in one transaction.
    repo.delete_library(lib.id).await.unwrap();

    let cascaded_deleted_at_before = presentation_row(&repo, cascaded.id).await.deleted_at;
    let independent_deleted_at_before = presentation_row(&repo, independently_trashed.id)
        .await
        .deleted_at;
    assert!(cascaded_deleted_at_before.is_some());
    assert!(independent_deleted_at_before.is_some());
    assert_ne!(
        cascaded_deleted_at_before, independent_deleted_at_before,
        "sanity: the two songs were trashed at DIFFERENT instants"
    );

    repo.restore_library(lib.id).await.unwrap();

    let cascaded_after = presentation_row(&repo, cascaded.id).await;
    assert!(
        cascaded_after.deleted_at.is_none(),
        "the cascaded song comes back with the library"
    );

    let independent_after = presentation_row(&repo, independently_trashed.id).await;
    assert!(
        independent_after.deleted_at.is_some(),
        "the independently-trashed song must stay trashed"
    );
    assert_eq!(
        independent_after.deleted_at, independent_deleted_at_before,
        "the independently-trashed song's tombstone is untouched"
    );

    let libs = repo.fetch_libraries().await.unwrap();
    let restored_lib = libs
        .iter()
        .find(|l| l.name == "Songs")
        .expect("library restored");
    assert!(restored_lib
        .presentations
        .iter()
        .any(|p| p.name == "Cascaded Song"));
    assert!(!restored_lib
        .presentations
        .iter()
        .any(|p| p.name == "Independently Trashed Song"));
}

#[tokio::test]
async fn restore_library_refuses_when_a_live_library_already_has_the_name() {
    // `idx_libraries_name_live_unique` only guards LIVE rows, so a live
    // library can legitimately claim the trashed one's name WHILE it sits
    // in the trash. Restoring must refuse with a typed Conflict (409), not
    // a raw SQLite constraint violation surfacing as a bare 500.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    repo.delete_library(lib.id).await.unwrap();

    // A fresh library legitimately reclaims the now-free name.
    repo.create_library("Songs").await.unwrap();

    let err = repo
        .restore_library(lib.id)
        .await
        .expect_err("restoring into a live-name collision must refuse");
    assert!(
        matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::Conflict(_))
        ),
        "expected RepositoryError::Conflict, got: {err}"
    );

    // The refused restore leaves the trashed row exactly as it was.
    let still_trashed = library_row(&repo, lib.id).await;
    assert!(still_trashed.deleted_at.is_some());
}
