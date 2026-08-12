//! #647 integration-level regression tests: the sync wire protocol now
//! carries `library_sync_id` alongside `library_name`, and the apply path
//! (`apply_sync_presentation`, `resolve_sync_apply_target`,
//! `ensure_library`, `ensure_library_for_tombstone` — all in
//! `sync_apply_library.rs`) must join by that STABLE identity, falling back
//! to the pre-#647 name-only join only when the identity hasn't converged
//! locally (an old peer, or transient library-manifest fetch failure).
//! Kept in its own sibling test file (test files are exempt from the
//! file-size gate, but the SPLIT itself mirrors `sync_apply_review_tests.rs`
//! / `library_sync_tests.rs` — one file per cohesive regression story).
use super::sync_test_support::{peer_song, peer_song_with_library_sync_id, repo, row, slide};
use crate::entities::library;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::collections::HashSet;

/// The local library row's own `sync_id` — a direct DB read, mirroring how
/// `sync_apply_review_tests.rs` reads `updated_at` off a freshly created
/// library row.
async fn library_sync_id_of(
    repo: &crate::Repository,
    library_id: presenter_core::LibraryId,
) -> String {
    library::Entity::find_by_id(library_id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("library row exists")
        .sync_id
}

/// #647 headline regression: a presentation ALREADY synced once (its own
/// sync_id is known locally) must, on a later update, join its library by
/// IDENTITY — never by the CURRENT `library_name` string it happens to
/// carry, which can now belong to a completely different, unrelated local
/// library (a rename in flight, or a #636 disambiguated collision name).
///
/// Before the fix: name-only resolution would reattach the presentation to
/// `unrelated` (mis-filing). After the fix: it stays under `renamed`,
/// because `library_sync_id` resolves to it directly, regardless of the
/// stale name in the incoming DTO.
#[tokio::test]
async fn presentation_update_joins_by_library_identity_never_a_stale_current_name() {
    let repo = repo().await;

    // `renamed`: the song's TRUE library. Created under "Old Name", then
    // renamed locally to "New Name" (simulating a rename that has already
    // converged via `apply_sync_library`, but whose OLD name the peer's
    // presentation manifest still remembers).
    let renamed = repo.create_library("Old Name").await.unwrap();
    let renamed_sync_id = library_sync_id_of(&repo, renamed.id).await;
    library::Entity::update_many()
        .col_expr(
            library::Column::Name,
            sea_orm::sea_query::Expr::value("New Name"),
        )
        .filter(library::Column::Id.eq(renamed.id.to_string()))
        .exec(&repo.db)
        .await
        .unwrap();

    // `unrelated`: a totally different library that now happens to own the
    // name "Old Name" -- exactly the mis-filing trap (some other library
    // currently bears the name this presentation's stale DTO still quotes).
    repo.create_library("Old Name").await.unwrap();

    // First sync: creates the presentation under `renamed` (fresh identity,
    // no adoption/ambiguity involved) -- must carry `renamed`'s
    // `library_sync_id` even at this first-ever sync, exactly like the
    // sibling `brand_new_presentation_is_created_under_the_correct_library_by_identity`
    // test: a brand-new SONG can still arrive with an already-converged
    // library identity (the library itself synced in an earlier cycle).
    // `peer_song` (library_name hardcoded "Songs", library_sync_id always
    // `None`) matches NEITHER `renamed` NOR `unrelated` -- using it here was
    // a fixture bug: it silently minted a brand-new third library named
    // "Songs" and attached the presentation there, which is exactly why the
    // very next sanity assertion failed (`library_id` was a fresh "Songs"
    // library, not `renamed.id`) without exercising the identity-vs-stale-
    // name behavior this test exists to prove at all.
    let initial = peer_song_with_library_sync_id(
        "peer-rename-trap",
        &renamed_sync_id,
        "Old Name",
        "Rename Trap Song",
        "verse",
        30,
    );
    let (outcome, id) = repo
        .apply_sync_presentation(&initial, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_id = id.expect("created presentation has an id");
    assert_eq!(
        row(&repo, pres_id).await.library_id,
        renamed.id.to_string(),
        "sanity: created under the true (identity-resolved) library"
    );

    // A later update from the peer: the song content changed, `library_name`
    // is STALE ("Old Name" -- what the peer still remembers), but
    // `library_sync_id` correctly names `renamed`'s identity.
    let stale_name_update = peer_song_with_library_sync_id(
        "peer-rename-trap",
        &renamed_sync_id,
        "Old Name",
        "Rename Trap Song",
        "verse two",
        0,
    );
    let (outcome, _) = repo
        .apply_sync_presentation(&stale_name_update, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Updated);

    let pres_row = row(&repo, pres_id).await;
    assert_eq!(
        pres_row.library_id,
        renamed.id.to_string(),
        "must stay under the TRUE library (resolved by identity), never \
         mis-file onto the unrelated library that now owns the stale name"
    );
}

/// #647: the SAME identity-first resolution for a brand-new presentation
/// (never synced before -- goes through `apply_unknown_sync_id` →
/// `ensure_library`, not the step-1 update path above). Also proves no
/// PHANTOM library gets manufactured under the stale name.
#[tokio::test]
async fn brand_new_presentation_is_created_under_the_correct_library_by_identity() {
    let repo = repo().await;

    let renamed = repo.create_library("Old Name").await.unwrap();
    let renamed_sync_id = library_sync_id_of(&repo, renamed.id).await;
    library::Entity::update_many()
        .col_expr(
            library::Column::Name,
            sea_orm::sea_query::Expr::value("New Name"),
        )
        .filter(library::Column::Id.eq(renamed.id.to_string()))
        .exec(&repo.db)
        .await
        .unwrap();
    repo.create_library("Old Name").await.unwrap(); // the unrelated collider

    let libraries_before = library::Entity::find().all(&repo.db).await.unwrap().len();

    let incoming = peer_song_with_library_sync_id(
        "peer-brand-new",
        &renamed_sync_id,
        "Old Name",
        "Brand New Song",
        "verse",
        0,
    );
    let (outcome, id) = repo
        .apply_sync_presentation(&incoming, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_row = row(&repo, id.expect("created")).await;

    assert_eq!(
        pres_row.library_id,
        renamed.id.to_string(),
        "a brand-new presentation must attach to the identity-resolved \
         library, never the unrelated one owning the stale name"
    );
    let libraries_after = library::Entity::find().all(&repo.db).await.unwrap().len();
    assert_eq!(
        libraries_before, libraries_after,
        "no phantom library must be manufactured when the identity resolves"
    );
}

/// #647 compat window: an OLD peer never sends `library_sync_id`
/// (`peer_song`'s default is `None`) — resolution must degrade to EXACTLY
/// the pre-#647 name-only join, unchanged.
#[tokio::test]
async fn an_old_peer_presentation_with_no_library_sync_id_still_joins_by_name() {
    let repo = repo().await;
    let songs = repo.create_library("Songs").await.unwrap();

    let incoming = peer_song("peer-old-style", "Old Style Song", "verse", 10);
    assert!(
        incoming.library_sync_id.is_none(),
        "sanity: peer_song simulates an old, name-only peer"
    );
    let (outcome, id) = repo
        .apply_sync_presentation(&incoming, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_row = row(&repo, id.expect("created")).await;
    assert_eq!(
        pres_row.library_id,
        songs.id.to_string(),
        "an old, name-only peer must still resolve by name exactly as before #647"
    );
}

/// #647: identity resolution must thread through the SAME LWW
/// revive-or-stay-tombstoned decision `ensure_library` has always made — a
/// LIVE incoming presentation whose identity-resolved library is currently
/// tombstoned, and whose own clock is OLDER than the library's tombstone
/// clock, must be written as tombstoned too (never live under a dead
/// parent), exactly like the pre-#647 name-based path (#634/#646).
#[tokio::test]
async fn a_live_presentation_resolved_by_identity_onto_a_tombstoned_library_is_forced_tombstoned() {
    let repo = repo().await;
    let lib = repo.create_library("Choir").await.unwrap();
    let lib_sync_id = library_sync_id_of(&repo, lib.id).await;
    repo.delete_library(lib.id).await.unwrap();
    let lib_row = library::Entity::find_by_id(lib.id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("library row still exists, tombstoned");
    let library_tombstone_at: chrono::DateTime<chrono::Utc> = lib_row.updated_at.into();

    let older_than_tombstone = library_tombstone_at - chrono::Duration::minutes(10);
    let incoming = crate::SyncPresentation {
        sync_id: "peer-forced-by-identity".to_string(),
        library_name: "Choir".to_string(),
        library_sync_id: Some(lib_sync_id),
        name: "Forced Song".to_string(),
        updated_at: older_than_tombstone,
        deleted_at: None,
        slides: vec![slide(0, "verse")],
    };
    let (outcome, id) = repo
        .apply_sync_presentation(&incoming, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_row = row(&repo, id.expect("created")).await;
    assert_eq!(
        pres_row.library_id,
        lib.id.to_string(),
        "attaches to the identity-resolved library even though it's tombstoned"
    );
    assert!(
        pres_row.deleted_at.is_some(),
        "must be written tombstoned too -- never live under a dead parent (#634/#646)"
    );
}

/// #647: `ensure_library_for_tombstone`'s identity-first resolution — a
/// brand-new TOMBSTONE from the peer (song never seen before, arrives
/// already deleted) attaches to the correct library by identity, even
/// though the DTO's `library_name` is stale.
#[tokio::test]
async fn a_brand_new_tombstone_attaches_to_the_correct_library_by_identity() {
    let repo = repo().await;
    let renamed = repo.create_library("Old Name").await.unwrap();
    let renamed_sync_id = library_sync_id_of(&repo, renamed.id).await;
    library::Entity::update_many()
        .col_expr(
            library::Column::Name,
            sea_orm::sea_query::Expr::value("New Name"),
        )
        .filter(library::Column::Id.eq(renamed.id.to_string()))
        .exec(&repo.db)
        .await
        .unwrap();
    repo.create_library("Old Name").await.unwrap(); // unrelated collider

    let now = chrono::Utc::now();
    let incoming = crate::SyncPresentation {
        sync_id: "peer-fresh-tombstone".to_string(),
        library_name: "Old Name".to_string(),
        library_sync_id: Some(renamed_sync_id),
        name: "Already Gone Song".to_string(),
        updated_at: now,
        deleted_at: Some(now),
        slides: Vec::new(),
    };
    let (outcome, id) = repo
        .apply_sync_presentation(&incoming, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(outcome, crate::SyncApplyOutcome::Created);
    let pres_row = row(&repo, id.expect("created")).await;
    assert_eq!(
        pres_row.library_id,
        renamed.id.to_string(),
        "a fresh tombstone must attach to the identity-resolved library, \
         never the unrelated one currently owning the stale name"
    );
}

/// #647: `resolve_sync_apply_target` (the pre-transaction lock-target probe
/// `state/sync.rs` uses) must scope its adopt-by-name candidate search to
/// the IDENTITY-resolved library too, not a same-named unrelated one — it
/// shares `find_library_id` with the real apply, so a divergence here would
/// mean it locks (or fails to lock) the WRONG row ahead of the real apply.
#[tokio::test]
async fn resolve_sync_apply_target_scopes_by_identity_not_a_stale_name() {
    let repo = repo().await;

    let renamed = repo.create_library("Old Name").await.unwrap();
    let renamed_sync_id = library_sync_id_of(&repo, renamed.id).await;
    library::Entity::update_many()
        .col_expr(
            library::Column::Name,
            sea_orm::sea_query::Expr::value("New Name"),
        )
        .filter(library::Column::Id.eq(renamed.id.to_string()))
        .exec(&repo.db)
        .await
        .unwrap();
    let unrelated = repo.create_library("Old Name").await.unwrap();

    // An existing LIVE local song under the TRUE library, same name as the
    // incoming candidate — the adopt-by-name target once identity scoping
    // resolves the right library.
    let existing = repo
        .create_presentation(renamed.id, "Untagged Song", None)
        .await
        .unwrap();

    // A DIFFERENT existing live song, same name, under the UNRELATED
    // library — if resolution ever fell back to name-only scoping (a bug),
    // this is the WRONG row it would find instead.
    repo.create_presentation(unrelated.id, "Untagged Song", None)
        .await
        .unwrap();

    let incoming = peer_song_with_library_sync_id(
        "peer-unknown-adopt",
        &renamed_sync_id,
        "Old Name",
        "Untagged Song",
        "verse",
        0,
    );
    let target = repo
        .resolve_sync_apply_target(&incoming, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(
        target,
        Some(existing.2.id),
        "must resolve the adopt-by-name candidate under the IDENTITY-resolved \
         library, never the unrelated same-named one"
    );
}

/// #647 (review follow-up): `apply_sync_presentation`'s step-2 adopt-by-name
/// WRITE path (`try_adopt_by_name`, via `sync_apply.rs`) must scope its
/// candidate search to the IDENTITY-resolved library too — the probe test
/// above (`resolve_sync_apply_target_scopes_by_identity_not_a_stale_name`)
/// only proves the shared `find_library_id` helper resolves correctly; this
/// proves the actual adoption WRITE lands on the right row end-to-end.
#[tokio::test]
async fn apply_sync_presentation_adopts_by_name_under_the_identity_resolved_library() {
    let repo = repo().await;

    let renamed = repo.create_library("Old Name").await.unwrap();
    let renamed_sync_id = library_sync_id_of(&repo, renamed.id).await;
    library::Entity::update_many()
        .col_expr(
            library::Column::Name,
            sea_orm::sea_query::Expr::value("New Name"),
        )
        .filter(library::Column::Id.eq(renamed.id.to_string()))
        .exec(&repo.db)
        .await
        .unwrap();
    let unrelated = repo.create_library("Old Name").await.unwrap();

    // The TRUE adopt-by-name candidate: a locally-created (untagged, random
    // sync_id) live song under `renamed`.
    let existing = repo
        .create_presentation(renamed.id, "Untagged Song", None)
        .await
        .unwrap();
    // A same-named DECOY under the unrelated library — if step 2 ever
    // resolved by a stale name instead of identity, this is the WRONG row
    // it would adopt onto instead.
    let decoy = repo
        .create_presentation(unrelated.id, "Untagged Song", None)
        .await
        .unwrap();

    let incoming = peer_song_with_library_sync_id(
        "peer-adopt-write",
        &renamed_sync_id,
        "Old Name",
        "Untagged Song",
        "adopted content",
        0,
    );
    let (outcome, id) = repo
        .apply_sync_presentation(&incoming, &HashSet::new())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        crate::SyncApplyOutcome::AdoptedByName,
        "sanity: this is the adopt-by-name path, not a fresh create"
    );
    assert_eq!(
        id,
        Some(existing.2.id),
        "must adopt the EXISTING row under the identity-resolved library, \
         never create a new row or adopt the unrelated decoy"
    );

    let adopted_row = row(&repo, existing.2.id).await;
    assert_eq!(
        adopted_row.sync_id, "peer-adopt-write",
        "adoption must write the PEER's sync_id onto the existing row in place"
    );
    assert_eq!(
        adopted_row.library_id,
        renamed.id.to_string(),
        "the adopted row must stay under the TRUE (identity-resolved) library"
    );

    // The decoy under the unrelated library must be completely untouched.
    let decoy_row = row(&repo, decoy.2.id).await;
    assert_ne!(
        decoy_row.sync_id, "peer-adopt-write",
        "the unrelated same-named decoy must never be the one adopted onto"
    );
}
