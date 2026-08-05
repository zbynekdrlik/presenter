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

fn slide_with_text(main: &str) -> Slide {
    Slide::new(
        0,
        SlideContent::new(
            SlideText::new(main).unwrap(),
            SlideText::new("").unwrap(),
            SlideText::new("").unwrap(),
            None,
        ),
    )
}

/// Build a Repository on a DEDICATED single-connection in-memory database
/// (own schema copy, not the shared-cache pool `repo()` uses) so a raw
/// `PRAGMA foreign_keys = OFF` reliably applies to the SAME connection that
/// then hard-deletes a library row — letting a test construct a presentation
/// whose `library_id` points at NO row at all, a state the FK-enforced
/// schema makes unreachable through any normal write path (#646 test
/// hardening for `backfill_live_library_names`'s silent drop).
async fn repo_allowing_fk_bypass() -> Repository {
    use presenter_migration::MigratorTrait;
    use sea_orm::{ConnectOptions, Database};
    let mut opts = ConnectOptions::new("sqlite::memory:");
    opts.max_connections(1).min_connections(1);
    let db = Database::connect(opts).await.expect("connect");
    Repository::apply_sqlite_pragmas(&db)
        .await
        .expect("pragmas");
    presenter_migration::Migrator::up(&db, None)
        .await
        .expect("migrate");
    Repository { db }
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

/// Tombstone ONLY the given library row directly, leaving its live
/// presentations underneath untouched — mirrors
/// `presentation_under_a_tombstoned_library_is_not_searchable` above.
async fn tombstone_library_row_only(repo: &Repository, library_id: presenter_core::LibraryId) {
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
}

#[tokio::test]
async fn search_limit_is_not_consumed_by_presentations_under_a_tombstoned_library() {
    // #646: `search_presentations`'s `.limit(remaining)` was applied BEFORE
    // the Rust-side tombstoned-library exclusion, so anomalous
    // live-under-tombstoned rows sorting alphabetically EARLY consumed the
    // LIMIT budget, starving a genuine live match sorting later in the same
    // page. Seed 2 such anomalies (named to sort first) plus ONE genuine
    // live match (sorts last) and search with limit=2 -- the genuine match
    // must still come back.
    let repo = repo().await;

    let anomaly_lib = Library::new(
        "AnomalyLib",
        vec![
            Presentation::new("Aardvark Anomaly Match", Vec::new()).unwrap(),
            Presentation::new("Bumblebee Anomaly Match", Vec::new()).unwrap(),
        ],
    )
    .unwrap();
    let anomaly_lib_id = anomaly_lib.id;
    repo.upsert_library(&anomaly_lib).await.unwrap();

    let genuine_lib = Library::new(
        "GenuineLib",
        vec![Presentation::new("Zebra Anomaly Match", Vec::new()).unwrap()],
    )
    .unwrap();
    repo.upsert_library(&genuine_lib).await.unwrap();

    tombstone_library_row_only(&repo, anomaly_lib_id).await;

    let results = repo.search_presenter("Anomaly Match", 2).await.unwrap();
    assert!(
        results
            .iter()
            .any(|r| r.presentation_name.as_deref() == Some("Zebra Anomaly Match")),
        "the genuine live match must not be starved by anomalies consuming \
         the LIMIT budget ahead of it, got: {results:?}"
    );
    assert!(
        results.iter().all(|r| {
            r.presentation_name.as_deref() != Some("Aardvark Anomaly Match")
                && r.presentation_name.as_deref() != Some("Bumblebee Anomaly Match")
        }),
        "the anomalies themselves must never surface, got: {results:?}"
    );
}

#[tokio::test]
async fn search_limit_is_not_consumed_by_slides_under_a_tombstoned_library() {
    // #646: same LIMIT-starvation shape as the presentations test above,
    // one level down in `search_slides` (the join is via
    // `presentation_entity`, not `library` directly). Insert the anomalies
    // FIRST -- tied Position=0, no secondary sort in the slide-text query,
    // so SQLite's tie-break falls back to insertion/rowid order, exactly
    // what an unfiltered LIMIT would return first.
    let repo = repo().await;

    let anomaly_lib = Library::new(
        "AnomalyLib2",
        vec![
            Presentation::new("A1", vec![slide_with_text("Starving Lyric One")]).unwrap(),
            Presentation::new("A2", vec![slide_with_text("Starving Lyric Two")]).unwrap(),
        ],
    )
    .unwrap();
    let anomaly_lib_id = anomaly_lib.id;
    repo.upsert_library(&anomaly_lib).await.unwrap();

    let genuine_lib = Library::new(
        "GenuineLib2",
        vec![Presentation::new("G1", vec![slide_with_text("Starving Lyric Three")]).unwrap()],
    )
    .unwrap();
    repo.upsert_library(&genuine_lib).await.unwrap();

    tombstone_library_row_only(&repo, anomaly_lib_id).await;

    let results = repo.search_presenter("Starving Lyric", 2).await.unwrap();
    assert!(
        results
            .iter()
            .any(|r| r.presentation_name.as_deref() == Some("G1")),
        "the genuine live slide match must not be starved by anomalies \
         consuming the LIMIT budget ahead of it, got: {results:?}"
    );
}

#[tokio::test]
async fn slide_under_a_library_row_that_no_longer_exists_at_all_is_dropped_silently() {
    // #646 test hardening: `backfill_live_library_names` already skips a
    // TOMBSTONED library (see the test above); this pins the OTHER branch
    // of the SAME `None` check -- a library row that is ENTIRELY MISSING
    // (not merely tombstoned) must be dropped the same way, never falling
    // back to a blank name.
    let repo = repo_allowing_fk_bypass().await;
    let presentation = Presentation::new(
        "Orphaned By Deletion",
        vec![slide_with_text("A vanished library lyric")],
    )
    .unwrap();
    let library = Library::new("WillBeHardDeleted", vec![presentation]).unwrap();
    let library_id = library.id;
    repo.upsert_library(&library).await.unwrap();

    // Sanity: searchable while the library row still exists.
    let before = repo
        .search_presenter("vanished library lyric", 10)
        .await
        .unwrap();
    assert!(
        before
            .iter()
            .any(|r| matches!(r.kind, SearchResultKind::Presentation)),
        "sanity: searchable while the library row still exists"
    );

    // Hard-delete ONLY the library row, bypassing the FK's own ON DELETE
    // CASCADE (which would otherwise remove the presentation + slide right
    // along with it) -- the presentation row is left dangling, exactly the
    // state `backfill_live_library_names` must defend against.
    use crate::entities::library as library_entity;
    use sea_orm::{ConnectionTrait, EntityTrait};
    repo.db
        .execute_unprepared("PRAGMA foreign_keys = OFF")
        .await
        .unwrap();
    library_entity::Entity::delete_by_id(library_id.to_string())
        .exec(&repo.db)
        .await
        .unwrap();
    repo.db
        .execute_unprepared("PRAGMA foreign_keys = ON")
        .await
        .unwrap();

    let after = repo
        .search_presenter("vanished library lyric", 10)
        .await
        .unwrap();
    assert!(
        after.is_empty(),
        "a slide whose library row no longer exists at all must never \
         surface in search results, got: {after:?}"
    );
}
