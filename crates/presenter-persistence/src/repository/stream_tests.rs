//! #705 stream repository integration tests — in-memory SQLite, one isolated
//! single-connection DB per test (the `entities.rs` roundtrip idiom, with
//! `PRAGMA foreign_keys = ON` so the FK CASCADE deletes fire). Covers config
//! CRUD, activation persistence, def assembly + ordering, per-element fallback,
//! asset records + guarded delete, and config_revision monotonicity.

use super::stream_assets::NewStreamAsset;
use super::{Repository, RepositoryError};
use crate::entities::stream_element;
use presenter_core::stream::{
    ContentTransition, Frame, ImageFit, SceneKind, StreamElementProps, TextAlign, TextStyle,
};
use presenter_migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, NotSet, Set};

async fn repo() -> Repository {
    let mut opts = ConnectOptions::new("sqlite::memory:");
    opts.max_connections(1).min_connections(1);
    let db = Database::connect(opts).await.expect("connect");
    db.execute_unprepared("PRAGMA foreign_keys = ON")
        .await
        .expect("fk pragma");
    Migrator::up(&db, None).await.expect("migrate");
    Repository { db }
}

fn frame() -> Frame {
    Frame {
        x_pct: 0.0,
        y_pct: 0.0,
        w_pct: 100.0,
        h_pct: 100.0,
    }
}

fn text_style() -> TextStyle {
    TextStyle {
        font_family: "Inter".to_string(),
        size_pct: 8.0,
        color: "#ffffff".to_string(),
        weight: 700,
        align: TextAlign::Center,
        line_height: 1.2,
        shadow: None,
    }
}

fn image(asset_id: i64) -> StreamElementProps {
    StreamElementProps::Image {
        asset_id,
        fit: ImageFit::Contain,
        frame: frame(),
        opacity: 1.0,
    }
}

fn countdown(timer_id: i64) -> StreamElementProps {
    StreamElementProps::Countdown {
        timer_id,
        style: text_style(),
        frame: frame(),
        content_transition: ContentTransition::default(),
    }
}

fn as_repo_error(err: &anyhow::Error) -> &RepositoryError {
    err.downcast_ref::<RepositoryError>()
        .unwrap_or_else(|| panic!("expected RepositoryError, got: {err:?}"))
}

fn sample_asset(sha: &str) -> NewStreamAsset {
    NewStreamAsset {
        sha256: sha.to_string(),
        original_filename: "logo.png".to_string(),
        mime: "image/png".to_string(),
        size_bytes: 2048,
        width: Some(1920),
        height: Some(1080),
    }
}

#[tokio::test]
async fn output_crud_happy_path() {
    let repo = repo().await;
    // Seed output exists.
    let outputs = repo.list_stream_outputs().await.unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].slug, "stream");

    let created = repo.create_stream_output("event", "Event").await.unwrap();
    assert_eq!(created.slug, "event");
    assert_eq!(created.default_transition_ms, 400);
    assert_eq!(created.config_revision, 0);

    let renamed = repo
        .rename_stream_output("event", "Main Event")
        .await
        .unwrap();
    assert_eq!(renamed.name, "Main Event");
    assert_eq!(renamed.config_revision, 1);

    let patched = repo
        .set_stream_output_transition("event", 250)
        .await
        .unwrap();
    assert_eq!(patched.default_transition_ms, 250);
    assert_eq!(patched.config_revision, 2);

    repo.delete_stream_output("event").await.unwrap();
    assert_eq!(repo.list_stream_outputs().await.unwrap().len(), 1);
}

#[tokio::test]
async fn duplicate_slug_conflicts() {
    let repo = repo().await;
    let err = repo
        .create_stream_output("stream", "Dup")
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Conflict(_)));
    repo.create_stream_output("ev", "Ev").await.unwrap();
    let err = repo.create_stream_output("ev", "Ev2").await.unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Conflict(_)));
}

