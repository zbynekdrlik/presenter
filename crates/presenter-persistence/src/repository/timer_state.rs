use sea_orm::{EntityTrait, Set};
use tracing::instrument;

use crate::entities::timers;
use presenter_core::{TimerState, TimersState};

use super::util::{timer_state_to_string, timers_model_to_state};
use super::Repository;

const TIMERS_SINGLETON_ID: &str = "timers";

impl Repository {
    #[instrument(skip_all)]
    pub async fn get_timers_state(&self) -> anyhow::Result<Option<TimersState>> {
        let model = timers::Entity::find_by_id(TIMERS_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?;
        Ok(match model {
            Some(model) => Some(timers_model_to_state(model)?),
            None => None,
        })
    }

    #[instrument(skip_all)]
    pub async fn upsert_timers_state(&self, state: &TimersState) -> anyhow::Result<()> {
        let now = chrono::Utc::now();
        let active = timers::ActiveModel {
            id: Set(TIMERS_SINGLETON_ID.to_string()),
            countdown_target: Set(state.countdown.target.into()),
            countdown_state: Set(timer_state_to_string(state.countdown.state.clone())),
            preach_started_at: Set(state.preach.started_at().map(Into::into)),
            preach_accumulated_seconds: Set(state.preach.accumulated_duration().num_seconds()),
            preach_state: Set(timer_state_to_string(state.preach.state.clone())),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };
        use sea_orm::sea_query::OnConflict;
        timers::Entity::insert(active)
            .on_conflict(
                OnConflict::column(timers::Column::Id)
                    .update_columns([
                        timers::Column::CountdownTarget,
                        timers::Column::CountdownState,
                        timers::Column::PreachStartedAt,
                        timers::Column::PreachAccumulatedSeconds,
                        timers::Column::PreachState,
                        timers::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;

        Ok(())
    }
}
