//! #558 S6 regression: a local slide/name edit and its `updated_at` bump must
//! be ATOMIC against a concurrent sync apply. Kept in its own file (not
//! `tests.rs`, which is already over the file-size cap) per the presenter-ci
//! playbook.
use crate::entities::presentation as presentation_entity;
use crate::{Repository, SyncPresentation};
use presenter_core::{PresentationId, Slide, SlideContent, SlideText};
use sea_orm::EntityTrait;

async fn repo() -> Repository {
    Repository::connect_in_memory()
        .await
        .expect("in-memory repo")
}

fn slide(order: u32, main: &str) -> Slide {
    Slide::new(
        order,
        SlideContent::new(
            SlideText::new(main).unwrap(),
            SlideText::new("").unwrap(),
            SlideText::new("").unwrap(),
            None,
        ),
    )
}

async fn row(repo: &Repository, id: PresentationId) -> presentation_entity::Model {
    presentation_entity::Entity::find_by_id(id.to_string())
        .one(&repo.db)
        .await
        .unwrap()
        .expect("presentation row exists")
}

/// S6 regression: `update_slide_content`'s content write and its
/// `updated_at` bump used to be TWO separate autocommit statements. A
/// concurrent sync apply landing between them reads the OLD (not-yet-bumped)
/// `updated_at`, "wins" the LWW race, and overwrites the just-made local
/// edit with the peer's (stale) slide content -- then the local touch
/// statement lands AFTER, stamping the row with a fresh "now()" timestamp
/// over content that is actually the PEER's copy, not the local edit. The
/// result: content from ONE source, `updated_at` from an unrelated bump --
/// exactly the corruption the fix (one transaction) makes impossible. Runs
/// many concurrent trials because the interleave is a real race, not a
/// deterministic single-shot condition (no loom in this workspace); the fix
/// makes the invariant hold on EVERY trial because it is atomicity, not
/// timing, that guarantees it.
#[tokio::test]
async fn slide_edit_and_touch_are_atomic_against_a_concurrent_sync_apply() {
    const TRIALS: usize = 300;
    const LOCAL_TEXT: &str = "after local edit";
    const PEER_TEXT: &str = "stale peer content";

    for trial in 0..TRIALS {
        let repo = repo().await;
        let lib = repo.create_library("Songs").await.unwrap();
        let (_, _, pres) = repo
            .create_presentation(lib.id, "Racer", Some(&[slide(0, "before")]))
            .await
            .unwrap();
        let slide_id = pres.slides[0].id;
        let before = row(&repo, pres.id).await;
        let sync_id = before.sync_id.clone();
        let base_updated_at: chrono::DateTime<chrono::Utc> = before.updated_at.into();

        // A peer copy that is barely newer than the CURRENT (pre-edit)
        // updated_at -- newer than the stale timestamp `apply` would read if
        // it lands mid-edit, but never newer than a genuine local touch.
        let peer_updated_at = base_updated_at + chrono::Duration::milliseconds(1);
        let incoming = SyncPresentation {
            sync_id,
            library_name: "Songs".to_string(),
            name: "Racer".to_string(),
            updated_at: peer_updated_at,
            deleted_at: None,
            slides: vec![slide(0, PEER_TEXT)],
        };

        let local_content = SlideContent::new(
            SlideText::new(LOCAL_TEXT).unwrap(),
            SlideText::new("").unwrap(),
            SlideText::new("").unwrap(),
            None,
        );

        let edit_repo = repo.clone();
        let edit = tokio::spawn(async move {
            edit_repo
                .update_slide_content(pres.id, slide_id, &local_content)
                .await
        });
        let apply_repo = repo.clone();
        let apply = tokio::spawn(async move {
            apply_repo
                .apply_sync_presentation(&incoming, &std::collections::HashSet::new())
                .await
        });
        let (edit_result, apply_result) = tokio::join!(edit, apply);
        apply_result
            .expect("apply task panicked")
            .expect("apply failed");
        // The local edit can legitimately fail with "not found" if the
        // concurrent apply's wholesale slide replacement lands ENTIRELY
        // before it (a separate, expected id-lifetime effect of "replace
        // slides wholesale" -- not the mutation+touch atomicity this test
        // targets). Any other error is unexpected.
        if let Err(err) = edit_result.expect("edit task panicked") {
            let msg = err.to_string();
            assert!(
                msg.contains("not found"),
                "trial {trial}: unexpected edit error: {msg}"
            );
        }

        let after = row(&repo, pres.id).await;
        let final_updated_at: chrono::DateTime<chrono::Utc> = after.updated_at.into();
        let detail = repo
            .fetch_presentation_detail(pres.id)
            .await
            .unwrap()
            .expect("presentation still exists");
        let final_text = detail.2.slides[0].content.main.value().to_string();

        let torn = final_text == PEER_TEXT && final_updated_at != peer_updated_at;
        assert!(
            !torn,
            "trial {trial}: torn state -- content is the PEER's stale copy \
             but updated_at ({final_updated_at:?}) is not the peer's verbatim \
             timestamp ({peer_updated_at:?}); a local touch clobbered the \
             timestamp AFTER a concurrent apply overwrote the content"
        );
    }
}