#[tokio::test]
async fn invalid_and_reserved_slugs_unprocessable() {
    let repo = repo().await;
    for slug in ["BAD", "api", "assets", "has space", ""] {
        let err = repo.create_stream_output(slug, "X").await.unwrap_err();
        assert!(
            matches!(as_repo_error(&err), RepositoryError::Invalid(_)),
            "slug {slug:?} should be Invalid"
        );
    }
}

#[tokio::test]
async fn scene_positions_are_per_kind() {
    let repo = repo().await;
    let a = repo
        .create_stream_scene("stream", "Base A", SceneKind::Base)
        .await
        .unwrap();
    let b = repo
        .create_stream_scene("stream", "Base B", SceneKind::Base)
        .await
        .unwrap();
    let o = repo
        .create_stream_scene("stream", "Overlay A", SceneKind::Overlay)
        .await
        .unwrap();
    assert_eq!(a.position, 0);
    assert_eq!(b.position, 1);
    assert_eq!(o.position, 0);
    assert_eq!(o.kind, SceneKind::Overlay);
}

#[tokio::test]
async fn duplicate_scene_name_case_insensitive_conflicts() {
    let repo = repo().await;
    repo.create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    let err = repo
        .create_stream_scene("stream", "  base  ", SceneKind::Overlay)
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Conflict(_)));
}

#[tokio::test]
async fn element_crud_and_kind_mismatch() {
    let repo = repo().await;
    let scene = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    let el = repo
        .create_stream_element(scene.id, image(1))
        .await
        .unwrap();
    // Same-kind update is fine.
    repo.update_stream_element(el.id, image(2)).await.unwrap();
    // Changing kind via update is rejected 422.
    let err = repo
        .update_stream_element(el.id, countdown(3))
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
    repo.delete_stream_element(el.id).await.unwrap();
}

#[tokio::test]
async fn invalid_props_rejected_on_create() {
    let repo = repo().await;
    let scene = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    // asset_id 0 fails the positive-ref rule in core validate_props.
    let err = repo
        .create_stream_element(scene.id, image(0))
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
}

#[tokio::test]
async fn activation_wrong_kind_unprocessable() {
    let repo = repo().await;
    let base = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    let overlay = repo
        .create_stream_scene("stream", "Overlay", SceneKind::Overlay)
        .await
        .unwrap();
    // Activating an overlay as the base scene is 422.
    let err = repo
        .set_active_scene("stream", Some(overlay.id))
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
    // Toggling the base scene as an overlay is 422.
    let err = repo
        .set_overlay_active("stream", base.id, true)
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
    // A missing body-referenced scene is 422 (TargetNotFound), not 404.
    let err = repo
        .set_active_scene("stream", Some(9999))
        .await
        .unwrap_err();
    assert!(matches!(
        as_repo_error(&err),
        RepositoryError::TargetNotFound(_)
    ));
}

#[tokio::test]
async fn activation_persists_and_reads_back() {
    let repo = repo().await;
    let base = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    let overlay = repo
        .create_stream_scene("stream", "Overlay", SceneKind::Overlay)
        .await
        .unwrap();
    repo.set_active_scene("stream", Some(base.id))
        .await
        .unwrap();
    repo.set_overlay_active("stream", overlay.id, true)
        .await
        .unwrap();
    let state = repo.get_stream_show_state("stream").await.unwrap();
    assert_eq!(state.active_scene_id, Some(base.id));
    assert_eq!(state.active_overlay_ids, vec![overlay.id]);

    // Clear resets base + overlays.
    repo.clear_stream_output("stream").await.unwrap();
    let state = repo.get_stream_show_state("stream").await.unwrap();
    assert_eq!(state.active_scene_id, None);
    assert!(state.active_overlay_ids.is_empty());
}

#[tokio::test]
async fn delete_active_base_clears_activation() {
    let repo = repo().await;
    let base = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    repo.set_active_scene("stream", Some(base.id))
        .await
        .unwrap();
    repo.delete_stream_scene(base.id).await.unwrap();
    let state = repo.get_stream_show_state("stream").await.unwrap();
    assert_eq!(state.active_scene_id, None);
}

