mod android_stage;
mod audit;
mod bible;
mod group_color;
mod library;
mod library_sync;
#[cfg(test)]
mod library_sync_tests;
mod playlist;
mod presentation;
#[cfg(test)]
mod presentation_atomicity_tests;
#[cfg(test)]
mod presentation_copy_tests;
mod resolume;
mod search;
#[cfg(test)]
mod search_trash_tests;
mod slide_stage_layout;
mod stage_state;
mod sync;
mod sync_apply;
mod sync_apply_library;
#[cfg(test)]
mod sync_apply_library_tests;
#[cfg(test)]
mod sync_apply_review_tests;
mod sync_apply_tombstone_cleanup;
#[cfg(test)]
mod sync_restore_review_tests;
#[cfg(test)]
mod sync_test_support;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod sync_trash_tests;
#[cfg(test)]
mod tests;
mod util;
mod video_source;

pub use library_sync::SyncLibraryManifestRow;
pub use sync::{SyncManifestRow, SyncPresentation, TrashedPresentation};
pub use sync_apply::{sync_should_apply, SyncApplyOutcome, PRUNE_HORIZON};
// #584: exposed so the server crate can `downcast_ref` a repository refusal
// to its typed variant instead of matching on the `Display` string.
pub use util::RepositoryError;

use util::{
    ableset_model_to_domain, osc_model_to_domain, timer_state_to_string, timers_model_to_state,
    velocity_mode_to_string,
};

use crate::audit::SettingsAuditSource;
use crate::entities::{ableset_settings, app_settings, osc_settings, timers};
use anyhow::{anyhow, Context};
use chrono::Utc;
use presenter_core::{
    AbleSetSettings, AbleSetSettingsDraft, OscSettings, OscSettingsDraft, TimersState,
};
use presenter_migration::{Migrator, MigratorTrait};
use sea_orm::Statement;
use sea_orm::{
    sea_query::OnConflict, ConnectionTrait, Database, DatabaseConnection, EntityTrait, Schema, Set,
    TransactionTrait,
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
        Self::apply_sqlite_pragmas(&db).await?;
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
            "PRAGMA foreign_keys = ON",
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
        // Wrap the settings upsert + audit insert in a single transaction so
        // a mid-flight failure cannot leave the row mutated without an audit
        // entry (or vice versa).
        let txn = self.db.begin().await?;

        // Capture previous state for audit (None if row missing).
        let before = osc_settings::Entity::find_by_id(OSC_SETTINGS_SINGLETON_ID.to_string())
            .one(&txn)
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
            .exec(&txn)
            .await?;

        let model = osc_settings::Entity::find_by_id(OSC_SETTINGS_SINGLETON_ID.to_string())
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("osc settings missing after upsert"))?;
        let domain = osc_model_to_domain(model)?;
        let after_json = serde_json::to_value(&domain)?;
        Self::record_settings_audit_on(
            &txn,
            "osc_settings",
            OSC_SETTINGS_SINGLETON_ID,
            source,
            actor,
            before_json,
            after_json,
        )
        .await?;

        txn.commit().await?;
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
        // `ensure_ableset_settings_table` is idempotent DDL — kept outside
        // the transaction to avoid serializing schema changes with row
        // writes.
        self.ensure_ableset_settings_table().await?;

        // Row + audit write atomically inside one transaction.
        let txn = self.db.begin().await?;

        // Capture previous state for audit.
        let before =
            ableset_settings::Entity::find_by_id(ABLESET_SETTINGS_SINGLETON_ID.to_string())
                .one(&txn)
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
            .exec(&txn)
            .await?;

        let model = ableset_settings::Entity::find_by_id(ABLESET_SETTINGS_SINGLETON_ID.to_string())
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("ableset settings missing after upsert"))?;
        let domain = ableset_model_to_domain(model)?;
        let after_json = serde_json::to_value(&domain)?;
        Self::record_settings_audit_on(
            &txn,
            "ableset_settings",
            ABLESET_SETTINGS_SINGLETON_ID,
            source,
            actor,
            before_json,
            after_json,
        )
        .await?;

        txn.commit().await?;
        Ok(domain)
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
