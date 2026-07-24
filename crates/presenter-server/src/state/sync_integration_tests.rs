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
    let (_pulled, applied, _errors) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
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
async fn a_fresh_peer_creates_a_never_held_recent_tombstone_as_trashed() {
    // R2 regression: `sync_should_apply`'s old `None => !peer_deleted` rule
    // treated EVERY tombstone the same when we hold no local row at all --
    // so a fresh/re-provisioned peer's FIRST sync permanently skipped
    // anything already trashed on the peer, and the two instances' trash
    // contents diverged forever. Fix: a RECENT tombstone (younger than the
    // 30-day prune horizon) for a row we've never held must be applied --
    // created locally IN TRASH, so trash contents converge. (The companion
    // test below covers the other half: a tombstone OLDER than the
    // horizon, which is what S7 actually protects against.)
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    let (_lib, id) = make_song(&a, "Songs", "Already Gone").await;
    a.delete_presentation(id).await.unwrap(); // trashed BEFORE B ever learns of it

    let (_pulled, applied, _errors) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert_eq!(
        applied, 1,
        "a fresh peer must create a recently-trashed row it never held, in trash state"
    );
    let trash = b.repository().list_trashed_presentations().await.unwrap();
    assert!(
        trash.iter().any(|t| t.name == "Already Gone"),
        "the row lands in B's trash, not silently dropped"
    );
    let libs = b.libraries().await.unwrap();
    assert!(
        find_song(&libs, "Already Gone").is_none(),
        "it must never appear live"
    );
}

#[tokio::test]
async fn a_tombstone_older_than_the_prune_horizon_is_never_resurrected_for_a_row_never_held() {
    // S7 regression, UPDATED for R2: `sync_should_apply`'s `local: None`
    // branch can no longer unconditionally skip every tombstone (see the
    // companion test above), so the "never resurrect an already-pruned
    // row" guarantee now rests entirely on the tombstone's AGE against the
    // 30-day prune horizon -- a genuinely pruned row's tombstone predates
    // it (real pruning never touches a fresher row). Planted directly via
    // `apply_sync_presentation`, which stores a peer's timestamp verbatim
    // (never echo) -- exactly like a real peer's `/sync` response would,
    // without needing to fast-forward 31 real days.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;

    let ancient = chrono::Utc::now() - chrono::Duration::days(31);
    let ancient_tombstone = presenter_persistence::SyncPresentation {
        sync_id: "sid-ancient-tombstone".to_string(),
        library_name: "Songs".to_string(),
        name: "Old Temp".to_string(),
        updated_at: ancient,
        deleted_at: Some(ancient),
        slides: Vec::new(),
    };
    a.repository()
        .apply_sync_presentation(&ancient_tombstone, &std::collections::HashSet::new())
        .await
        .unwrap();

    // B has NEVER held this row at all.
    let (_pulled, applied, _errors) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert_eq!(
        applied, 0,
        "a tombstone older than the prune horizon must never be resurrected"
    );
    assert!(
        b.repository()
            .list_trashed_presentations()
            .await
            .unwrap()
            .is_empty(),
        "no trash row must appear"
    );
    let libs = b.libraries().await.unwrap();
    assert!(
        find_song(&libs, "Old Temp").is_none(),
        "the song must not appear live either"
    );
}

