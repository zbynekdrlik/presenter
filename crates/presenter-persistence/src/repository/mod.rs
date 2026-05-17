mod bible;
mod group_color;
mod library;
mod playlist;
mod presentation;
mod search;
#[cfg(test)]
mod tests;
mod util;

use util::{
    ableset_model_to_domain, android_stage_display_model_to_domain, osc_model_to_domain,
    resolume_model_to_domain, stage_state_model_to_state, timer_state_to_string,
    timers_model_to_state, velocity_mode_to_string, video_source_model_to_domain,
};

use crate::audit::SettingsAuditSource;
use crate::entities::{
    ableset_settings, android_stage_display, app_settings, osc_settings, resolume_host,
    stage_state, timers, video_source,
};
use anyhow::{anyhow, Context};
use chrono::Utc;
use presenter_core::{
    AbleSetSettings, AbleSetSettingsDraft, AndroidStageDisplay, AndroidStageDisplayDraft,
    AndroidStageDisplayId, OscSettings, OscSettingsDraft, ResolumeHost, ResolumeHostDraft,
    ResolumeHostId, StageState, TimersState, VideoSource, VideoSourceDraft, VideoSourceId,
};
use presenter_migration::{Migrator, MigratorTrait};
use sea_orm::Statement;
use sea_orm::{
    sea_query::{Expr, OnConflict},
    ActiveModelTrait, ColumnTrait, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, Schema, Set,
};
use std::fmt::Debug;
use tracing::instrument;

const TIMERS_SINGLETON_ID: &str = "timers";
const STAGE_STATE_SINGLETON_ID: &str = "stage-state";
const OSC_SETTINGS_SINGLETON_ID: &str = "osc";
const ABLESET_SETTINGS_SINGLETON_ID: &str = "ableset";
#[derive(Debug, Clone)]
pub struct Repository {
    pub(crate) db: DatabaseConnection,
}

#[derive(Debug, Clone)]
pub struct DatabaseSettings {
    pub url: String,
}

impl DatabaseSettings {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

impl Repository {
    #[instrument(skip_all)]
    pub async fn connect(settings: &DatabaseSettings) -> anyhow::Result<Self> {
        let db = Database::connect(settings.url.as_str())
            .await
            .with_context(|| format!("failed to connect to database at {}", settings.url))?;
        Self::apply_sqlite_pragmas(&db).await?;
        Self::migrate(&db).await?;
        Ok(Self { db })
    }

    #[instrument(skip_all)]
    pub async fn connect_in_memory() -> anyhow::Result<Self> {
        let db = Database::connect("sqlite::memory:?cache=shared")
            .await
            .context("failed to start in-memory sqlite")?;
        Self::migrate(&db).await?;
        Ok(Self { db })
    }

    async fn migrate(db: &DatabaseConnection) -> anyhow::Result<()> {
        Migrator::up(db, None).await?;
        Ok(())
    }

    async fn apply_sqlite_pragmas(db: &DatabaseConnection) -> anyhow::Result<()> {
        let backend = db.get_database_backend();
        for pragma in [
            "PRAGMA journal_mode = WAL",
            "PRAGMA wal_autocheckpoint = 1000",
            "PRAGMA busy_timeout = 5000",
        ] {
            db.execute(Statement::from_string(backend, pragma.to_string()))
                .await
                .with_context(|| format!("failed to execute {pragma}"))?;
        }
        Ok(())
    }

    /// Run a WAL checkpoint to keep the WAL file from growing unbounded.
    pub async fn wal_checkpoint(&self) -> anyhow::Result<()> {
        let backend = self.db.get_database_backend();
        self.db
            .execute(Statement::from_string(
                backend,
                "PRAGMA wal_checkpoint(TRUNCATE)".to_string(),
            ))
            .await
            .context("WAL checkpoint failed")?;
        Ok(())
    }

    pub fn connection(&self) -> &DatabaseConnection {
        &self.db
    }

    #[cfg(test)]
    pub fn connection_for_tests(&self) -> &DatabaseConnection {
        &self.db
    }

