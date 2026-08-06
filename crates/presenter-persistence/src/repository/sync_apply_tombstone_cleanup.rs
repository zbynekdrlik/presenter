//! Playlist-entry cleanup for a sync-apply write that lands (or stays) as a
//! tombstone (#649), plus the one-time startup backfill for entries the
//! pre-#649 bug already left dangling on deployed DBs (#658). Split out of
//! `sync_apply.rs` into its own self-contained `impl Repository` block —
//! same pattern as `android_stage.rs`/`resolume.rs`/`video_source.rs` —
//! rather than growing `sync_apply.rs`, which is already at the file-size
//! gate's target cap (989 prod lines).
use super::Repository;
use crate::entities::playlist_entry;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tracing::instrument;

impl Repository {
    /// Delete every `playlist_entry` row referencing `presentation_id` —
    /// mirrors `delete_presentation`'s local-delete cleanup
    /// (`presentation.rs`) for a `write_synced_row` write landing as a
    /// tombstone, whether FORCED by a #634/#646 library cascade or a
    /// GENUINE, non-forced tombstone the peer sent us (#649's gap: the old
    /// code only ran this cleanup for the forced case). Idempotent — a
    /// re-sync of an already-tombstoned row simply deletes zero rows.
    pub(super) async fn clean_playlist_entries_for_tombstone<C: sea_orm::ConnectionTrait>(
        conn: &C,
        presentation_id: &str,
    ) -> anyhow::Result<()> {
        playlist_entry::Entity::delete_many()
            .filter(playlist_entry::Column::PresentationId.eq(presentation_id.to_string()))
            .exec(conn)
            .await?;
        Ok(())
    }

    /// One-time startup backfill (#658): sweep `playlist_entry` rows a
    /// GENUINE incoming tombstone left dangling BEFORE the #649 fix landed.
    /// Pre-#649, `write_synced_row` only ran
    /// `clean_playlist_entries_for_tombstone` for a forced tombstone, never a
    /// genuine one — and that same tombstone re-arriving today is
    /// `SkippedNotNewer` under the strict `>` LWW gate (identical clock,
    /// `sync_apply.rs:114-121`), so the cleanup path can never re-fire for
    /// it on its own. Safe to run unconditionally post-#649: every tombstone
    /// path now cleans as it writes, so this only ever touches pre-existing
    /// residue, never a legitimately live reference. Idempotent — a DB that
    /// never hit the bug deletes zero rows every time it runs. Mirrors
    /// `prune_orphan_slide_stage_layouts`'s single-statement sweep
    /// (`slide_stage_layout.rs`). Called once at boot
    /// (`state/mod.rs::from_config`) rather than on the periodic
    /// trash-prune ticker (`background_tasks.rs`) — this backfills
    /// historical residue, it is not a recurring maintenance need.
    #[instrument(skip_all)]
    pub async fn backfill_orphaned_playlist_entries(&self) -> anyhow::Result<u64> {
        // #658 RED: no sweep exists yet — this stub is exactly the gap this
        // ticket closes: nothing currently revisits a `playlist_entry` left
        // dangling by the pre-#649 tombstone bug. The GREEN commit replaces
        // this with the real DELETE.
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use crate::repository::sync_test_support::repo;
    use crate::entities::{playlist_entry, presentation as presentation_entity};
    use sea_orm::{sea_query::Expr, ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

    /// #658 RED: seeds a tombstoned presentation with a dangling
    /// `playlist_entry` — the exact pre-#649 residue shape (the presentation
    /// row is tombstoned directly, bypassing `delete_presentation`, which
    /// already cleans entries post-fix) — plus a live presentation's entry.
    /// The sweep must remove only the dangling one.
    #[tokio::test]
    async fn backfill_removes_dangling_entries_but_keeps_live_ones() {
        let repo = repo().await;
        let library = repo.create_library("Songs").await.unwrap();

        let (_, _, live) = repo
            .create_presentation(library.id, "Live Song", None)
            .await
            .unwrap();
        let (_, _, orphaned) = repo
            .create_presentation(library.id, "Orphaned Song", None)
            .await
            .unwrap();

        // Manufacture the pre-#649 residue: tombstone the presentation
        // WITHOUT going through `delete_presentation` (which already cleans
        // playlist entries post-fix), leaving its entry dangling.
        presentation_entity::Entity::update_many()
            .col_expr(
                presentation_entity::Column::DeletedAt,
                Expr::value(chrono::Utc::now().to_rfc3339()),
            )
            .filter(presentation_entity::Column::Id.eq(orphaned.id.to_string()))
            .exec(&repo.db)
            .await
            .unwrap();

        let playlist = repo.create_playlist("Sunday", false).await.unwrap();
        let live_entry_id = uuid::Uuid::new_v4().to_string();
        playlist_entry::ActiveModel {
            id: Set(live_entry_id.clone()),
            playlist_id: Set(playlist.id.to_string()),
            entry_type: Set("presentation".to_string()),
            presentation_id: Set(Some(live.id.to_string())),
            position: Set(0),
            midi_note: Set(None),
            label: Set(None),
        }
        .insert(&repo.db)
        .await
        .unwrap();

        let dangling_entry_id = uuid::Uuid::new_v4().to_string();
        playlist_entry::ActiveModel {
            id: Set(dangling_entry_id.clone()),
            playlist_id: Set(playlist.id.to_string()),
            entry_type: Set("presentation".to_string()),
            presentation_id: Set(Some(orphaned.id.to_string())),
            position: Set(1),
            midi_note: Set(None),
            label: Set(None),
        }
        .insert(&repo.db)
        .await
        .unwrap();

        let deleted = repo.backfill_orphaned_playlist_entries().await.unwrap();
        assert_eq!(deleted, 1, "only the dangling entry must be swept");

        assert!(
            playlist_entry::Entity::find_by_id(dangling_entry_id)
                .one(&repo.db)
                .await
                .unwrap()
                .is_none(),
            "the tombstoned presentation's entry must be gone"
        );
        assert!(
            playlist_entry::Entity::find_by_id(live_entry_id)
                .one(&repo.db)
                .await
                .unwrap()
                .is_some(),
            "the live presentation's entry must survive"
        );
    }

    #[tokio::test]
    async fn backfill_is_a_noop_on_a_clean_db() {
        let repo = repo().await;
        let deleted = repo.backfill_orphaned_playlist_entries().await.unwrap();
        assert_eq!(deleted, 0);
    }
}
