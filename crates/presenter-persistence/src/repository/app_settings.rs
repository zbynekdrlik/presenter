use chrono::Utc;
use sea_orm::{sea_query::OnConflict, EntityTrait, Set};
use tracing::instrument;

use crate::entities::app_settings;

use super::Repository;

impl Repository {
    #[instrument(skip_all)]
    pub async fn get_app_setting(&self, key: &str) -> anyhow::Result<Option<String>> {
        let result = app_settings::Entity::find_by_id(key.to_string())
            .one(&self.db)
            .await?;
        Ok(result.map(|model| model.value))
    }

    #[instrument(skip_all)]
    pub async fn set_app_setting(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let model = app_settings::ActiveModel {
            key: Set(key.to_string()),
            value: Set(value.to_string()),
            updated_at: Set(Utc::now().into()),
        };

        app_settings::Entity::insert(model)
            .on_conflict(
                OnConflict::column(app_settings::Column::Key)
                    .update_columns([app_settings::Column::Value, app_settings::Column::UpdatedAt])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
