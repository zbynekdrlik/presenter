//! #555 end-to-end sync proof: two independent AppStates, each on a real ephemeral
//! HTTP port, reconciled via `run_sync_cycle` both ways.
use std::net::SocketAddr;

use presenter_core::{LibraryId, PresentationId};

use crate::router::build_router;
use crate::state::sync::run_sync_cycle;
use crate::state::AppState;

/// Bind the state's real router on 127.0.0.1:0 and return its base URL.
async fn serve(state: AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let router = build_router(state);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

async fn make_song(state: &AppState, lib: &str, name: &str) -> (LibraryId, PresentationId) {
    let library = state.create_library(lib).await.unwrap();
    let (lib_id, _, pres, _) = state
        .create_presentation(library.id, name, None)
        .await
        .unwrap();
    (lib_id, pres.id)
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

async fn edit_first_slide(state: &AppState, id: PresentationId, text: &str) {
    let detail = state.presentation_detail(id).await.unwrap().unwrap().2;
    let slide_id = detail.slides[0].id;
    state
        .update_slide_content(
            id,
            slide_id,
            text.to_string(),
            String::new(),
            String::new(),
            None,
            None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn create_propagates_a_to_b() {
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    make_song(&a, "Songs", "Amazing Grace").await;

    // B pulls from A.
    let (_pulled, applied) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert!(applied >= 1, "B must import the song created on A");

    let libs = b.libraries().await.unwrap();
    assert!(
        find_song(&libs, "Amazing Grace").is_some(),
        "the song created on A appears on B"
    );
}

#[tokio::test]
async fn edit_and_rename_propagate() {
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    let (_lib, id) = make_song(&a, "Songs", "Song").await;
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();

    // Rename + edit on A.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    a.rename_presentation(id, "Renamed").await.unwrap();
    edit_first_slide(&a, id, "new text").await;

    run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    let libs = b.libraries().await.unwrap();
    let p = find_song(&libs, "Renamed").expect("renamed song synced to B");
    assert_eq!(p.slides[0].content.main.value(), "new text");
}

#[tokio::test]
async fn delete_to_trash_and_restore_propagate() {
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    let (_lib, id) = make_song(&a, "Songs", "Temp").await;
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();

    // Delete on A → soft delete → syncs as an edit.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    a.delete_presentation(id).await.unwrap();
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    let libs = b.libraries().await.unwrap();
    assert!(
        find_song(&libs, "Temp").is_none(),
        "B hides the deleted song"
    );
    let trash = b.repository().list_trashed_presentations().await.unwrap();
    assert!(
        trash.iter().any(|t| t.name == "Temp"),
        "B shows it in trash"
    );

    // Restore on A → propagates back to live.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    a.restore_presentation(id).await.unwrap();
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    let libs = b.libraries().await.unwrap();
    assert!(
        find_song(&libs, "Temp").is_some(),
        "the restore propagated: B shows the song live again"
    );
}

#[tokio::test]
async fn pruned_row_is_never_resurrected_by_a_stale_peer_tombstone() {
    // S7 regression: once we've pruned a trashed row (our own 30-day
    // schedule fired), the peer's manifest may still list the tombstone
    // (its OWN prune hasn't fired yet) -- applying that stale tombstone
    // against "we have no row at all" must be a NO-OP, not a re-create.
    // Each side prunes on its own schedule and nothing resurrects.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    let (_lib, id) = make_song(&a, "Songs", "Old Temp").await;
    run_sync_cycle(&b, &a_url, &client()).await.unwrap(); // B learns of it

    // Delete on A, sync the trash to B, then B prunes it immediately
    // (simulating B's own 30-day schedule firing first) while A's manifest
    // still lists the (unpruned) tombstone.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    a.delete_presentation(id).await.unwrap();
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert!(
        !b.repository()
            .list_trashed_presentations()
            .await
            .unwrap()
            .is_empty(),
        "sanity: B holds the trashed row before pruning"
    );
    b.repository()
        .prune_deleted_presentations(chrono::Duration::seconds(-3600))
        .await
        .unwrap();
    assert!(
        b.repository()
            .list_trashed_presentations()
            .await
            .unwrap()
            .is_empty(),
        "sanity: B's prune removed the row entirely"
    );

    // B pulls A again: A's manifest STILL lists the (unpruned) tombstone.
    let (_pulled, applied) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert_eq!(
        applied, 0,
        "a pruned row must never be resurrected by the peer's stale tombstone"
    );
    assert!(
        b.repository()
            .list_trashed_presentations()
            .await
            .unwrap()
            .is_empty(),
        "no trash row must reappear"
    );
    let libs = b.libraries().await.unwrap();
    assert!(
        find_song(&libs, "Old Temp").is_none(),
        "the song must not reappear live either"
    );
}

#[tokio::test]
async fn lww_newer_edit_wins_in_a_two_sided_conflict() {
    // Same song shared by both, then edited on both — the newer edit wins.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    let b_url = serve(b.clone()).await;
    let (_la, ia) = make_song(&a, "Songs", "Conflict").await;
    // B pulls A so both share the sync_id.
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();

    // Edit B first, then A later (A is newer).
    let libs = b.libraries().await.unwrap();
    let bp = find_song(&libs, "Conflict").expect("B holds the song");
    edit_first_slide(&b, bp.id, "B edit").await;
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    edit_first_slide(&a, ia, "A edit (newer)").await;

    // Reconcile both ways; A's newer edit must win everywhere.
    run_sync_cycle(&a, &b_url, &client()).await.unwrap();
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();

    for st in [&a, &b] {
        let libs = st.libraries().await.unwrap();
        let p = find_song(&libs, "Conflict").expect("song present on both");
        assert_eq!(
            p.slides[0].content.main.value(),
            "A edit (newer)",
            "the strictly newer edit wins on both instances"
        );
    }
}

#[tokio::test]
async fn fully_synced_pair_produces_zero_writes_on_next_cycle() {
    // The no-echo guard.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    let b_url = serve(b.clone()).await;
    make_song(&a, "Songs", "Steady").await;
    run_sync_cycle(&b, &a_url, &client()).await.unwrap(); // B imports

    // A pulling B back must not see B's applied copy as "newer" (peer
    // timestamp was preserved verbatim on apply).
    let (pulled_back, applied_back) = run_sync_cycle(&a, &b_url, &client()).await.unwrap();
    assert_eq!(applied_back, 0, "A must not re-apply its own change (echo)");
    assert_eq!(pulled_back, 0, "A must not even re-pull it");

    // Next cycle B→A: nothing newer on A → zero pulled/applied.
    let (pulled, applied) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert_eq!(applied, 0, "a settled pair must not re-apply (no echo)");
    assert_eq!(pulled, 0, "a settled pair must not re-pull");
}

#[tokio::test]
async fn adopt_by_name_pairs_independently_created_copies() {
    // Both instances hold "the same song" created independently (different
    // sync_ids) — the transitional case. Sync must MERGE them, not duplicate.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    let (_la, ia) = make_song(&a, "Songs", "Twin").await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let (_lb, ib) = make_song(&b, "Songs", "Twin").await; // B's copy is newer

    // A pulls B: B's copy is newer → A adopts B's identity, still ONE song.
    let b_url = serve(b.clone()).await;
    run_sync_cycle(&a, &b_url, &client()).await.unwrap();
    let libs = a.libraries().await.unwrap();
    let twins: Vec<_> = libs
        .iter()
        .flat_map(|l| l.presentations.iter())
        .filter(|p| p.name == "Twin")
        .collect();
    assert_eq!(twins.len(), 1, "adopt-by-name must not duplicate the song");

    // And B pulling A now finds nothing newer (identities converged).
    let (_p, applied) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert_eq!(applied, 0, "converged identities produce no further writes");

    let _ = (ia, ib);
}

/// Poll `sync_status_snapshot` until `last_run` differs from `after`, or panic
/// once `timeout` elapses. A bounded retry loop, not an arbitrary sleep.
async fn wait_for_next_cycle(
    state: &AppState,
    after: Option<chrono::DateTime<chrono::Utc>>,
    timeout: std::time::Duration,
) -> crate::state::sync::SyncStatus {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let status = state.sync_status_snapshot().await;
        if status.last_run.is_some() && status.last_run != after {
            return status;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "the real sync loop did not complete a cycle within {timeout:?} \
                 (last_run={:?}) - it likely shut down immediately after spawn",
                status.last_run
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn real_sync_loop_survives_past_startup_and_completes_multiple_cycles() {
    // S1 regression: `maybe_spawn_sync` used to drop the loop's shutdown
    // oneshot Sender right after spawning it. Dropping the Sender makes
    // `tokio::select! { _ = &mut shutdown_rx => break, ... }` resolve its
    // shutdown branch essentially immediately - the loop died before ever
    // running a real cycle in production. Every other sync test in this file
    // calls `run_sync_cycle` directly, bypassing the loop/spawn path
    // entirely, so CI never caught it. This test drives the REAL spawned
    // loop via `maybe_spawn_sync` + the nudge channel and requires it to
    // survive across TWO separate triggered cycles - a loop that dies right
    // after (or even during) its first lucky cycle fails the second wait.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    make_song(&a, "Songs", "Loop Song").await;

    b.maybe_spawn_sync(Some(a_url));

    b.nudge_sync();
    let first = wait_for_next_cycle(&b, None, std::time::Duration::from_secs(10)).await;
    assert!(
        first.last_success.is_some(),
        "the real loop's first cycle must succeed: {first:?}"
    );

    // A second, independent nudge must ALSO complete a cycle - proving the
    // loop is still alive, not that it happened to survive one lucky race.
    b.nudge_sync();
    let second = wait_for_next_cycle(&b, first.last_run, std::time::Duration::from_secs(10)).await;
    assert!(
        second.last_success.is_some(),
        "the real loop's second cycle must also succeed: {second:?}"
    );

    let libs = b.libraries().await.unwrap();
    assert!(
        find_song(&libs, "Loop Song").is_some(),
        "the real (spawned) loop actually pulled and applied the peer's song"
    );
}
