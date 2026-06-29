//! Stage-state singleton persistence for [`Repository`].
//!
//! Extracted from `repository/mod.rs` (#496) to keep the central module under
//! the file-size cap and to localise the `active_entry_index` column added for
//! the worship-pp repeated-song fix. Behaviour is unchanged — these are the
//! same `impl Repository` methods, only relocated, plus the new column.

use super::util::stage_state_model_to_state;
use super::{Repository, STAGE_STATE_SINGLETON_ID};
use crate::entities::stage_state;
use chrono::Utc;
use presenter_core::StageState;
use sea_orm::{sea_query::OnConflict, EntityTrait, Set};
use tracing::instrument;

impl Repository {
    #[instrument(skip_all)]
    pub async fn get_stage_state(&self) -> anyhow::Result<Option<StageState>> {
        let model = stage_state::Entity::find_by_id(STAGE_STATE_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?;
        model
            .map(|record| stage_state_model_to_state(record).map_err(anyhow::Error::from))
            .transpose()
    }

    #[instrument(skip_all)]
    pub async fn upsert_stage_state(&self, state: &StageState) -> anyhow::Result<()> {
        let now = Utc::now();
        let model = stage_state::ActiveModel {
            id: Set(STAGE_STATE_SINGLETON_ID.to_string()),
            presentation_id: Set(state.presentation_id.map(|id| id.into_uuid().to_string())),
            current_slide_id: Set(state.current_slide_id.map(|id| id.into_uuid().to_string())),
            next_slide_id: Set(state.next_slide_id.map(|id| id.into_uuid().to_string())),
            playlist_id: Set(state.playlist_id.map(|id| id.into_uuid().to_string())),
            // #496: persist which playlist occurrence is active so rebuilds
            // (timer ticks, fresh stage connections) keep targeting the same row.
            active_entry_index: Set(state.active_entry_index.map(|i| i as i32)),
            updated_at: Set(now.into()),
        };

        stage_state::Entity::insert(model)
            .on_conflict(
                OnConflict::column(stage_state::Column::Id)
                    .update_columns([
                        stage_state::Column::PresentationId,
                        stage_state::Column::CurrentSlideId,
                        stage_state::Column::NextSlideId,
                        stage_state::Column::PlaylistId,
                        stage_state::Column::ActiveEntryIndex,
                        stage_state::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;

        Ok(())
    }
}