#[tokio::test]
async fn scene_reorder_validates_and_rewrites() {
    let repo = repo().await;
    let a = repo
        .create_stream_scene("stream", "A", SceneKind::Base)
        .await
        .unwrap();
    let b = repo
        .create_stream_scene("stream", "B", SceneKind::Base)
        .await
        .unwrap();
    // Reverse order → positions rewritten.
    repo.set_scene_order("stream", vec![b.id, a.id])
        .await
        .unwrap();
    let def = repo.load_output_def("stream").await.unwrap();
    let names: Vec<&str> = def.scenes.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["B", "A"]);
    // Wrong set (missing an id) → 422.
    let err = repo
        .set_scene_order("stream", vec![a.id])
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
    // Duplicate ids → 422.
    let err = repo
        .set_scene_order("stream", vec![a.id, a.id])
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
}

#[tokio::test]
async fn element_reorder_validates_and_rewrites() {
    let repo = repo().await;
    let scene = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    // Created in order → z_order 0,1,2.
    let a = repo
        .create_stream_element(scene.id, image(1))
        .await
        .unwrap();
    let b = repo
        .create_stream_element(scene.id, countdown(2))
        .await
        .unwrap();
    let c = repo
        .create_stream_element(scene.id, image(3))
        .await
        .unwrap();
    // Reverse order → z_order rewritten, def reflects it.
    repo.set_element_order(scene.id, vec![c.id, b.id, a.id])
        .await
        .unwrap();
    let def = repo.load_output_def("stream").await.unwrap();
    let ids: Vec<i64> = def.scenes[0].elements.iter().map(|e| e.id).collect();
    assert_eq!(ids, vec![c.id, b.id, a.id]);
    let z: Vec<i32> = def.scenes[0].elements.iter().map(|e| e.z_order).collect();
    assert_eq!(z, vec![0, 1, 2]);
    // Wrong set (missing an id) → 422.
    let err = repo
        .set_element_order(scene.id, vec![a.id, b.id])
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
    // Duplicate ids → 422.
    let err = repo
        .set_element_order(scene.id, vec![a.id, a.id, a.id])
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
}

#[tokio::test]
async fn config_revision_bumps_on_config_writes_not_activation() {
    let repo = repo().await;
    assert_eq!(rev(&repo).await, 0);
    let scene = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    assert_eq!(rev(&repo).await, 1);
    repo.create_stream_element(scene.id, image(1))
        .await
        .unwrap();
    assert_eq!(rev(&repo).await, 2);
    // Activation is show-state — must NOT bump config_revision.
    repo.set_active_scene("stream", Some(scene.id))
        .await
        .unwrap();
    assert_eq!(rev(&repo).await, 2);
}

async fn rev(repo: &Repository) -> u64 {
    repo.get_stream_output("stream")
        .await
        .unwrap()
        .config_revision
}

#[tokio::test]
async fn load_output_def_orders_scenes_and_elements() {
    let repo = repo().await;
    // Overlay created BEFORE the base scene — the def must still sort base first.
    repo.create_stream_scene("stream", "Overlay", SceneKind::Overlay)
        .await
        .unwrap();
    let base = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    repo.create_stream_element(base.id, image(1)).await.unwrap();
    repo.create_stream_element(base.id, countdown(2))
        .await
        .unwrap();
    let def = repo.load_output_def("stream").await.unwrap();
    // Base scene sorts before overlay regardless of creation order.
    assert_eq!(def.scenes[0].name, "Base");
    assert_eq!(def.scenes[1].name, "Overlay");
    let z: Vec<i32> = def.scenes[0].elements.iter().map(|e| e.z_order).collect();
    assert_eq!(z, vec![0, 1]);
}

