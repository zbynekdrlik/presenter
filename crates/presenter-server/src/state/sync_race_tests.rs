//! #580 regression tests: the rare concurrent library-delete-vs-
//! presentation-create race. Split into its own file rather than growing
//! `sync_integration_tests.rs` (already near the file-size gate) — mirrors
//! that file's two-peer `AppState` + `run_sync_cycle` harness with a small
//! set of local helper duplicates.
//!
//! Scenario (from the issue): peer A deletes library `L`; peer B,
//! concurrently, creates a NEW live presentation `P2` under the
//! SAME-named `L` before ever seeing A's delete. #578 gave libraries their
//! own LWW sync (`updated_at`/`sync_id`/`deleted_at`, `apply_sync_library`)
//! but `ensure_library` (used when applying a LIVE presentation with no
//! matching LIVE library) ignored a same-named TOMBSTONED library entirely
//! and always minted a fresh live one — diverging the two peers. The fix
//! (decision on #580: option (b)) decides live-vs-tombstoned from the
//! tombstoned library's own `updated_at`, the SAME LWW mechanism already
//! used everywhere else in this sync design.
use std::net::SocketAddr;
use std::time::Duration;

use presenter_core::{LibraryId, PresentationId};
use tokio::time::sleep;

use crate::router::build_router;
use crate::state::sync::run_sync_cycle;
use crate::state::AppState;

async fn serve(state: AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let router = build_router(state);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().build().unwrap()
}

fn find_song(libs: &[presenter_core::Library], name: &str) -> Option<presenter_core::Presentation> {
    libs.iter()
        .flat_map(|l| l.presentations.iter())
        .find(|p| p.name == name)
        .cloned()
}

async fn make_song(state: &AppState, lib: &str, name: &str) -> (LibraryId, PresentationId) {
    let library = state.create_library(lib).await.unwrap();
    let (lib_id, _, pres, _) = state
        .create_presentation(library.id, name, None)
        .await
        .unwrap();
    (lib_id, pres.id)
}

/// How many library rows (LIVE + tombstoned) currently carry this name — the
/// #580 bug signature is a SECOND live row minted beside the existing
/// (tombstoned) one instead of reusing/reviving it.
async fn library_row_count_by_name(state: &AppState, name: &str) -> usize {
    state
        .repository()
        .list_library_sync_manifest()
        .await
        .unwrap()
        .into_iter()
        .filter(|l| l.name == name)
        .count()
}

async fn library_is_deleted(state: &AppState, name: &str) -> Option<bool> {
    state
        .repository()
        .list_library_sync_manifest()
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.name == name)
        .map(|l| l.deleted_at.is_some())
}

#[tokio::test]
async fn concurrent_library_delete_and_presentation_create_keeps_tombstone_when_delete_is_newer() {
    // Branch under test: the library DELETE is the newer event. Decision (b)
    // says L must STAY tombstoned and P2 attach to it (hidden) — both sides
    // converge on the SAME single library row, never a second freshly-minted
    // live one.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    let b_url = serve(b.clone()).await;

    make_song(&a, "Songs", "Existing").await;
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert!(
        find_song(&b.libraries().await.unwrap(), "Existing").is_some(),
        "sanity: B holds the library + song before the race"
    );
    let lib_id_b = b
        .libraries()
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.name == "Songs")
        .expect("B holds the live library after the first sync")
        .id;

    // B creates P2 under the SAME library — the older of the two racing
    // events — never yet synced to A.
    b.create_presentation(lib_id_b, "P2", None).await.unwrap();

    // A deletes the whole library — strictly NEWER than B's create.
    sleep(Duration::from_millis(5)).await;
    let lib_id_a = a
        .libraries()
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.name == "Songs")
        .unwrap()
        .id;
    a.delete_library(lib_id_a).await.unwrap();
    assert!(
        find_song(&a.libraries().await.unwrap(), "Existing").is_none(),
        "sanity: the library is gone on A right after the delete"
    );

    // Sync in both directions, per the issue: B pulls A's tombstone, then A
    // pulls B's still-live P2.
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    run_sync_cycle(&a, &b_url, &client()).await.unwrap();

    assert!(
        find_song(&a.libraries().await.unwrap(), "P2").is_none(),
        "P2 must stay HIDDEN on A — attached to the still-tombstoned library, \
         never surfaced through a resurrected live one"
    );
    assert_eq!(
        library_is_deleted(&a, "Songs").await,
        Some(true),
        "the library delete is the newer event — it must stay tombstoned, never revived"
    );
    assert_eq!(
        library_row_count_by_name(&a, "Songs").await,
        1,
        "#580 bug signature: a fresh live library must never be minted beside \
         the existing tombstoned one for the same name"
    );

    // Converge B too — both sides end up in the SAME state (library hidden,
    // P2 live-but-hidden underneath).
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert_eq!(
        library_row_count_by_name(&b, "Songs").await,
        1,
        "B must not end up with a duplicate library row either"
    );
    assert_eq!(library_is_deleted(&b, "Songs").await, Some(true));
}

#[tokio::test]
async fn concurrent_library_delete_and_presentation_create_revives_library_when_create_is_newer() {
    // Same race, other tie-break — P2's create is the NEWER event. Decision
    // (b): revive L (clear deleted_at, bump updated_at) and attach P2; a
    // presentation tombstoned earlier UNDER L must stay tombstoned — reviving
    // the LIBRARY must never revive presentations under it.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    let b_url = serve(b.clone()).await;

    let (lib_id_a, trashed_id) = make_song(&a, "Songs", "AlreadyTrashed").await;
    a.delete_presentation(trashed_id).await.unwrap();

    run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    let lib_id_b = b
        .libraries()
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.name == "Songs")
        .expect("B holds the live (but song-empty) library after the first sync")
        .id;

    // A deletes the library — the older of the two racing events.
    a.delete_library(lib_id_a).await.unwrap();

    // B creates P2 under the SAME library name — strictly NEWER — never
    // having synced A's delete yet.
    sleep(Duration::from_millis(5)).await;
    b.create_presentation(lib_id_b, "P2", None).await.unwrap();

    // Sync both directions.
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    run_sync_cycle(&a, &b_url, &client()).await.unwrap();

    assert!(
        find_song(&a.libraries().await.unwrap(), "P2").is_some(),
        "P2's create is the newer event — the library must be REVIVED so P2 \
         is visible again"
    );
    assert_eq!(
        library_is_deleted(&a, "Songs").await,
        Some(false),
        "the library must be revived (deleted_at cleared), not left tombstoned"
    );
    assert_eq!(
        library_row_count_by_name(&a, "Songs").await,
        1,
        "#580 bug signature: reviving must reuse the EXISTING library row, \
         never mint a second fresh one"
    );
    assert!(
        find_song(&a.libraries().await.unwrap(), "AlreadyTrashed").is_none(),
        "reviving the LIBRARY must never revive a presentation tombstoned under it"
    );
}
