//! Playlist-entry cleanup for a sync-apply write that lands (or stays) as a
//! tombstone (#649). Split out of `sync_apply.rs` into its own
//! self-contained `impl Repository` block — same pattern as
//! `android_stage.rs`/`resolume.rs`/`video_source.rs` — rather than growing
//! `sync_apply.rs`, which is already at the file-size gate's target cap.
use super::Repository;
use crate::entities::playlist_entry;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

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
}