#[tokio::test]
async fn a_single_broken_song_never_aborts_the_whole_sync_cycle() {
    // #558 round-4 U1(b): `run_sync_cycle` used `?` on the per-song fetch
    // (send/error_for_status/json), so ONE song's fetch failing (here: a
    // corrupted stored slide id that can NEVER parse as a UUID -- a
    // failure independent of U1(a)'s oversize-text fix, isolating THIS
    // behavior specifically) aborted the WHOLE cycle via
    // `anyhow::Result`'s early return. Every OTHER song in that cycle's
    // manifest -- whatever order it happened to come in -- silently never
    // synced, forever (retried every 30s, but the same song keeps failing
    // first). FIX: isolate each song's fetch/apply -- log + count the
    // failure and continue to the next manifest entry.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;

    let library = a.create_library("Songs").await.unwrap();
    let (_, _, broken_pres, _) = a
        .create_presentation(library.id, "Broken Song", None)
        .await
        .unwrap();
    let (_, _, _good_pres, _) = a
        .create_presentation(library.id, "Good Song", None)
        .await
        .unwrap();
    let broken_id = broken_pres.id;

    let broken_detail = a.presentation_detail(broken_id).await.unwrap().unwrap().2;
    let broken_slide_id = broken_detail.slides[0].id.to_string();
    use presenter_persistence::entities::slide as slide_entity;
    use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter};
    slide_entity::Entity::update_many()
        .col_expr(slide_entity::Column::Id, Expr::value("not-a-uuid"))
        .filter(slide_entity::Column::Id.eq(broken_slide_id))
        .exec(a.repository().connection())
        .await
        .unwrap();

    let (pulled, applied, errors) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert_eq!(pulled, 2, "both manifest entries were attempted");
    assert_eq!(
        applied, 1,
        "the good song still applies despite the broken one"
    );
    assert_eq!(
        errors, 1,
        "the broken song's failure is counted, not silently swallowed"
    );

    let libs = b.libraries().await.unwrap();
    assert!(
        find_song(&libs, "Good Song").is_some(),
        "the good song must sync even though another manifest entry 500s"
    );
    assert!(
        find_song(&libs, "Broken Song").is_none(),
        "the broken song's row never applies (its fetch permanently fails)"
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
    let (pulled_back, applied_back, _errors) = run_sync_cycle(&a, &b_url, &client()).await.unwrap();
    assert_eq!(applied_back, 0, "A must not re-apply its own change (echo)");
    assert_eq!(pulled_back, 0, "A must not even re-pull it");

    // Next cycle B→A: nothing newer on A → zero pulled/applied.
    let (pulled, applied, _errors) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
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
    let (_p, applied, _errors) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert_eq!(applied, 0, "converged identities produce no further writes");

    let _ = (ia, ib);
}

#[tokio::test]
async fn two_live_same_name_twins_on_one_peer_both_converge_on_the_other() {
    // #558 round-4 U2: two LIVE same-name songs on A used to serially
    // adopt onto ONE row on B within a single cycle — whichever twin was
    // processed first got CREATED, then the second twin's adopt-by-name
    // step found that just-created row as its sole live candidate and
    // overwrote its sync_id + content, silently discarding the first
    // twin. The orphaned first identity then refetched and re-adopted
    // every following cycle forever, ping-ponging content back and forth
    // while B was only ever able to hold ONE of the two twins at a time.
    // FIX: adopt-by-name is single-shot per name — a local candidate whose
    // OWN sync_id is itself present in the peer's manifest is never
    // adopted (the peer already tracks it under its own identity); the
    // second twin creates its own new row instead, so both converge in
    // one cycle.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;

    let library = a.create_library("Songs").await.unwrap();
    let (_, _, p1, _) = a
        .create_presentation(library.id, "Twin Pair", None)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let (_, _, p2, _) = a
        .create_presentation(library.id, "Twin Pair", None)
        .await
        .unwrap();
    let (i1, i2) = (p1.id, p2.id);

    run_sync_cycle(&b, &a_url, &client()).await.unwrap();

    let libs = b.libraries().await.unwrap();
    let twins: Vec<_> = libs
        .iter()
        .flat_map(|l| l.presentations.iter())
        .filter(|p| p.name == "Twin Pair")
        .collect();
    assert_eq!(
        twins.len(),
        2,
        "both same-name twins must exist on B after one cycle, not just one"
    );

    // Converged identities: a second cycle must be a total no-op — the old
    // bug kept refetching and re-adopting both twins forever.
    let (pulled, applied, _errors) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert_eq!(
        pulled, 0,
        "converged twins must never be refetched every cycle"
    );
    assert_eq!(
        applied, 0,
        "converged twins must never be re-applied every cycle"
    );

    let _ = (i1, i2);
}