#[tokio::test]
async fn load_output_def_skips_unparseable_element() {
    let repo = repo().await;
    let scene = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    let good = repo
        .create_stream_element(scene.id, image(1))
        .await
        .unwrap();
    // Insert a corrupt-props element directly, bypassing validation.
    let now = chrono::Utc::now();
    stream_element::ActiveModel {
        id: NotSet,
        scene_id: Set(scene.id as i32),
        kind: Set("image".to_string()),
        z_order: Set(5),
        props: Set("this is not json".to_string()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&repo.db)
    .await
    .unwrap();
    let def = repo.load_output_def("stream").await.unwrap();
    let elements = &def.scenes[0].elements;
    assert_eq!(elements.len(), 1, "corrupt element must be skipped");
    assert_eq!(elements[0].id, good.id);
}

#[tokio::test]
async fn asset_dedup_by_sha256() {
    let repo = repo().await;
    let a = repo
        .insert_or_get_stream_asset(sample_asset("deadbeef"))
        .await
        .unwrap();
    let b = repo
        .insert_or_get_stream_asset(sample_asset("deadbeef"))
        .await
        .unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(repo.list_stream_assets().await.unwrap().len(), 1);
    let got = repo.get_stream_asset(a.id).await.unwrap();
    assert_eq!(got.sha256, "deadbeef");
}

#[tokio::test]
async fn asset_delete_refused_while_referenced() {
    let repo = repo().await;
    let asset = repo
        .insert_or_get_stream_asset(sample_asset("abc123"))
        .await
        .unwrap();
    let scene = repo
        .create_stream_scene("stream", "Logo Scene", SceneKind::Base)
        .await
        .unwrap();
    repo.create_stream_element(scene.id, image(asset.id))
        .await
        .unwrap();
    let err = repo.delete_stream_asset(asset.id).await.unwrap_err();
    match as_repo_error(&err) {
        RepositoryError::ConflictDetail(msg) => {
            assert!(msg.contains("Logo Scene"), "message names the scene: {msg}");
        }
        other => panic!("expected ConflictDetail, got {other:?}"),
    }
    // An unreferenced asset deletes cleanly.
    let other = repo
        .insert_or_get_stream_asset(sample_asset("f00d"))
        .await
        .unwrap();
    repo.delete_stream_asset(other.id).await.unwrap();
}

#[tokio::test]
async fn not_found_for_missing_resources() {
    let repo = repo().await;
    let e1 = repo.get_stream_output("nope").await.unwrap_err();
    assert!(matches!(as_repo_error(&e1), RepositoryError::NotFound(_)));
    let e2 = repo.rename_stream_scene(9999, "X").await.unwrap_err();
    assert!(matches!(as_repo_error(&e2), RepositoryError::NotFound(_)));
    let e3 = repo.delete_stream_element(9999).await.unwrap_err();
    assert!(matches!(as_repo_error(&e3), RepositoryError::NotFound(_)));
    let e4 = repo.get_stream_asset(9999).await.unwrap_err();
    assert!(matches!(as_repo_error(&e4), RepositoryError::NotFound(_)));
}

#[tokio::test]
async fn transition_bounds_enforced() {
    let repo = repo().await;
    let err = repo
        .set_stream_output_transition("stream", 20_000)
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
    let scene = repo
        .create_stream_scene("stream", "Base", SceneKind::Base)
        .await
        .unwrap();
    let err = repo
        .set_stream_scene_transition(scene.id, Some(20_000))
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
    // A valid transition is accepted and persisted.
    let updated = repo
        .set_stream_scene_transition(scene.id, Some(500))
        .await
        .unwrap();
    assert_eq!(updated.transition_ms, Some(500));
}

#[tokio::test]
async fn kind_transitions_set_clear_and_bump() {
    let repo = repo().await;
    // Fresh output: kind columns start unset (inherit default).
    let seeded = repo.get_stream_output("stream").await.unwrap();
    assert_eq!(seeded.base_transition_ms, None);
    assert_eq!(seeded.overlay_transition_ms, None);
    let rev0 = seeded.config_revision;

    // Set base = 0 (cut) + overlay = 800 in one call → both persist, ONE bump.
    let patched = repo
        .set_stream_output_kind_transitions("stream", Some(Some(0)), Some(Some(800)))
        .await
        .unwrap();
    assert_eq!(patched.base_transition_ms, Some(0));
    assert_eq!(patched.overlay_transition_ms, Some(800));
    assert_eq!(patched.config_revision, rev0 + 1, "one config bump");

    // Absent field (None) leaves overlay unchanged; base updated to 250.
    let patched = repo
        .set_stream_output_kind_transitions("stream", Some(Some(250)), None)
        .await
        .unwrap();
    assert_eq!(patched.base_transition_ms, Some(250));
    assert_eq!(
        patched.overlay_transition_ms,
        Some(800),
        "overlay untouched"
    );

    // Explicit null clears base back to inherit (None); overlay still untouched.
    let patched = repo
        .set_stream_output_kind_transitions("stream", Some(None), None)
        .await
        .unwrap();
    assert_eq!(patched.base_transition_ms, None, "cleared to inherit");
    assert_eq!(patched.overlay_transition_ms, Some(800));

    // The def carries both columns too.
    let def = repo.load_output_def("stream").await.unwrap();
    assert_eq!(def.base_transition_ms, None);
    assert_eq!(def.overlay_transition_ms, Some(800));

    // Out-of-range value is rejected (>10000).
    let err = repo
        .set_stream_output_kind_transitions("stream", Some(Some(20_000)), None)
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Invalid(_)));
}

