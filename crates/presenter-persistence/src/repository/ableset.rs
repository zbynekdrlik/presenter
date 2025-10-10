use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, Schema, Set};
use tracing::instrument;

use crate::entities::ableset_settings;
use presenter_core::{AbleSetSettings, AbleSetSettingsDraft};

use super::util::ableset_model_to_domain;
use super::Repository;

const ABLESET_SETTINGS_SINGLETON_ID: &str = "ableset";

impl Repository {
    #[instrument(skip_all)]
    pub async fn get_ableset_settings(&self) -> anyhow::Result<AbleSetSettings> {
        self.ensure_ableset_settings_table().await?;
        if let Some(mut model) =
            ableset_settings::Entity::find_by_id(ABLESET_SETTINGS_SINGLETON_ID.to_string())
                .one(&self.db)
                .await?
        {
            let defaults = AbleSetSettingsDraft::default();
            let mut needs_update = false;
            if model.http_port == 5950 {
                model.http_port = defaults.http_port as i32;
                needs_update = true;
            }
            if model.osc_port == 5950 {
                model.osc_port = defaults.osc_port as i32;
                needs_update = true;
            }
            if model.library_name.trim().eq_ignore_ascii_case("NEWLEVEL") {
                model.library_name = defaults.library_name.clone();
                needs_update = true;
            }
            if needs_update {
                let mut active: ableset_settings::ActiveModel = model.clone().into();
                active.http_port = sea_orm::ActiveValue::set(model.http_port);
                active.osc_port = sea_orm::ActiveValue::set(model.osc_port);
                active.updated_at = sea_orm::ActiveValue::set(chrono::Utc::now().into());
                active.update(&self.db).await?;
                model =
                    ableset_settings::Entity::find_by_id(ABLESET_SETTINGS_SINGLETON_ID.to_string())
                        .one(&self.db)
                        .await?
                        .ok_or_else(|| {
                            anyhow::anyhow!("ableset settings missing after migration")
                        })?;
            }
            return Ok(ableset_model_to_domain(model)?);
        }
        self.insert_ableset_settings(AbleSetSettingsDraft::default())
            .await
    }

    #[instrument(skip_all)]
    pub async fn upsert_ableset_settings(
        &self,
        draft: &AbleSetSettingsDraft,
    ) -> anyhow::Result<AbleSetSettings> {
        draft.validate().map_err(|err| anyhow::anyhow!(err))?;
        self.insert_ableset_settings(draft.clone()).await
    }

    async fn insert_ableset_settings(
        &self,
        draft: AbleSetSettingsDraft,
    ) -> anyhow::Result<AbleSetSettings> {
        self.ensure_ableset_settings_table().await?;
        let now = chrono::Utc::now();
        let active = ableset_settings::ActiveModel {
            id: sea_orm::ActiveValue::set(ABLESET_SETTINGS_SINGLETON_ID.to_string()),
            enabled: sea_orm::ActiveValue::set(draft.enabled),
            host: sea_orm::ActiveValue::set(draft.host.trim().to_string()),
            osc_port: sea_orm::ActiveValue::set(draft.osc_port as i32),
            http_port: sea_orm::ActiveValue::set(draft.http_port as i32),
            library_name: sea_orm::ActiveValue::set(draft.library_name.trim().to_string()),
            song_prefix_length: sea_orm::ActiveValue::set(draft.song_prefix_length as i32),
            created_at: sea_orm::ActiveValue::set(now.into()),
            updated_at: sea_orm::ActiveValue::set(now.into()),
        };

        use sea_orm::sea_query::OnConflict;
        ableset_settings::Entity::insert(active)
            .on_conflict(
                OnConflict::column(ableset_settings::Column::Id)
                    .update_columns([
                        ableset_settings::Column::Enabled,
                        ableset_settings::Column::Host,
                        ableset_settings::Column::OscPort,
                        ableset_settings::Column::HttpPort,
                        ableset_settings::Column::LibraryName,
                        ableset_settings::Column::SongPrefixLength,
                        ableset_settings::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;

        let model = ableset_settings::Entity::find_by_id(ABLESET_SETTINGS_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow::anyhow!("ableset settings missing after upsert"))?;

        Ok(ableset_model_to_domain(model)?)
    }

    async fn ensure_ableset_settings_table(&self) -> anyhow::Result<()> {
        let backend = self.db.get_database_backend();
        let builder = Schema::new(backend);
        let table = builder
            .create_table_from_entity(ableset_settings::Entity)
            .if_not_exists()
            .to_owned();
        let statement = backend.build(&table);
        self.db.execute(statement).await?;
        Ok(())
    }
}
