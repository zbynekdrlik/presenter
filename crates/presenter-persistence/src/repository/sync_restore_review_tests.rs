//! #646 finding: `restore_presentation` must distinguish "the parent
//! library row is entirely missing" (404) from "the parent library is
//! still tombstoned" (409) — the old `is_none_or` folded both into the same
//! `Conflict`. Kept in its own sibling file rather than growing
//! `sync_trash_tests.rs` (853 prod lines) toward the 1000-line hard cap.
use super::sync_test_support::{row, slide};
use crate::entities::library;
use crate::RepositoryError;

/// Build a Repository on a DEDICATED single-connection in-memory database so
/// a raw `PRAGMA foreign_keys = OFF` reliably applies to the SAME connection
/// that then hard-deletes a library row — letting a test construct a
/// presentation whose `library_id` points at NO row at all, a state the
/// FK-enforced schema makes unreachable through any normal write path
/// (mirrors `search_trash_tests.rs`'s `repo_allowing_fk_bypass`).
async fn repo_allowing_fk_bypass() -> crate::Repository {
    use presenter_migration::MigratorTrait;
    use sea_orm::{ConnectOptions, Database};
    let mut opts = ConnectOptions::new("sqlite::memory:");
    opts.max_connections(1).min_connections(1);
    let db = Database::connect(opts).await.expect("connect");
    crate::Repository::apply_sqlite_pragmas(&db)
        .await
        .expect("pragmas");
    presenter_migration::Migrator::up(&db, None)
        .await
        .expect("migrate");
    crate::Repository { db }
}

#[tokio::test]
async fn restore_presentation_returns_not_found_when_the_parent_library_row_is_entirely_missing() {
    // #646: the OLD `is_none_or` folded "library row missing entirely" into
    // the SAME Conflict/409 as "library still tombstoned" -- a missing row
    // is a DIFFERENT, NotFound-shaped (404) situation.
    let repo = repo_allowing_fk_bypass().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let (_, _, song) = repo
        .create_presentation(lib.id, "Orphaned Trash", Some(&[slide(0, "a")]))
        .await
        .unwrap();
    repo.delete_presentation(song.id).await.unwrap();

    // Hard-delete ONLY the library row, bypassing the FK's own ON DELETE
    // CASCADE (which would otherwise remove the still-trashed presentation
    // right along with it) -- the presentation row is left dangling,
    // exactly the state `restore_presentation` must defend against.
    use sea_orm::{ConnectionTrait, EntityTrait};
    repo.db
        .execute_unprepared("PRAGMA foreign_keys = OFF")
        .await
        .unwrap();
    library::Entity::delete_by_id(lib.id.to_string())
        .exec(&repo.db)
        .await
        .unwrap();
    repo.db
        .execute_unprepared("PRAGMA foreign_keys = ON")
        .await
        .unwrap();

    let err = repo
        .restore_presentation(song.id)
        .await
        .expect_err("restore must refuse when the parent library row is gone");
    assert!(
        matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::NotFound(_))
        ),
        "a missing library row is NotFound (404), not Conflict (409) -- got: {err}"
    );

    let still_trashed = row(&repo, song.id).await;
    assert!(
        still_trashed.deleted_at.is_some(),
        "a refused restore must leave the presentation exactly as tombstoned \
         as before -- no partial mutation"
    );
}
