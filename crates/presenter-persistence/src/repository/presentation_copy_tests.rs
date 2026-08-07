//! #570: deep-copy a presentation into another library. Kept in its own file
//! (not `tests.rs`, which is already over the file-size cap) per the
//! presenter-ci playbook.
use super::util::RepositoryError;
use crate::entities::presentation as presentation_entity;
use crate::Repository;
use presenter_core::{LibraryId, Slide, SlideContent, SlideText};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

async fn repo() -> Repository {
    Repository::connect_in_memory()
        .await
        .expect("in-memory repo")
}

fn slide(order: u32, main: &str, group: Option<&str>) -> Slide {
    Slide::new(
        order,
        SlideContent::new(
            SlideText::new(main).unwrap(),
            SlideText::new(format!("{main}-translation")).unwrap(),
            SlideText::new(format!("{main}-stage")).unwrap(),
            group.map(presenter_core::slide::SlideGroup::new),
        ),
    )
}

/// Two libraries + a two-slide source song in the first; returns
/// (repo, source_library, target_library, source_presentation).
async fn setup() -> (
    Repository,
    presenter_core::Library,
    presenter_core::Library,
    presenter_core::Presentation,
) {
    let repo = repo().await;
    let lib_a = repo.create_library("Source Lib").await.unwrap();
    let lib_b = repo.create_library("Target Lib").await.unwrap();
    let slides = vec![slide(0, "verse", Some("Verse 1")), slide(1, "chorus", None)];
    let (_, _, source) = repo
        .create_presentation(lib_a.id, "Amazing Song", Some(&slides))
        .await
        .unwrap();
    (repo, lib_a, lib_b, source)
}

#[tokio::test]
async fn copy_lands_in_the_target_library_with_same_content_but_fresh_ids() {
    let (repo, _, lib_b, source) = setup().await;

    let (copied_lib, copied_lib_name, copy) = repo
        .copy_presentation_to_library(source.id, lib_b.id)
        .await
        .unwrap();

    assert_eq!(copied_lib, lib_b.id, "copy must land in the target library");
    assert_eq!(copied_lib_name, "Target Lib");
    assert_ne!(copy.id, source.id, "copy must be a NEW presentation");
    assert_eq!(copy.name, source.name);
    assert_eq!(copy.slides.len(), source.slides.len());
    for (copied, original) in copy.slides.iter().zip(source.slides.iter()) {
        assert_eq!(copied.content, original.content, "content copied verbatim");
        assert_ne!(copied.id, original.id, "every slide id must be fresh");
    }

    // The copy syncs as its OWN song (#555) — never pairs with the original.
    let sync_ids: Vec<String> = presentation_entity::Entity::find()
        .all(&repo.db)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.sync_id)
        .collect();
    assert_eq!(sync_ids.len(), 2);
    assert_ne!(sync_ids[0], sync_ids[1], "sync_id must be fresh");
}

#[tokio::test]
async fn copy_is_independent_of_the_original_after_edits_and_delete() {
    let (repo, _, lib_b, source) = setup().await;
    let (_, _, copy) = repo
        .copy_presentation_to_library(source.id, lib_b.id)
        .await
        .unwrap();

    // Mutate + trash the ORIGINAL.
    repo.update_slide_content(
        source.id,
        source.slides[0].id,
        &slide(0, "rewritten", None).content,
    )
    .await
    .unwrap();
    repo.delete_presentation(source.id).await.unwrap();

    let (_, _, copy_after) = repo
        .fetch_presentation_detail(copy.id)
        .await
        .unwrap()
        .expect("copy must survive edits + deletion of the original");
    assert_eq!(
        copy_after.slides[0].content.main.value(),
        "verse",
        "the copy's content must not follow the original's edits"
    );
}

#[tokio::test]
async fn copy_remaps_slide_stage_layout_markers_onto_the_new_slide_ids() {
    let (repo, _, lib_b, source) = setup().await;
    repo.set_slide_stage_layout(source.id, source.slides[1].id, "ndi-fullscreen")
        .await
        .unwrap();

    let (_, _, copy) = repo
        .copy_presentation_to_library(source.id, lib_b.id)
        .await
        .unwrap();

    let markers = repo.list_slide_stage_layouts(copy.id).await.unwrap();
    assert_eq!(
        markers.get(&copy.slides[1].id.to_string()),
        Some(&"ndi-fullscreen".to_string()),
        "marker must follow the copied slide under its NEW id"
    );
    assert!(
        !markers.contains_key(&source.slides[1].id.to_string()),
        "the copy's markers must not point at the ORIGINAL slide id"
    );
}

#[tokio::test]
async fn copy_into_a_missing_library_errors_and_writes_nothing() {
    let (repo, _, _, source) = setup().await;

    let err = repo
        .copy_presentation_to_library(source.id, LibraryId::new())
        .await
        .expect_err("copy into a vanished library must fail");
    // #584: assert the TYPED variant the router downcasts on, not just the
    // Display text — a `.context(...)` added upstream could still change
    // `to_string()` without this assertion catching a regression.
    assert!(
        matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::TargetNotFound(_))
        ),
        "expected RepositoryError::TargetNotFound, got: {err}"
    );
    assert!(
        err.to_string().contains("target library not found"),
        "{err}"
    );

    let count = presentation_entity::Entity::find()
        .all(&repo.db)
        .await
        .unwrap()
        .len();
    assert_eq!(count, 1, "no copy row may be written on failure");
}

