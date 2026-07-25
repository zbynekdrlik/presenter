//! #578 unit tests for the library-sync machinery: `apply_sync_library`'s LWW
//! (sync_id match → adopt-by-name → create-or-skip), the prune, and
//! `rename_library`'s updated_at bump.
use super::Repository;
use crate::entities::library;
use crate::{SyncApplyOutcome, SyncLibraryManifestRow};
use chrono::{Duration, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

async fn repo() -> Repository {
    Repository::connect_in_memory()
        .await
        .expect("in-memory repo")
}

/// The stored `(sync_id, updated_at, deleted_at)` for the single LIVE-or-not
/// library row with this name (fails if there are 0 or 2+).
async fn library_row_by_name(repo: &Repository, name: &str) -> library::Model {
    library::Entity::find()
        .filter(library::Column::Name.eq(name.to_string()))
        .one(&repo.db)
        .await
        .unwrap()
        .expect("library row exists")
}

fn incoming(
    sync_id: &str,
    name: &str,
    minutes_from_now: i64,
    deleted: bool,
) -> SyncLibraryManifestRow {
    let ts = Utc::now() + Duration::minutes(minutes_from_now);
    SyncLibraryManifestRow {
        sync_id: sync_id.to_string(),
        name: name.to_string(),
        updated_at: ts,
        deleted_at: deleted.then_some(ts),
    }
}

#[tokio::test]
async fn create_from_unknown_live_sync_id() {
    let repo = repo().await;
    let outcome = repo
        .apply_sync_library(&incoming("peer-1", "Songs", 0, false))
        .await
        .unwrap();
    assert_eq!(outcome, SyncApplyOutcome::Created);
    let libs = repo.fetch_libraries().await.unwrap();
    assert_eq!(libs.len(), 1);
    assert_eq!(libs[0].name, "Songs");
    assert_eq!(library_row_by_name(&repo, "Songs").await.sync_id, "peer-1");
}

#[tokio::test]
async fn rename_in_place_by_sync_id_preserves_id_and_presentations() {
    let repo = repo().await;
    let lib = repo.create_library("Old Name").await.unwrap();
    repo.create_presentation(lib.id, "Song", None)
        .await
        .unwrap();
    let sid = library_row_by_name(&repo, "Old Name").await.sync_id;

    let outcome = repo
        .apply_sync_library(&incoming(&sid, "New Name", 1, false))
        .await
        .unwrap();
    assert_eq!(outcome, SyncApplyOutcome::Updated);

    let libs = repo.fetch_libraries().await.unwrap();
    assert_eq!(libs.len(), 1);
    assert_eq!(libs[0].name, "New Name", "rename applied in place");
    assert_eq!(libs[0].id, lib.id, "the local id (and its FK) is preserved");
    assert_eq!(
        libs[0].presentations.len(),
        1,
        "the presentation stays attached across the rename"
    );
}

#[tokio::test]
async fn tombstone_by_sync_id_hides_the_library() {
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let sid = library_row_by_name(&repo, "Songs").await.sync_id;

    let outcome = repo
        .apply_sync_library(&incoming(&sid, "Songs", 1, true))
        .await
        .unwrap();
    assert_eq!(outcome, SyncApplyOutcome::Updated);
    assert!(
        repo.fetch_libraries().await.unwrap().is_empty(),
        "the tombstoned library is hidden from the live listing"
    );
    // The row survives as a tombstone (still in the sync manifest).
    let row = library_row_by_name(&repo, "Songs").await;
    assert_eq!(row.id, lib.id.to_string());
    assert!(row.deleted_at.is_some());
}

#[tokio::test]
async fn adopt_by_name_converges_two_independently_created_libraries() {
    let repo = repo().await;
    repo.create_library("Songs").await.unwrap();
    let local_sid = library_row_by_name(&repo, "Songs").await.sync_id;

    // The peer's "Songs" (a DIFFERENT random sync_id, newer) adopts onto ours.
    let outcome = repo
        .apply_sync_library(&incoming("peer-songs", "Songs", 1, false))
        .await
        .unwrap();
    assert_eq!(outcome, SyncApplyOutcome::AdoptedByName);

    let row = library_row_by_name(&repo, "Songs").await;
    assert_eq!(
        row.sync_id, "peer-songs",
        "local adopted the peer's sync_id"
    );
    assert_ne!(row.sync_id, local_sid);
    assert_eq!(
        repo.fetch_libraries().await.unwrap().len(),
        1,
        "no duplicate library created — adopted in place"
    );
}

#[tokio::test]
async fn a_peer_tombstone_never_adopts_a_live_same_name_library() {
    // Decision B: a tombstone with an UNKNOWN sync_id must never reach across
    // and trash a live same-named local library — two sites can independently
    // hold different libraries that share a name.
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let local_sid = library_row_by_name(&repo, "Songs").await.sync_id;

    let outcome = repo
        .apply_sync_library(&incoming("peer-other", "Songs", 1, true))
        .await
        .unwrap();
    assert_eq!(
        outcome,
        SyncApplyOutcome::Created,
        "a tombstone with an unknown sync_id creates its OWN separate trashed row"
    );

    // Our live "Songs" is untouched (still live, still our sync_id).
    let libs = repo.fetch_libraries().await.unwrap();
    assert_eq!(libs.len(), 1);
    assert_eq!(
        libs[0].id, lib.id,
        "the live library is not trashed by the peer tombstone"
    );
    let rows = library::Entity::find()
        .filter(library::Column::Name.eq("Songs".to_string()))
        .all(&repo.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "one live row + one separate tombstone row");
    let live = rows.iter().find(|r| r.deleted_at.is_none()).unwrap();
    assert_eq!(live.sync_id, local_sid, "the live row keeps our identity");
}

#[tokio::test]
async fn a_recent_unknown_tombstone_is_created_as_trashed() {
    // A fresh/re-provisioned peer's first sync must materialize a recently
    // trashed library it never held, so trash contents converge (#558 R2).
    let repo = repo().await;
    let outcome = repo
        .apply_sync_library(&incoming("peer-trash", "Retired", -1, true))
        .await
        .unwrap();
    assert_eq!(outcome, SyncApplyOutcome::Created);
    assert!(
        repo.fetch_libraries().await.unwrap().is_empty(),
        "it's created in the trashed state — hidden from the live listing"
    );
    assert!(
        repo.list_library_sync_manifest()
            .await
            .unwrap()
            .iter()
            .any(|l| l.name == "Retired" && l.deleted_at.is_some()),
        "the tombstone exists in the manifest so it converges with the peer"
    );
}

#[tokio::test]
async fn an_unknown_tombstone_older_than_the_prune_horizon_is_never_resurrected() {
    let repo = repo().await;
    let mut old = incoming("peer-gone", "Ancient", 0, true);
    let ts = Utc::now() - Duration::days(40);
    old.updated_at = ts;
    old.deleted_at = Some(ts);
    let outcome = repo.apply_sync_library(&old).await.unwrap();
    assert_eq!(outcome, SyncApplyOutcome::SkippedNotNewer);
    assert!(
        repo.list_library_sync_manifest().await.unwrap().is_empty(),
        "an already-pruned tombstone must never be resurrected as a fresh row"
    );
}

#[tokio::test]
async fn skips_when_local_is_newer() {
    let repo = repo().await;
    repo.create_library("Songs").await.unwrap();
    let sid = library_row_by_name(&repo, "Songs").await.sync_id;
    // A peer edit OLDER than our local row must be skipped.
    let outcome = repo
        .apply_sync_library(&incoming(&sid, "Stale", -1, false))
        .await
        .unwrap();
    assert_eq!(outcome, SyncApplyOutcome::SkippedNotNewer);
    assert_eq!(
        library_row_by_name(&repo, "Songs").await.name,
        "Songs",
        "a stale peer edit did not overwrite the newer local name"
    );
}

#[tokio::test]
async fn rename_library_bumps_updated_at_for_propagation() {
    let repo = repo().await;
    let lib = repo.create_library("Before").await.unwrap();
    let before = library_row_by_name(&repo, "Before").await.updated_at;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    repo.rename_library(lib.id, "After").await.unwrap();
    let after = library_row_by_name(&repo, "After").await;
    assert!(
        after.updated_at > before,
        "rename must bump updated_at so the change propagates under LWW"
    );
}

#[tokio::test]
async fn rename_and_delete_missing_library_error() {
    let repo = repo().await;
    let missing = presenter_core::LibraryId::new();
    assert!(repo.rename_library(missing, "x").await.is_err());
    assert!(
        repo.delete_library(missing).await.is_err(),
        "delete of a missing library errors (the router maps it to 404)"
    );
}

#[tokio::test]
async fn prune_removes_only_old_tombstones() {
    let repo = repo().await;
    // A live library, plus a recent tombstone (via apply). The old tombstone
    // is inserted DIRECTLY — `apply_sync_library` refuses to materialize a
    // tombstone older than the horizon (that's a separate test), so we seed
    // the genuinely-old trashed row straight into the DB for the prune check.
    repo.create_library("Live").await.unwrap();
    use sea_orm::Set;
    let old_ts = Utc::now() - Duration::days(40);
    library::Entity::insert(library::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        name: Set("OldTrash".into()),
        search_name: Set("oldtrash".into()),
        created_at: Set(old_ts.into()),
        updated_at: Set(old_ts.into()),
        sync_id: Set("old-trash".into()),
        deleted_at: Set(Some(old_ts.into())),
    })
    .exec(&repo.db)
    .await
    .unwrap();
    repo.apply_sync_library(&incoming("recent", "RecentTrash", 0, true))
        .await
        .unwrap();

    let pruned = repo
        .prune_deleted_libraries(Duration::days(30))
        .await
        .unwrap();
    assert_eq!(pruned, 1, "only the >30d tombstone is pruned");
    let names: Vec<String> = repo
        .list_library_sync_manifest()
        .await
        .unwrap()
        .into_iter()
        .map(|l| l.name)
        .collect();
    assert!(names.contains(&"Live".to_string()));
    assert!(names.contains(&"RecentTrash".to_string()));
    assert!(
        !names.contains(&"OldTrash".to_string()),
        "old tombstone pruned"
    );
}
