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

#[tokio::test]
async fn presentation_under_a_tombstoned_library_is_not_searchable() {
    // #635: `search_presentations` filtered the PRESENTATION's own
    // `deleted_at` but never checked whether its PARENT LIBRARY was
    // tombstoned; `search_slides` had the same gap on the slide-TEXT phase
    // (and even fell back to a BLANK library name via `unwrap_or_default()`
    // instead of skipping). A presentation can end up live under a
    // tombstoned library through the sync-apply race #634 fixes (or any
    // other bug) -- search must exclude it defensively regardless of how the
    // inconsistent state arose, so this seeds it directly by tombstoning
    // ONLY the library (never through `delete_library`, which would also
    // tombstone the presentation and mask this exact gap).
    let repo = repo().await;
    let slide = Slide::new(
        0,
        SlideContent::new(
            SlideText::new("A very distinctive lyric line two").unwrap(),
            SlideText::new("").unwrap(),
            SlideText::new("").unwrap(),
            None,
        ),
    );
    let presentation = Presentation::new("Orphaned Under Dead Library", vec![slide]).unwrap();
    let library = Library::new("WillBeTombstonedOnly", vec![presentation]).unwrap();
    let library_id = library.id;
    repo.upsert_library(&library).await.unwrap();

    // Sanity: searchable while the library is live, both by name and by
    // slide text.
    let before_name = repo
        .search_presenter("Orphaned Under Dead Library", 10)
        .await
        .unwrap();
    assert!(
        before_name
            .iter()
            .any(|r| matches!(r.kind, SearchResultKind::Presentation)),
        "sanity: the live presentation must be searchable by name"
    );
    let before_text = repo
        .search_presenter("distinctive lyric line two", 10)
        .await
        .unwrap();
    assert!(
        before_text
            .iter()
            .any(|r| matches!(r.kind, SearchResultKind::Presentation)),
        "sanity: the live presentation's lyrics must be searchable"
    );

    // Tombstone ONLY the library row directly -- the presentation underneath
    // stays LIVE, reproducing the inconsistent state search must defend
    // against regardless of its origin.
    use crate::entities::library as library_entity;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    library_entity::Entity::update_many()
        .col_expr(
            library_entity::Column::DeletedAt,
            sea_orm::sea_query::Expr::value(chrono::Utc::now().to_rfc3339()),
        )
        .filter(library_entity::Column::Id.eq(library_id.to_string()))
        .exec(&repo.db)
        .await
        .unwrap();

    let after_name = repo
        .search_presenter("Orphaned Under Dead Library", 10)
        .await
        .unwrap();
    assert!(
        after_name.is_empty(),
        "a presentation under a tombstoned library must not surface by name, \
         got: {after_name:?}"
    );

    let after_text = repo
        .search_presenter("distinctive lyric line two", 10)
        .await
        .unwrap();
    assert!(
        after_text.is_empty(),
        "a presentation under a tombstoned library must not surface by \
         slide text, got: {after_text:?}"
    );
}
