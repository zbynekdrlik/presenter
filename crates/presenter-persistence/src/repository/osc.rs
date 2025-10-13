use sea_orm::EntityTrait;
use tracing::instrument;

use crate::entities::osc_settings;
use presenter_core::{OscSettings, OscSettingsDraft};

use super::util::{osc_model_to_domain, velocity_mode_to_string};
use super::Repository;

const OSC_SETTINGS_SINGLETON_ID: &str = "osc";

impl Repository {
    #[instrument(skip_all)]
    pub async fn get_osc_settings(&self) -> anyhow::Result<OscSettings> {
        if let Some(model) = osc_settings::Entity::find_by_id(OSC_SETTINGS_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?
        {
            return Ok(osc_model_to_domain(model)?);
        }
        self.insert_osc_settings(OscSettingsDraft::default()).await
    }

    #[instrument(skip_all)]
    pub async fn upsert_osc_settings(
        &self,
        draft: &OscSettingsDraft,
    ) -> anyhow::Result<OscSettings> {
        draft.validate().map_err(|err| anyhow::anyhow!(err))?;
        self.insert_osc_settings(draft.clone()).await
    }

    async fn insert_osc_settings(&self, draft: OscSettingsDraft) -> anyhow::Result<OscSettings> {
        let now = chrono::Utc::now();
        let address = draft.address_pattern.trim().to_string();
        let mode = velocity_mode_to_string(draft.velocity_mode).to_string();
        let active = osc_settings::ActiveModel {
            id: sea_orm::ActiveValue::set(OSC_SETTINGS_SINGLETON_ID.to_string()),
            enabled: sea_orm::ActiveValue::set(draft.enabled),
            listen_port: sea_orm::ActiveValue::set(draft.listen_port as i32),
            address_pattern: sea_orm::ActiveValue::set(address.clone()),
            velocity_mode: sea_orm::ActiveValue::set(mode.clone()),
            created_at: sea_orm::ActiveValue::set(now.into()),
            updated_at: sea_orm::ActiveValue::set(now.into()),
        };

        use sea_orm::sea_query::OnConflict;
        osc_settings::Entity::insert(active)
            .on_conflict(
                OnConflict::column(osc_settings::Column::Id)
                    .update_columns([
                        osc_settings::Column::Enabled,
                        osc_settings::Column::ListenPort,
                        osc_settings::Column::AddressPattern,
                        osc_settings::Column::VelocityMode,
                        osc_settings::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;

        let model = osc_settings::Entity::find_by_id(OSC_SETTINGS_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow::anyhow!("osc settings missing after upsert"))?;
        Ok(osc_model_to_domain(model)?)
    }
}
