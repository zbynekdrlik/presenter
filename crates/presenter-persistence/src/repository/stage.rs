use sea_orm::{EntityTrait, Set};
use tracing::instrument;

use crate::entities::stage_state;
use presenter_core::StageState;

use super::util::stage_state_model_to_state;
use super::Repository;

const STAGE_STATE_SINGLETON_ID: &str = "stage-state";

impl Repository {
    #[instrument(skip_all)]
    pub async fn get_stage_state(&self) -> anyhow::Result<Option<StageState>> {
        let model = stage_state::Entity::find_by_id(STAGE_STATE_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?;
        Ok(match model {
            Some(model) => Some(stage_state_model_to_state(model)?),
            None => None,
        })
    }

    #[instrument(skip_all)]
    pub async fn upsert_stage_state(&self, state: &StageState) -> anyhow::Result<()> {
        let active = stage_state::ActiveModel {
            id: Set(STAGE_STATE_SINGLETON_ID.to_string()),
            presentation_id: Set(state.presentation_id.map(|id| id.to_string())),
            current_slide_id: Set(state.current_slide_id.map(|id| id.to_string())),
            next_slide_id: Set(state.next_slide_id.map(|id| id.to_string())),
            updated_at: Set(chrono::Utc::now().into()),
        };
        use sea_orm::sea_query::OnConflict;
        stage_state::Entity::insert(active)
            .on_conflict(
                OnConflict::column(stage_state::Column::Id)
                    .update_columns([
                        stage_state::Column::PresentationId,
                        stage_state::Column::CurrentSlideId,
                        stage_state::Column::NextSlideId,
                        stage_state::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