    #[instrument(skip(self, before, after))]
    pub async fn record_settings_audit(
        &self,
        setting_table: &'static str,
        setting_id: &str,
        source: SettingsAuditSource,
        actor: &str,
        before: Option<serde_json::Value>,
        after: serde_json::Value,
    ) -> anyhow::Result<()> {
        use crate::entities::settings_audit;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let active = settings_audit::ActiveModel {
            id: sea_orm::ActiveValue::set(id),
            setting_table: sea_orm::ActiveValue::set(setting_table.to_string()),
            setting_id: sea_orm::ActiveValue::set(setting_id.to_string()),
            source: sea_orm::ActiveValue::set(source.as_str().to_string()),
            actor: sea_orm::ActiveValue::set(actor.to_string()),
            before_json: sea_orm::ActiveValue::set(before.map(|v| v.to_string())),
            after_json: sea_orm::ActiveValue::set(after.to_string()),
            changed_at: sea_orm::ActiveValue::set(now.into()),
        };
        crate::entities::settings_audit::Entity::insert(active)
            .exec(&self.db)
            .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn list_settings_audit(
        &self,
        setting_table: Option<&str>,
        setting_id: Option<&str>,
        since: Option<chrono::DateTime<chrono::Utc>>,
        limit: u64,
    ) -> anyhow::Result<Vec<crate::audit::SettingsAuditEntry>> {
        use crate::entities::settings_audit;
        use sea_orm::QuerySelect;
        let mut q = settings_audit::Entity::find()
            .order_by_desc(settings_audit::Column::ChangedAt)
            .limit(limit);
        if let Some(t) = setting_table {
            q = q.filter(settings_audit::Column::SettingTable.eq(t));
        }
        if let Some(id) = setting_id {
            q = q.filter(settings_audit::Column::SettingId.eq(id));
        }
        if let Some(t) = since {
            let stamp: chrono::DateTime<chrono::FixedOffset> = t.into();
            q = q.filter(settings_audit::Column::ChangedAt.gte(stamp));
        }
        let rows = q.all(&self.db).await?;
        rows.into_iter()
            .map(|m| {
                let source = match m.source.as_str() {
                    "http_setter" => SettingsAuditSource::HttpSetter,
                    "companion_setter" => SettingsAuditSource::CompanionSetter,
                    "startup_default" => SettingsAuditSource::StartupDefault,
                    "schema_migration" => SettingsAuditSource::SchemaMigration,
                    other => anyhow::bail!("unknown source: {other}"),
                };
                Ok(crate::audit::SettingsAuditEntry {
                    id: m.id,
                    setting_table: m.setting_table,
                    setting_id: m.setting_id,
                    source,
                    actor: m.actor,
                    before_json: m
                        .before_json
                        .map(|s| serde_json::from_str(&s))
                        .transpose()?,
                    after_json: serde_json::from_str(&m.after_json)?,
                    changed_at: m.changed_at.into(),
                })
            })
            .collect()
    }

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

    #[instrument(skip_all)]
    pub async fn delete_app_setting(&self, key: &str) -> anyhow::Result<()> {
        app_settings::Entity::delete_by_id(key.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn get_osc_settings(&self) -> anyhow::Result<OscSettings> {
        if let Some(model) = osc_settings::Entity::find_by_id(OSC_SETTINGS_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?
        {
            return Ok(osc_model_to_domain(model)?);
        }
        self.insert_osc_settings(
            OscSettingsDraft::default(),
            SettingsAuditSource::StartupDefault,
            "system",
        )
        .await
    }

    #[instrument(skip_all)]
    pub async fn upsert_osc_settings(
        &self,
        draft: &OscSettingsDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<OscSettings> {
        draft.validate().map_err(|err| anyhow!(err))?;
        self.insert_osc_settings(draft.clone(), source, actor).await
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

    async fn insert_osc_settings(
        &self,
        draft: OscSettingsDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<OscSettings> {
        // Capture previous state for audit (None if row missing).
        let before = osc_settings::Entity::find_by_id(OSC_SETTINGS_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?
            .map(|m| osc_model_to_domain(m))
            .transpose()?;
        let before_json = before.as_ref().map(serde_json::to_value).transpose()?;

        let now = Utc::now();
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
            .ok_or_else(|| anyhow!("osc settings missing after upsert"))?;
        let domain = osc_model_to_domain(model)?;
        let after_json = serde_json::to_value(&domain)?;
        self.record_settings_audit(
            "osc_settings",
            OSC_SETTINGS_SINGLETON_ID,
            source,
            actor,
            before_json,
            after_json,
        )
        .await?;
        Ok(domain)
    }

    #[instrument(skip_all)]
    pub async fn get_ableset_settings(&self) -> anyhow::Result<AbleSetSettings> {
        self.ensure_ableset_settings_table().await?;
        if let Some(model) =
            ableset_settings::Entity::find_by_id(ABLESET_SETTINGS_SINGLETON_ID.to_string())
                .one(&self.db)
                .await?
        {
            return Ok(ableset_model_to_domain(model)?);
        }
        self.insert_ableset_settings(
            AbleSetSettingsDraft::default(),
            SettingsAuditSource::StartupDefault,
            "system",
        )
        .await
    }

    #[instrument(skip_all)]
    pub async fn upsert_ableset_settings(
        &self,
        draft: &AbleSetSettingsDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<AbleSetSettings> {
        draft.validate().map_err(|err| anyhow!(err))?;
        self.insert_ableset_settings(draft.clone(), source, actor)
            .await
    }

    async fn insert_ableset_settings(
        &self,
        draft: AbleSetSettingsDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<AbleSetSettings> {
        self.ensure_ableset_settings_table().await?;
        // Capture previous state for audit.
        let before = ableset_settings::Entity::find_by_id(ABLESET_SETTINGS_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?
            .map(|m| ableset_model_to_domain(m))
            .transpose()?;
        let before_json = before.as_ref().map(serde_json::to_value).transpose()?;

        let now = Utc::now();
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
            .ok_or_else(|| anyhow!("ableset settings missing after upsert"))?;
        let domain = ableset_model_to_domain(model)?;
        let after_json = serde_json::to_value(&domain)?;
        self.record_settings_audit(
            "ableset_settings",
            ABLESET_SETTINGS_SINGLETON_ID,
            source,
            actor,
            before_json,
            after_json,
        )
        .await?;
        Ok(domain)
    }

    pub async fn list_resolume_hosts(&self) -> anyhow::Result<Vec<ResolumeHost>> {
        let models = resolume_host::Entity::find()
            .order_by_asc(resolume_host::Column::Label)
            .all(&self.db)
            .await?;
        models.into_iter().map(resolume_model_to_domain).collect()
    }

    pub async fn list_android_stage_displays(&self) -> anyhow::Result<Vec<AndroidStageDisplay>> {
        let models = android_stage_display::Entity::find()
            .order_by_asc(android_stage_display::Column::Label)
            .all(&self.db)
            .await?;
        models
            .into_iter()
            .map(android_stage_display_model_to_domain)
            .collect()
    }

    #[instrument(skip_all)]
    pub async fn create_resolume_host(
        &self,
        draft: &ResolumeHostDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<ResolumeHost> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let id = ResolumeHostId::new();
        let now = Utc::now();
        let model = resolume_host::ActiveModel {
            id: Set(id.to_string()),
            label: Set(draft.label.trim().to_string()),
            host: Set(draft.host.trim().to_string()),
            port: Set(draft.port as i32),
            is_enabled: Set(draft.is_enabled),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };

        resolume_host::Entity::insert(model).exec(&self.db).await?;

        let inserted = resolume_host::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("resolume host missing after insert"))?;
        let host = resolume_model_to_domain(inserted)?;
        let after_json = serde_json::to_value(&host)?;
        self.record_settings_audit(
            "resolume_host",
            &id.to_string(),
            source,
            actor,
            None,
            after_json,
        )
        .await?;
        Ok(host)
    }

    pub async fn create_android_stage_display(
        &self,
        draft: &AndroidStageDisplayDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<AndroidStageDisplay> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let id = AndroidStageDisplayId::new();
        let now = Utc::now();
        let model = android_stage_display::ActiveModel {
            id: Set(id.to_string()),
            label: Set(draft.label.trim().to_string()),
            host: Set(draft.host.trim().to_string()),
            port: Set(draft.port as i32),
            launch_component: Set(draft.launch_component.trim().to_string()),
            is_enabled: Set(draft.is_enabled),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };

        android_stage_display::Entity::insert(model)
            .exec(&self.db)
            .await?;

        let inserted = android_stage_display::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("android stage display missing after insert"))?;
        let display = android_stage_display_model_to_domain(inserted)?;
        let after_json = serde_json::to_value(&display)?;
        self.record_settings_audit(
            "android_stage_display",
            &id.to_string(),
            source,
            actor,
            None,
            after_json,
        )
        .await?;
        Ok(display)
    }

    #[instrument(skip_all)]
    pub async fn update_resolume_host(
        &self,
        id: ResolumeHostId,
        draft: &ResolumeHostDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<ResolumeHost> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let existing = resolume_host::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("resolume host not found"))?;
        let before = resolume_model_to_domain(existing.clone())?;
        let before_json = serde_json::to_value(&before)?;

        let mut model = existing.into_active_model();
        model.label = Set(draft.label.trim().to_string());
        model.host = Set(draft.host.trim().to_string());
        model.port = Set(draft.port as i32);
        model.is_enabled = Set(draft.is_enabled);
        model.updated_at = Set(Utc::now().into());

        let updated = model.update(&self.db).await?;
        let host = resolume_model_to_domain(updated)?;
        let after_json = serde_json::to_value(&host)?;
        self.record_settings_audit(
            "resolume_host",
            &id.to_string(),
            source,
            actor,
            Some(before_json),
            after_json,
        )
        .await?;
        Ok(host)
    }

    pub async fn update_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
        draft: &AndroidStageDisplayDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<AndroidStageDisplay> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let existing = android_stage_display::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("android stage display not found"))?;
        let before = android_stage_display_model_to_domain(existing.clone())?;
        let before_json = serde_json::to_value(&before)?;

        let mut model = existing.into_active_model();
        model.label = Set(draft.label.trim().to_string());
        model.host = Set(draft.host.trim().to_string());
        model.port = Set(draft.port as i32);
        model.launch_component = Set(draft.launch_component.trim().to_string());
        model.is_enabled = Set(draft.is_enabled);
        model.updated_at = Set(Utc::now().into());

        let updated = model.update(&self.db).await?;
        let display = android_stage_display_model_to_domain(updated)?;
        let after_json = serde_json::to_value(&display)?;
        self.record_settings_audit(
            "android_stage_display",
            &id.to_string(),
            source,
            actor,
            Some(before_json),
            after_json,
        )
        .await?;
        Ok(display)
    }

    #[instrument(skip_all)]
    pub async fn delete_resolume_host(
        &self,
        id: ResolumeHostId,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        let existing = resolume_host::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        let before_json = existing
            .map(|m| {
                let host = resolume_model_to_domain(m)?;
                serde_json::to_value(&host).map_err(anyhow::Error::from)
            })
            .transpose()?;

        let result = resolume_host::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("resolume host not found"));
        }
        self.record_settings_audit(
            "resolume_host",
            &id.to_string(),
            source,
            actor,
            before_json,
            serde_json::json!({"deleted": true, "id": id.to_string()}),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        let existing = android_stage_display::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        let before_json = existing
            .map(|m| {
                let display = android_stage_display_model_to_domain(m)?;
                serde_json::to_value(&display).map_err(anyhow::Error::from)
            })
            .transpose()?;

        let result = android_stage_display::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("android stage display not found"));
        }
        self.record_settings_audit(
            "android_stage_display",
            &id.to_string(),
            source,
            actor,
            before_json,
            serde_json::json!({"deleted": true, "id": id.to_string()}),
        )
        .await?;
        Ok(())
    }

    // ── Video Sources ──────────────────────────────────────────────

    pub async fn list_video_sources(&self) -> anyhow::Result<Vec<VideoSource>> {
        let models = video_source::Entity::find()
            .order_by_asc(video_source::Column::Label)
            .all(&self.db)
            .await?;
        models
            .into_iter()
            .map(video_source_model_to_domain)
            .collect()
    }

    pub async fn get_active_video_source(&self) -> anyhow::Result<Option<VideoSource>> {
        let model = video_source::Entity::find()
            .filter(video_source::Column::IsActive.eq(true))
            .one(&self.db)
            .await?;
        model.map(video_source_model_to_domain).transpose()
    }

    #[instrument(skip_all)]
    pub async fn create_video_source(
        &self,
        draft: &VideoSourceDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<VideoSource> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let id = VideoSourceId::new();
        let now = Utc::now();
        let model = video_source::ActiveModel {
            id: Set(id.to_string()),
            label: Set(draft.label.trim().to_string()),
            ndi_name: Set(draft.ndi_name.trim().to_string()),
            is_active: Set(false),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };

        video_source::Entity::insert(model).exec(&self.db).await?;

        let inserted = video_source::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("video source missing after insert"))?;
        let domain = video_source_model_to_domain(inserted)?;
        let after_json = serde_json::to_value(&domain)?;
        self.record_settings_audit(
            "video_source",
            &id.to_string(),
            source,
            actor,
            None,
            after_json,
        )
        .await?;
        Ok(domain)
    }

    #[instrument(skip_all)]
    pub async fn update_video_source(
        &self,
        id: VideoSourceId,
        draft: &VideoSourceDraft,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<VideoSource> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let existing = video_source::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("video source not found"))?;
        let before = video_source_model_to_domain(existing.clone())?;
        let before_json = serde_json::to_value(&before)?;

        let mut model = existing.into_active_model();
        model.label = Set(draft.label.trim().to_string());
        model.ndi_name = Set(draft.ndi_name.trim().to_string());
        model.updated_at = Set(Utc::now().into());

        let updated = model.update(&self.db).await?;
        let domain = video_source_model_to_domain(updated)?;
        let after_json = serde_json::to_value(&domain)?;
        self.record_settings_audit(
            "video_source",
            &id.to_string(),
            source,
            actor,
            Some(before_json),
            after_json,
        )
        .await?;
        Ok(domain)
    }

    #[instrument(skip_all)]
    pub async fn delete_video_source(
        &self,
        id: VideoSourceId,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        let existing = video_source::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        let before_json = existing
            .map(|m| {
                let domain = video_source_model_to_domain(m)?;
                serde_json::to_value(&domain).map_err(anyhow::Error::from)
            })
            .transpose()?;

        let result = video_source::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("video source not found"));
        }
        self.record_settings_audit(
            "video_source",
            &id.to_string(),
            source,
            actor,
            before_json,
            serde_json::json!({"deleted": true, "id": id.to_string()}),
        )
        .await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn activate_video_source(
        &self,
        id: VideoSourceId,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<VideoSource> {
        // Capture before state of target row.
        let existing = video_source::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("video source not found"))?;
        let before = video_source_model_to_domain(existing.clone())?;
        let before_json = serde_json::to_value(&before)?;

        // Deactivate all first
        video_source::Entity::update_many()
            .col_expr(video_source::Column::IsActive, Expr::value(false))
            .col_expr(
                video_source::Column::UpdatedAt,
                Expr::value(Into::<sea_orm::prelude::DateTimeWithTimeZone>::into(
                    Utc::now(),
                )),
            )
            .filter(video_source::Column::IsActive.eq(true))
            .exec(&self.db)
            .await?;

        let mut model = existing.into_active_model();
        model.is_active = Set(true);
        model.updated_at = Set(Utc::now().into());

        let updated = model.update(&self.db).await?;
        let domain = video_source_model_to_domain(updated)?;
        let after_json = serde_json::to_value(&domain)?;
        self.record_settings_audit(
            "video_source",
            &id.to_string(),
            source,
            actor,
            Some(before_json),
            after_json,
        )
        .await?;
        Ok(domain)
    }