#[tokio::test]
async fn trashing_one_of_two_independently_created_same_name_songs_never_cross_contaminates() {
    // #558 round-3 Decision B / T1, integration-matrix scenario. Unlike
    // `adopt_by_name_pairs_independently_created_copies` above (where BOTH
    // sides are still LIVE when they first sync, and adopt-by-name
    // deliberately MERGES them into one song — that behavior is
    // unchanged), this covers a peer TOMBSTONE with an unknown sync_id: A
    // trashes its own "Shared Name" song BEFORE B ever learns it existed.
    // Before Decision B, that tombstone could reach across and adopt (and
    // thereby trash) B's completely unrelated, live, same-named song.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    let b_url = serve(b.clone()).await;

    let (_la, ia) = make_song(&a, "Songs", "Shared Name").await;
    edit_first_slide(&a, ia, "A's content").await;
    let (_lb, ib) = make_song(&b, "Songs", "Shared Name").await;
    edit_first_slide(&b, ib, "B's content").await;

    // A trashes its OWN copy before any sync cycle ever runs — B never
    // learns A's song existed live at all, only as a tombstone.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    a.delete_presentation(ia).await.unwrap();

    // B pulls A: an unknown-sync_id TOMBSTONE arrives while B holds a LIVE
    // same-named song. It must create its OWN new trashed row, never
    // adopt B's live song by name.
    run_sync_cycle(&b, &a_url, &client()).await.unwrap();

    let b_libs = b.libraries().await.unwrap();
    let b_own = find_song(&b_libs, "Shared Name").expect("B's own song stays live");
    let b_own_detail = b.presentation_detail(b_own.id).await.unwrap().unwrap().2;
    assert_eq!(
        b_own_detail.slides[0].content.main.value(),
        "B's content",
        "B's own live song is completely untouched by A's tombstone"
    );
    let b_trash = b.repository().list_trashed_presentations().await.unwrap();
    assert_eq!(
        b_trash.len(),
        1,
        "A's tombstone landed as its OWN separate trashed row, not merged onto B's song"
    );
    assert_eq!(b_trash[0].name, "Shared Name");

    // A pulls B: A's own local candidate is trashed (not live), so B's
    // live song has no live candidate to adopt-by-name onto either — it
    // is created as a brand-new live row.
    run_sync_cycle(&a, &b_url, &client()).await.unwrap();
    let a_libs = a.libraries().await.unwrap();
    let a_songs: Vec<_> = a_libs
        .iter()
        .flat_map(|l| l.presentations.iter())
        .filter(|p| p.name == "Shared Name")
        .collect();
    assert_eq!(
        a_songs.len(),
        1,
        "A now holds exactly ONE live 'Shared Name' — B's copy (A's own is trashed)"
    );
    assert_eq!(a_songs[0].slides[0].content.main.value(), "B's content");
    let a_trash = a.repository().list_trashed_presentations().await.unwrap();
    assert_eq!(a_trash.len(), 1, "A's own copy is still trashed, untouched");

    // Restoring A's copy on B brings back A's content under A's sync_id,
    // WITHOUT touching B's own song.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let restore_id =
        presenter_core::PresentationId::from_uuid(uuid::Uuid::parse_str(&b_trash[0].id).unwrap());
    b.restore_presentation(restore_id).await.unwrap();

    let b_libs_after_restore = b.libraries().await.unwrap();
    let b_songs_after_restore: Vec<_> = b_libs_after_restore
        .iter()
        .flat_map(|l| l.presentations.iter())
        .filter(|p| p.name == "Shared Name")
        .collect();
    assert_eq!(
        b_songs_after_restore.len(),
        2,
        "B now holds BOTH songs live: its own, plus the restored A copy"
    );
    let restored_content: Vec<_> = b_songs_after_restore
        .iter()
        .map(|p| p.slides[0].content.main.value())
        .collect();
    assert!(
        restored_content.contains(&"A's content"),
        "the restored song carries A's original content"
    );
    assert!(
        restored_content.contains(&"B's content"),
        "B's own song is untouched by the restore"
    );
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

    b.nudge_sync().await;
    let first = wait_for_next_cycle(&b, None, std::time::Duration::from_secs(10)).await;
    assert!(
        first.last_success.is_some(),
        "the real loop's first cycle must succeed: {first:?}"
    );

    // A second, independent nudge must ALSO complete a cycle - proving the
    // loop is still alive, not that it happened to survive one lucky race.
    b.nudge_sync().await;
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

#[tokio::test]
async fn two_spawns_on_one_app_state_leave_exactly_one_live_loop() {
    // R5 regression: `spawn_sync_task` overwrote the coordinator's shutdown
    // slot with the NEW sender BEFORE checking whether a loop was already
    // running. Overwriting it drops the OLD sender, which resolves the
    // running loop's `shutdown_rx` and kills it -- while the NEW task then
    // finds `nudge_rx` already taken (by the first loop) and refuses to
    // start. Net effect of a second spawn call: ZERO live loops. Fix: check
    // whether a loop is already running BEFORE storing anything; a second
    // spawn is then a no-op that leaves the FIRST loop untouched.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;
    make_song(&a, "Songs", "Loop Song").await;

    b.maybe_spawn_sync(Some(a_url.clone()));
    b.maybe_spawn_sync(Some(a_url)); // a second spawn call on the SAME AppState

    b.nudge_sync().await;
    let first = wait_for_next_cycle(&b, None, std::time::Duration::from_secs(10)).await;
    assert!(
        first.last_success.is_some(),
        "a live loop must survive two spawn calls: {first:?}"
    );

    // A second, independent nudge must ALSO complete a cycle - proving the
    // surviving loop is still alive, not that it happened to run once.
    b.nudge_sync().await;
    let second = wait_for_next_cycle(&b, first.last_run, std::time::Duration::from_secs(10)).await;
    assert!(
        second.last_success.is_some(),
        "the surviving loop must keep completing cycles: {second:?}"
    );

    let libs = b.libraries().await.unwrap();
    assert!(
        find_song(&libs, "Loop Song").is_some(),
        "the surviving loop actually pulled and applied the peer's song"
    );
}

#[tokio::test]
async fn an_all_fail_cycle_reports_unhealthy_instead_of_success() {
    // #558 V6: per-song isolation (#558 round-4 U1(b)) means ONE bad song
    // never aborts the cycle — but that isolation let a cycle where EVERY
    // fetched entry failed fall through to the `Ok` branch and
    // unconditionally refresh `last_success` / clear `last_error`, hiding a
    // 100%-failed cycle behind a healthy-looking status. Corrupts BOTH
    // songs' stored slide ids (same technique as
    // `a_single_broken_song_never_aborts_the_whole_sync_cycle` — a
    // corrupted id can never parse as a UUID, so every per-song content
    // fetch 500s) so the cycle is genuinely all-fail, then drives the REAL
    // spawned loop (not `run_sync_cycle` directly) so the status this
    // asserts on is exactly what the operator-facing status endpoint would
    // show.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;

    let library = a.create_library("Songs").await.unwrap();
    for name in ["Broken One", "Broken Two"] {
        let (_, _, pres, _) = a.create_presentation(library.id, name, None).await.unwrap();
        let detail = a.presentation_detail(pres.id).await.unwrap().unwrap().2;
        let slide_id = detail.slides[0].id.to_string();
        use presenter_persistence::entities::slide as slide_entity;
        use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter};
        slide_entity::Entity::update_many()
            .col_expr(
                slide_entity::Column::Id,
                Expr::value(format!("not-a-uuid-{name}")),
            )
            .filter(slide_entity::Column::Id.eq(slide_id))
            .exec(a.repository().connection())
            .await
            .unwrap();
    }

    b.maybe_spawn_sync(Some(a_url));
    b.nudge_sync().await;
    let status = wait_for_next_cycle(&b, None, std::time::Duration::from_secs(10)).await;

    assert!(
        status.last_error.is_some(),
        "an all-fail cycle must surface last_error, not report itself healthy: {status:?}"
    );
    assert!(
        status.last_success.is_none(),
        "an all-fail cycle must never refresh last_success: {status:?}"
    );
    assert_eq!(
        status.last_cycle_errors, 2,
        "both fetch failures must be counted: {status:?}"
    );
}

