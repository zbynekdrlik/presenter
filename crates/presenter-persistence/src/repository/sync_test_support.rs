//! Shared helpers for the #555 song-sync test modules (`sync_tests` +
//! `sync_trash_tests`). Split out when the single test file crossed the
//! 1000-line hard cap (#558 quality gate) — pure mechanical move, no logic.
use crate::entities::presentation as presentation_entity;
use crate::Repository;
use presenter_core::{PresentationId, Slide, SlideContent, SlideText};
use sea_orm::EntityTrait;

pub(super) async fn repo() -> Repository {
    Repository::connect_in_memory()
        .await
        .expect("in-memory repo")
}

pub(super) fn slide(order: u32, main: &str) -> Slide {
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
pub(super) async fn row(repo: &Repository, id: PresentationId) -> presentation_entity::Model {
    presentation_entity::Entity::find_by_id(id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("presentation row exists")
}

pub(super) async fn updated_at_of(
    repo: &Repository,
    id: PresentationId,
) -> chrono::DateTime<chrono::Utc> {
    row(repo, id).await.updated_at.into()
}

/// `library_sync_id: None` — simulates an OLD, name-only peer (the pre-#647
/// wire shape). Every existing test built on this helper keeps exercising
/// the compat-fallback (name-only) path, unchanged. Use
/// `peer_song_with_library_sync_id` for a test that needs the NEW
/// identity-join behavior.
pub(super) fn peer_song(
    sync_id: &str,
    name: &str,
    main: &str,
    minutes_ago: i64,
) -> crate::SyncPresentation {
    crate::SyncPresentation {
        sync_id: sync_id.to_string(),
        library_name: "Songs".to_string(),
        library_sync_id: None,
        name: name.to_string(),
        updated_at: chrono::Utc::now() - chrono::Duration::minutes(minutes_ago),
        deleted_at: None,
        slides: vec![slide(0, main)],
    }
}

/// Same shape as `peer_song`, but with an explicit (`Some`) `library_sync_id`
/// — for a #647 test that needs the NEW identity-join behavior instead of
/// `peer_song`'s always-`None` compat-fallback default. `deleted_at` is
/// always `None` (a live entry); a test needing a tombstone constructs its
/// own literal.
pub(super) fn peer_song_with_library_sync_id(
    sync_id: &str,
    library_sync_id: &str,
    library_name: &str,
    name: &str,
    main: &str,
    minutes_ago: i64,
) -> crate::SyncPresentation {
    crate::SyncPresentation {
        sync_id: sync_id.to_string(),
        library_name: library_name.to_string(),
        library_sync_id: Some(library_sync_id.to_string()),
        name: name.to_string(),
        updated_at: chrono::Utc::now() - chrono::Duration::minutes(minutes_ago),
        deleted_at: None,
        slides: vec![slide(0, main)],
    }
}