    pub async fn deactivate_all_video_sources(
        &self,
        source: SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<()> {
        // Capture before state — list of active sources.
        let active_rows: Vec<_> = video_source::Entity::find()
            .filter(video_source::Column::IsActive.eq(true))
            .all(&self.db)
            .await?;
        let before_list: Vec<serde_json::Value> = active_rows
            .iter()
            .cloned()
            .map(|m| {
                let domain = video_source_model_to_domain(m)?;
                serde_json::to_value(&domain).map_err(anyhow::Error::from)
            })
            .collect::<anyhow::Result<_>>()?;

        video_source::Entity::update_many()
            .col_expr(video_source::Column::IsActive, Expr::value(false))
            .col_expr(
                video_source::Column::UpdatedAt,
                Expr::value(Into::<sea_orm::prelude::DateTimeWithTimeZone>::into(
                    Utc::now(),
                )),
            )
            .filter(video_source::Column::IsActive.eq(true))
            .exec(&self.db)
            .await?;

        if !active_rows.is_empty() {
            self.record_settings_audit(
                "video_source",
                "deactivate_all",
                source,
                actor,
                Some(serde_json::Value::Array(before_list)),
                serde_json::json!({"deactivated_all": true}),
            )
            .await?;
        }
        Ok(())
    }

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
                        stage_state::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;

        Ok(())
    }