#[tokio::test]
async fn rename_scene_clash_against_sibling_conflicts() {
    let repo = repo().await;
    repo.create_stream_scene("stream", "Alpha", SceneKind::Base)
        .await
        .unwrap();
    let beta = repo
        .create_stream_scene("stream", "Beta", SceneKind::Base)
        .await
        .unwrap();
    // Renaming Beta to a DIFFERENT existing scene's name clashes (409).
    let err = repo
        .rename_stream_scene(beta.id, "alpha")
        .await
        .unwrap_err();
    assert!(matches!(as_repo_error(&err), RepositoryError::Conflict(_)));
    // Renaming Beta to its own name (excluded from the check) is fine.
    let renamed = repo.rename_stream_scene(beta.id, "Beta").await.unwrap();
    assert_eq!(renamed.name, "Beta");
}

#[tokio::test]
async fn reorder_bumps_config_revision() {
    let repo = repo().await;
    let a = repo
        .create_stream_scene("stream", "A", SceneKind::Base)
        .await
        .unwrap();
    let b = repo
        .create_stream_scene("stream", "B", SceneKind::Base)
        .await
        .unwrap();
    let before = repo
        .get_stream_output("stream")
        .await
        .unwrap()
        .config_revision;
    repo.set_scene_order("stream", vec![b.id, a.id])
        .await
        .unwrap();
    let after = repo
        .get_stream_output("stream")
        .await
        .unwrap()
        .config_revision;
    assert_eq!(after, before + 1);
}

#[tokio::test]
async fn delete_output_cascades_scenes_and_elements() {
    let repo = repo().await;
    repo.create_stream_output("event", "Event").await.unwrap();
    let scene = repo
        .create_stream_scene("event", "Base", SceneKind::Base)
        .await
        .unwrap();
    let el = repo
        .create_stream_element(scene.id, image(1))
        .await
        .unwrap();
    repo.delete_stream_output("event").await.unwrap();
    // FK CASCADE removed the scene + element rows, not just the output.
    let scene_err = repo.rename_stream_scene(scene.id, "X").await.unwrap_err();
    assert!(matches!(
        as_repo_error(&scene_err),
        RepositoryError::NotFound(_)
    ));
    let el_err = repo.delete_stream_element(el.id).await.unwrap_err();
    assert!(matches!(
        as_repo_error(&el_err),
        RepositoryError::NotFound(_)
    ));
}
