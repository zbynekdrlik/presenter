//! #789: the NDI name is a video source's only identity. The legacy
//! `video_sources.label` column (NOT NULL since
//! `m20260625_000001_add_video_sources`) stays in the schema — no destructive
//! migration on live SNV/PP data — but the repository writes `label = ndi_name`
//! and never reads it back into the domain.

use super::Repository;
use crate::audit::SettingsAuditSource;
use crate::entities::video_source;
use chrono::Utc;
use presenter_core::{VideoSourceDraft, VideoSourceId};
use sea_orm::{EntityTrait, Set};

async fn stored_label(repo: &Repository, id: VideoSourceId) -> String {
    video_source::Entity::find_by_id(id.to_string())
        .one(repo.connection_for_tests())
        .await
        .unwrap()
        .expect("row exists")
        .label
}

#[tokio::test]
async fn create_writes_the_legacy_label_column_as_the_ndi_name() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let created = repo
        .create_video_source(
            &VideoSourceDraft::new("  RESOLUME-PP (cg-obs) "),
            SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(created.ndi_name, "RESOLUME-PP (cg-obs)");
    assert_eq!(
        stored_label(&repo, created.id).await,
        "RESOLUME-PP (cg-obs)"
    );
}

#[tokio::test]
async fn update_rewrites_the_legacy_label_column_to_the_new_ndi_name() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let created = repo
        .create_video_source(
            &VideoSourceDraft::new("CAM1 (usb)"),
            SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    let updated = repo
        .update_video_source(
            created.id,
            &VideoSourceDraft::new("CAM2 (hdmi)"),
            SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(updated.ndi_name, "CAM2 (hdmi)");
    assert_eq!(stored_label(&repo, created.id).await, "CAM2 (hdmi)");
}

/// A pre-#789 row whose hand-typed label differs from its NDI name (every
/// existing SNV/PP source) must still list, activate and edit.
#[tokio::test]
async fn legacy_row_with_a_hand_typed_label_still_lists_and_activates() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let id = VideoSourceId::new();
    let now = Utc::now();
    video_source::Entity::insert(video_source::ActiveModel {
        id: Set(id.to_string()),
        label: Set("tv".to_string()),
        ndi_name: Set("STREAM-SNV (stream)".to_string()),
        is_active: Set(false),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    })
    .exec(repo.connection_for_tests())
    .await
    .unwrap();

    let listed = repo.list_video_sources().await.unwrap();
    let row = listed
        .iter()
        .find(|s| s.id == id)
        .expect("legacy row listed");
    assert_eq!(row.ndi_name, "STREAM-SNV (stream)");

    let activated = repo
        .activate_video_source(id, SettingsAuditSource::HttpSetter, "test")
        .await
        .unwrap();
    assert!(activated.is_active);
    assert_eq!(activated.ndi_name, "STREAM-SNV (stream)");
    let active = repo.get_active_video_source().await.unwrap();
    assert_eq!(active.map(|s| s.id), Some(id));
}

#[tokio::test]
async fn list_is_ordered_by_ndi_name_not_by_the_legacy_label() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let now = Utc::now();
    // Legacy labels sort the OPPOSITE way to the NDI names.
    for (label, ndi_name) in [("a-first", "ZULU (z)"), ("z-last", "ALPHA (a)")] {
        video_source::Entity::insert(video_source::ActiveModel {
            id: Set(VideoSourceId::new().to_string()),
            label: Set(label.to_string()),
            ndi_name: Set(ndi_name.to_string()),
            is_active: Set(false),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        })
        .exec(repo.connection_for_tests())
        .await
        .unwrap();
    }
    let names: Vec<String> = repo
        .list_video_sources()
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.ndi_name)
        .collect();
    assert_eq!(names, vec!["ALPHA (a)".to_string(), "ZULU (z)".to_string()]);
}
