//! #558 S10 regression: search must never surface a trashed (soft-deleted)
//! song via its slide-lyrics text. Kept in its own file rather than
//! `tests.rs` (already over the file-size cap) per the presenter-ci playbook.
use crate::Repository;
use presenter_core::{Library, Presentation, SearchResultKind, Slide, SlideContent, SlideText};

async fn repo() -> Repository {
    Repository::connect_in_memory()
        .await
        .expect("in-memory repo")
}

#[tokio::test]
async fn trashed_song_lyrics_are_not_searchable() {
    // S10 regression: search_presenter's two NAME-search phases
    // (search_libraries' matched-presentations prefetch, search_presentations
    // itself) filter deleted_at IS NULL, but the slide-TEXT phase
    // (search_slides) did not — a trashed song's lyrics still surfaced it.
    let repo = repo().await;
    let slide = Slide::new(
        0,
        SlideContent::new(
            SlideText::new("A very distinctive lyric line").unwrap(),
            SlideText::new("").unwrap(),
            SlideText::new("").unwrap(),
            None,
        ),
    );
    let presentation = Presentation::new("Doomed Song", vec![slide]).unwrap();
    let pres_id = presentation.id;
    let library = Library::new("Songs", vec![presentation]).unwrap();
    repo.upsert_library(&library).await.unwrap();

    // Sanity: searchable while live.
    let before = repo
        .search_presenter("distinctive lyric", 10)
        .await
        .unwrap();
    assert!(
        before
            .iter()
            .any(|r| matches!(r.kind, SearchResultKind::Presentation)),
        "the live song's lyrics must be searchable"
    );

    repo.delete_presentation(pres_id).await.unwrap();

    let after = repo
        .search_presenter("distinctive lyric", 10)
        .await
        .unwrap();
    assert!(
        after.is_empty(),
        "a trashed song's lyrics must never surface in search results, got: {after:?}"
    );
}

#[tokio::test]
async fn trashed_library_is_not_searchable_by_name() {
    // #578 review gap: `delete_library` now soft-deletes the library row
    // itself (tombstone, not hard-delete) so the deletion syncs correctly —
    // but `search_libraries` never gained a `deleted_at IS NULL` filter, so
    // a tombstoned library still matched by NAME and surfaced as a
    // Library-kind search result even though it is hidden from
    // `fetch_libraries` / `list_library_summaries`.
    let repo = repo().await;
    let library = Library::new("VeryUniqueLibraryName", Vec::new()).unwrap();
    let library_id = library.id;
    repo.upsert_library(&library).await.unwrap();

    // Sanity: searchable while live.
    let before = repo
        .search_presenter("VeryUniqueLibraryName", 10)
        .await
        .unwrap();
    assert!(
        before
            .iter()
            .any(|r| matches!(r.kind, SearchResultKind::Library)),
        "the live library must be searchable by name"
    );

    repo.delete_library(library_id).await.unwrap();

    let after = repo
        .search_presenter("VeryUniqueLibraryName", 10)
        .await
        .unwrap();
    assert!(
        after.is_empty(),
        "a tombstoned library must never surface in search results, got: {after:?}"
    );
}