/// #642: `copy_presentation_to_library` has no special-case for "target ==
/// source library" — every existing test above targets a DISTINCT
/// destination. This pins that a same-library copy still succeeds and still
/// produces a fully independent row (fresh id, fresh sync_id, own slides),
/// landing the app in a state with TWO same-named presentations in ONE
/// library. That duplicate-name state is a pre-existing, general
/// characteristic of the app — NOT something `copy_presentation_to_library`
/// introduces or is expected to guard against: `rename_presentation`
/// (`presentation.rs`) performs no uniqueness check either, so a user can
/// already reach two same-named songs in one library by renaming, with or
/// without ever copying. There is no per-library name-uniqueness constraint
/// in the schema (see `presenter-migration`). Given that, this ticket's own
/// "auto-rename or dedupe?" question is answered here as: neither — doing so
/// only for the copy path would be an inconsistent partial fix for a
/// system-wide allowance. AbleSet's name-based last-write-wins resolution
/// silently picking one of the two is a known, accepted consequence of
/// allowing duplicate names at all (documented on the ticket), not a defect
/// in this function.
#[tokio::test]
async fn copy_into_the_same_library_creates_an_independent_duplicate() {
    let repo = repo().await;
    let lib = repo.create_library("Songs").await.unwrap();
    let slides = vec![slide(0, "verse", Some("Verse 1")), slide(1, "chorus", None)];
    let (_, _, source) = repo
        .create_presentation(lib.id, "Amazing Song", Some(&slides))
        .await
        .unwrap();

    let (copied_lib, copied_lib_name, copy) = repo
        .copy_presentation_to_library(source.id, lib.id)
        .await
        .expect("copying into the same library must succeed");

    assert_eq!(copied_lib, lib.id, "copy stays in the same library");
    assert_eq!(copied_lib_name, "Songs");
    assert_ne!(
        copy.id, source.id,
        "copy must be a NEW presentation, not a rename"
    );
    assert_eq!(
        copy.name, source.name,
        "same-library copy duplicates the name — this is the collision AbleSet's \
         name-based resolution has to tolerate (documented on #642), not a bug here"
    );
    assert_eq!(copy.slides.len(), source.slides.len());
    for (copied, original) in copy.slides.iter().zip(source.slides.iter()) {
        assert_eq!(copied.content, original.content, "content copied verbatim");
        assert_ne!(copied.id, original.id, "every slide id must be fresh");
    }

    // Both presentations now live, side by side, in the ONE library.
    let in_library = presentation_entity::Entity::find()
        .filter(presentation_entity::Column::LibraryId.eq(lib.id.to_string()))
        .filter(presentation_entity::Column::DeletedAt.is_null())
        .all(&repo.db)
        .await
        .unwrap();
    assert_eq!(
        in_library.len(),
        2,
        "source and copy must both be live in the same library"
    );
    let sync_ids: Vec<String> = in_library.into_iter().map(|m| m.sync_id).collect();
    assert_ne!(
        sync_ids[0], sync_ids[1],
        "sync_id must be fresh even for a same-library copy (#555 — never pairs with the original)"
    );

    // Independence: editing/trashing the ORIGINAL must not touch the copy,
    // even though both now share a library and a name.
    repo.update_slide_content(
        source.id,
        source.slides[0].id,
        &slide(0, "rewritten", None).content,
    )
    .await
    .unwrap();
    repo.delete_presentation(source.id).await.unwrap();

    let (_, _, copy_after) = repo
        .fetch_presentation_detail(copy.id)
        .await
        .unwrap()
        .expect("the copy must survive edits + deletion of the same-library original");
    assert_eq!(
        copy_after.slides[0].content.main.value(),
        "verse",
        "the copy's content must not follow the original's edits"
    );
}

#[tokio::test]
async fn copying_a_trashed_presentation_errors() {
    let (repo, _, lib_b, source) = setup().await;
    repo.delete_presentation(source.id).await.unwrap();

    let err = repo
        .copy_presentation_to_library(source.id, lib_b.id)
        .await
        .expect_err("a soft-deleted song must not be copyable");
    // #584: assert the TYPED variant, not just the Display text.
    assert!(
        matches!(
            err.downcast_ref::<RepositoryError>(),
            Some(RepositoryError::NotFound(_))
        ),
        "expected RepositoryError::NotFound, got: {err}"
    );
    assert!(err.to_string().contains("presentation not found"), "{err}");

    let live = presentation_entity::Entity::find()
        .filter(presentation_entity::Column::DeletedAt.is_null())
        .all(&repo.db)
        .await
        .unwrap();
    assert!(live.is_empty(), "no copy row may be written on failure");
}