/// An ad-hoc peer serving one manifest entry (`sync_id`/`name` in library
/// "Songs") whose CONTENT fetch blocks on a `Notify` gate until released
/// (#558 W5). Returns `(peer_url, entered, gate)`: `entered` fires the
/// MOMENT the content-fetch handler starts running (proving the request is
/// genuinely in flight server-side, never a timing guess); `gate` holds the
/// handler there until the caller calls `gate.notify_one()`.
async fn spawn_gated_peer(
    sync_id: String,
    name: &str,
) -> (
    String,
    std::sync::Arc<tokio::sync::Notify>,
    std::sync::Arc<tokio::sync::Notify>,
) {
    use std::sync::Arc;
    use tokio::sync::Notify;

    let now = chrono::Utc::now();
    let manifest_body = serde_json::json!([{
        "syncId": sync_id, "libraryName": "Songs", "name": name,
        "updatedAt": now, "deletedAt": null,
    }]);
    let content_body = serde_json::json!({
        "syncId": sync_id, "libraryName": "Songs", "name": name,
        "updatedAt": now, "deletedAt": null, "slides": [],
    });

    let entered = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let entered_for_handler = entered.clone();
    let gate_for_handler = gate.clone();
    let app = axum::Router::new()
        .route(
            "/sync/manifest",
            axum::routing::get(move || {
                let body = manifest_body.clone();
                async move { axum::Json(body) }
            }),
        )
        .route(
            "/sync/presentations/{sync_id}",
            axum::routing::get(
                move |axum::extract::Path(_id): axum::extract::Path<String>| {
                    let entered = entered_for_handler.clone();
                    let gate = gate_for_handler.clone();
                    let body = content_body.clone();
                    async move {
                        entered.notify_one();
                        gate.notified().await;
                        axum::Json(body)
                    }
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), entered, gate)
}

#[tokio::test]
async fn an_edit_op_lock_is_not_held_while_the_peer_content_fetch_is_in_flight() {
    // #558 W5: `fetch_and_apply_one` used to acquire the shared
    // per-presentation lock BEFORE the peer's (up to 15s) HTTP content
    // fetch — a concurrent edit op on the SAME presentation blocked for the
    // whole request. FIX: fetch first, lock only around the apply. The
    // gated peer (`spawn_gated_peer`) holds the content request open until
    // this test explicitly releases it (never an arbitrary sleep — fully
    // sequenced), while a non-blocking `try_lock` probe proves the lock is
    // free the ENTIRE time the fetch is in flight.
    let state = AppState::in_memory().await.unwrap();
    let (_, local_id) = make_song(&state, "Songs", "Slow Peer Song").await;

    use presenter_persistence::entities::presentation as presentation_entity;
    use sea_orm::EntityTrait;
    let row = presentation_entity::Entity::find_by_id(local_id.to_string())
        .one(state.repository().connection())
        .await
        .unwrap()
        .expect("local row exists");

    let (peer_url, entered, gate) = spawn_gated_peer(row.sync_id, "Slow Peer Song").await;

    let state_for_cycle = state.clone();
    let cycle =
        tokio::spawn(async move { run_sync_cycle(&state_for_cycle, &peer_url, &client()).await });

    // Wait until the content-fetch handler has genuinely been entered
    // (in flight) BEFORE probing the lock — a sequenced hook, never a
    // timing guess.
    entered.notified().await;
    assert!(
        state.presentation_lock_try_acquire_for_test(local_id),
        "the per-presentation lock must be FREE while the peer content fetch is in \
         flight (#558 W5) — the old code acquired it BEFORE the fetch, so this would \
         have failed on the pre-fix implementation"
    );

    gate.notify_one();
    let (_pulled, applied, _errors) = cycle.await.unwrap().unwrap();
    assert_eq!(
        applied, 1,
        "the sync cycle must still complete normally once the fetch is released"
    );
}

#[tokio::test]
async fn sync_apply_invalidates_the_ableset_cache_on_the_pulling_side() {
    // #575 acceptance: a song arriving via PP<->SNV sync must become
    // AbleSet-resolvable on the pulling side WITHOUT a settings re-save.
    // Before the fix, `run_sync_cycle`'s post-apply `drop_presentation_caches`
    // only cleared the per-id presentation cache — the separate AbleSet
    // prefix->id cache never noticed a sync-applied change at all.
    //
    // The cache MUST be built NON-EMPTY before the sync cycle: an empty
    // cache is already force-rebuilt on every resolve regardless of this
    // fix (`ensure_ableset_cache`'s `cache.entries.is_empty()` escape
    // hatch), which would make an empty-cache test pass even without the
    // fix — a false negative.
    let a = AppState::in_memory().await.unwrap();
    let b = AppState::in_memory().await.unwrap();
    let a_url = serve(a.clone()).await;

    // B already tracks a library named "Songs" via AbleSet and has an
    // EXISTING song resolved into a non-empty cache.
    let (_lib_id, existing_id) = make_song(&b, "Songs", "099 Existing").await;
    let draft = presenter_core::AbleSetSettingsDraft {
        enabled: true,
        host: "ableset.invalid".to_string(),
        osc_port: 39051,
        http_port: 80,
        library_name: "Songs".to_string(),
        song_prefix_length: 3,
    };
    b.update_ableset_settings(
        draft,
        presenter_persistence::SettingsAuditSource::HttpSetter,
        "test",
    )
    .await
    .unwrap();
    assert_eq!(
        b.resolve_ableset_presentation("099").await.unwrap(),
        Some(existing_id),
        "cache must be non-empty before the sync cycle — this is what proves the fix"
    );

    // A NEW song is created on A, in a library also named "Songs" (a
    // separate row from B's — sync matches library by name).
    make_song(&a, "Songs", "001 Amazing Grace").await;

    // B pulls it via a real sync cycle — no AbleSet settings touched on B.
    let (_pulled, applied, _errors) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
    assert_eq!(applied, 1, "B must pull the new song from A");

    let resolved = b
        .resolve_ableset_presentation("001")
        .await
        .expect("resolve must not error");
    assert!(
        resolved.is_some(),
        "a song that just arrived via sync must be AbleSet-resolvable without a \
         settings re-save"
    );
    let libs = b.libraries().await.unwrap();
    let synced = find_song(&libs, "001 Amazing Grace").expect("synced song exists on B");
    assert_eq!(
        resolved,
        Some(synced.id),
        "resolved id must be the newly synced-in presentation"
    );
}