    pub async fn get_timers_state(&self) -> anyhow::Result<Option<TimersState>> {
        let model = timers::Entity::find_by_id(TIMERS_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?;
        model
            .map(|record| timers_model_to_state(record).map_err(anyhow::Error::from))
            .transpose()
    }

    #[instrument(skip_all)]
    pub async fn upsert_timers_state(&self, state: &TimersState) -> anyhow::Result<()> {
        let now = Utc::now();
        let model = timers::ActiveModel {
            id: Set(TIMERS_SINGLETON_ID.to_string()),
            countdown_target: Set(state.countdown.target.into()),
            countdown_state: Set(timer_state_to_string(state.countdown.state)),
            preach_state: Set(timer_state_to_string(state.preach.state)),
            preach_started_at: Set(state.preach.started_at().map(Into::into)),
            preach_accumulated_seconds: Set(state.preach.accumulated_duration().num_seconds()),
            preach_limit_seconds: Set(state.preach.limit_seconds().map(|s| s as i64)),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };

        timers::Entity::insert(model)
            .on_conflict(
                OnConflict::column(timers::Column::Id)
                    .update_columns([
                        timers::Column::CountdownTarget,
                        timers::Column::CountdownState,
                        timers::Column::PreachState,
                        timers::Column::PreachStartedAt,
                        timers::Column::PreachAccumulatedSeconds,
                        timers::Column::PreachLimitSeconds,
                        timers::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;

        Ok(())
    }
}
