mod helpers;
mod library;
mod playlist;
mod search;
#[cfg(test)]
mod tests;

pub use helpers::RepositoryError;
use helpers::{
    ableset_model_to_domain, android_stage_display_model_to_domain, osc_model_to_domain,
    parse_uuid, resolume_model_to_domain, stage_state_model_to_state, timer_state_to_string,
    timers_model_to_state, to_domain_passage, to_domain_slide, to_domain_translation,
    velocity_mode_to_string,
};

use crate::entities::{
    ableset_settings, android_stage_display, app_settings, bible_passage, bible_translation,
    osc_settings, presentation as presentation_entity, resolume_host, slide as slide_entity,
    stage_state, timers,
};
use anyhow::{anyhow, Context};
use chrono::Utc;
use presenter_core::{
    bible::BibleIngestionBatch, search::fold_query, AbleSetSettings, AbleSetSettingsDraft,
    AndroidStageDisplay, AndroidStageDisplayDraft, AndroidStageDisplayId, BiblePassage,
    BibleReference, BibleTranslation, LibraryId, OscSettings, OscSettingsDraft, Presentation,
    PresentationId, ResolumeHost, ResolumeHostDraft, ResolumeHostId, Slide, SlideContent, SlideId,
    StageState, TimersState,
};
use presenter_migration::{Migrator, MigratorTrait};
use sea_orm::{
    sea_query::{Expr, OnConflict},
    ActiveModelTrait, ColumnTrait, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Schema, Set, TransactionTrait,
};
use std::fmt::Debug;
use tracing::instrument;
use uuid::Uuid;

const TIMERS_SINGLETON_ID: &str = "timers";
const STAGE_STATE_SINGLETON_ID: &str = "stage-state";
const OSC_SETTINGS_SINGLETON_ID: &str = "osc";
const ABLESET_SETTINGS_SINGLETON_ID: &str = "ableset";
const BIBLE_INSERT_CHUNK: usize = 500;

#[derive(Debug, Clone)]
pub struct Repository {
    db: DatabaseConnection,
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

    pub fn connection(&self) -> &DatabaseConnection {
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
    pub async fn create_presentation(
        &self,
        library_id: LibraryId,
        name: &str,
    ) -> anyhow::Result<(LibraryId, String, Presentation)> {
        let presentation_uuid = Uuid::new_v4();
        let library_uuid = library_id.to_string();
        let txn = self.db.begin().await?;

        presentation_entity::Entity::insert(presentation_entity::ActiveModel {
            id: Set(presentation_uuid.to_string()),
            library_id: Set(library_uuid.clone()),
            name: Set(name.to_string()),
            search_name: Set(fold_query(name)),
            created_at: Set(Utc::now().into()),
        })
        .exec(&txn)
        .await?;

        slide_entity::Entity::insert(slide_entity::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            presentation_id: Set(presentation_uuid.to_string()),
            position: Set(0),
            main_text: Set(String::new()),
            main_text_search: Set(String::new()),
            translation_text: Set(String::new()),
            translation_text_search: Set(String::new()),
            stage_text: Set(String::new()),
            stage_text_search: Set(String::new()),
            group_name: Set(None),
            created_at: Set(Utc::now().into()),
            metadata_json: Set(None),
        })
        .exec(&txn)
        .await?;

        txn.commit().await?;

        let detail = self
            .fetch_presentation_detail(PresentationId::from_uuid(presentation_uuid))
            .await?;

        detail.ok_or_else(|| anyhow!("failed to load newly created presentation"))
    }

    #[instrument(skip_all)]
    pub async fn rename_presentation(
        &self,
        presentation_id: PresentationId,
        name: &str,
    ) -> anyhow::Result<()> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("presentation name cannot be empty"));
        }
        let id = presentation_id.to_string();
        let result = presentation_entity::Entity::update_many()
            .col_expr(presentation_entity::Column::Name, Expr::value(trimmed))
            .col_expr(
                presentation_entity::Column::SearchName,
                Expr::value(fold_query(trimmed)),
            )
            .filter(presentation_entity::Column::Id.eq(id.clone()))
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("presentation not found"));
        }
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn purge_presentation_content(&self) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;

        slide_entity::Entity::delete_many().exec(&txn).await?;
        presentation_entity::Entity::delete_many()
            .exec(&txn)
            .await?;
        crate::entities::library::Entity::delete_many()
            .exec(&txn)
            .await?;
        stage_state::Entity::delete_by_id(STAGE_STATE_SINGLETON_ID.to_string())
            .exec(&txn)
            .await?;

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn fetch_presentation_detail(
        &self,
        presentation_id: PresentationId,
    ) -> anyhow::Result<Option<(LibraryId, String, Presentation)>> {
        let pres_model = presentation_entity::Entity::find_by_id(presentation_id.to_string())
            .one(&self.db)
            .await?;
        let Some(pres_model) = pres_model else {
            return Ok(None);
        };

        let slides = slide_entity::Entity::find()
            .filter(slide_entity::Column::PresentationId.eq(pres_model.id.clone()))
            .order_by_asc(slide_entity::Column::Position)
            .all(&self.db)
            .await?;

        let slide_models = slides
            .into_iter()
            .map(to_domain_slide)
            .collect::<Result<Vec<_>, RepositoryError>>()?;

        let presentation = Presentation::new(pres_model.name.clone(), slide_models)?
            .with_id(PresentationId::from_uuid(parse_uuid(&pres_model.id)?));

        let library_id = LibraryId::from_uuid(parse_uuid(&pres_model.library_id)?);
        let library_name =
            crate::entities::library::Entity::find_by_id(pres_model.library_id.clone())
                .one(&self.db)
                .await?
                .map(|lib| lib.name)
                .unwrap_or_default();

        Ok(Some((library_id, library_name, presentation)))
    }

    #[instrument(skip_all)]
    pub async fn fetch_first_presentation_detail(
        &self,
    ) -> anyhow::Result<Option<(LibraryId, String, Presentation)>> {
        let presentation = presentation_entity::Entity::find()
            .order_by_asc(presentation_entity::Column::CreatedAt)
            .one(&self.db)
            .await?;
        let Some(model) = presentation else {
            return Ok(None);
        };
        let presentation_id = PresentationId::from_uuid(parse_uuid(&model.id)?);
        self.fetch_presentation_detail(presentation_id).await
    }

    #[instrument(skip_all)]
    pub async fn update_slide_content(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
        content: &SlideContent,
    ) -> anyhow::Result<()> {
        let main_text = content.main.value().to_owned();
        let translation_text = content.translation.value().to_owned();
        let stage_text = content.stage.value().to_owned();
        let main_search = fold_query(content.main.value());
        let translation_search = fold_query(content.translation.value());
        let stage_search = fold_query(content.stage.value());

        let result = slide_entity::Entity::update_many()
            .col_expr(slide_entity::Column::MainText, Expr::value(main_text))
            .col_expr(
                slide_entity::Column::MainTextSearch,
                Expr::value(main_search),
            )
            .col_expr(
                slide_entity::Column::TranslationText,
                Expr::value(translation_text),
            )
            .col_expr(
                slide_entity::Column::TranslationTextSearch,
                Expr::value(translation_search),
            )
            .col_expr(slide_entity::Column::StageText, Expr::value(stage_text))
            .col_expr(
                slide_entity::Column::StageTextSearch,
                Expr::value(stage_search),
            )
            .col_expr(
                slide_entity::Column::GroupName,
                Expr::value(content.group.as_ref().map(|group| group.name().to_owned())),
            )
            .filter(slide_entity::Column::Id.eq(slide_id.to_string()))
            .filter(slide_entity::Column::PresentationId.eq(presentation_id.to_string()))
            .exec(&self.db)
            .await?;

        if result.rows_affected == 0 {
            return Err(anyhow!(
                "slide {} not found in presentation {}",
                slide_id,
                presentation_id
            ));
        }

        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn replace_presentation_slides(
        &self,
        presentation_id: PresentationId,
        slides: &[Slide],
    ) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;

        slide_entity::Entity::delete_many()
            .filter(slide_entity::Column::PresentationId.eq(presentation_id.to_string()))
            .exec(&txn)
            .await?;

        for (index, slide) in slides.iter().enumerate() {
            let active = slide_entity::ActiveModel {
                id: Set(slide.id.to_string()),
                presentation_id: Set(presentation_id.to_string()),
                position: Set(index as i32),
                main_text: Set(slide.content.main.value().to_owned()),
                main_text_search: Set(fold_query(slide.content.main.value())),
                translation_text: Set(slide.content.translation.value().to_owned()),
                translation_text_search: Set(fold_query(slide.content.translation.value())),
                stage_text: Set(slide.content.stage.value().to_owned()),
                stage_text_search: Set(fold_query(slide.content.stage.value())),
                group_name: Set(slide
                    .content
                    .group
                    .as_ref()
                    .map(|group| group.name().to_owned())),
                created_at: Set(Utc::now().into()),
                metadata_json: Set(slide
                    .metadata
                    .as_ref()
                    .map(|m| serde_json::to_string(m).ok())
                    .flatten()),
            };
            slide_entity::Entity::insert(active).exec(&txn).await?;
        }

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn replace_bible_translation_passages(
        &self,
        batch: &BibleIngestionBatch,
    ) -> anyhow::Result<()> {
        let (translation, passages) = batch.clone().into_parts();
        let txn = self.db.begin().await?;

        bible_translation::Entity::delete_by_id(translation.code.clone())
            .exec(&txn)
            .await?;

        let translation_model = bible_translation::ActiveModel {
            code: Set(translation.code.clone()),
            name: Set(translation.name.clone()),
            language: Set(translation.language.clone()),
            show_in_dashboard: Set(true),
            source: Set(translation.source.clone()),
            created_at: Set(Utc::now().into()),
        };

        bible_translation::Entity::insert(translation_model)
            .exec(&txn)
            .await?;

        let mut chunk = Vec::with_capacity(BIBLE_INSERT_CHUNK);
        for passage in passages {
            let reference = &passage.reference;
            let model = bible_passage::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                translation_code: Set(translation.code.clone()),
                book: Set(reference.book.clone()),
                book_code: Set(reference.book_code.clone().unwrap_or_default()),
                book_number: Set(reference.book_number.unwrap_or(0) as i32),
                chapter: Set(reference.chapter as i32),
                verse_start: Set(reference.verse_start as i32),
                verse_end: Set(reference.verse_end as i32),
                content: Set(passage.text.clone()),
                created_at: Set(Utc::now().into()),
            };

            chunk.push(model);
            if chunk.len() == BIBLE_INSERT_CHUNK {
                let to_insert = std::mem::take(&mut chunk);
                bible_passage::Entity::insert_many(to_insert)
                    .exec(&txn)
                    .await?;
            }
        }

        if !chunk.is_empty() {
            bible_passage::Entity::insert_many(chunk).exec(&txn).await?;
        }

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn list_bible_translations(&self) -> anyhow::Result<Vec<BibleTranslation>> {
        let models = bible_translation::Entity::find()
            .order_by_asc(bible_translation::Column::Name)
            .all(&self.db)
            .await?;
        Ok(models.into_iter().map(to_domain_translation).collect())
    }

    #[instrument(skip_all)]
    pub async fn search_bible_passages(
        &self,
        translation_code: &str,
        query: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<BiblePassage>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let translation = bible_translation::Entity::find_by_id(translation_code.to_string())
            .one(&self.db)
            .await?;
        let Some(translation) = translation else {
            return Ok(Vec::new());
        };

        let pattern = format!("%{}%", query);
        let rows = bible_passage::Entity::find()
            .filter(bible_passage::Column::TranslationCode.eq(translation_code.to_string()))
            .filter(bible_passage::Column::Content.like(pattern))
            .order_by_asc(bible_passage::Column::Book)
            .order_by_asc(bible_passage::Column::Chapter)
            .order_by_asc(bible_passage::Column::VerseStart)
            .limit(limit as u64)
            .all(&self.db)
            .await?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            results.push(to_domain_passage(row, translation.clone())?);
        }
        Ok(results)
    }

    #[instrument(skip_all)]
    pub async fn find_bible_passage(
        &self,
        translation_code: &str,
        reference: &BibleReference,
    ) -> anyhow::Result<Option<BiblePassage>> {
        let translation = bible_translation::Entity::find_by_id(translation_code.to_string())
            .one(&self.db)
            .await?;
        let Some(translation) = translation else {
            return Ok(None);
        };

        let passage = bible_passage::Entity::find()
            .filter(bible_passage::Column::TranslationCode.eq(translation_code.to_string()))
            .filter(bible_passage::Column::Book.eq(reference.book.clone()))
            .filter(bible_passage::Column::Chapter.eq(reference.chapter as i32))
            .filter(bible_passage::Column::VerseStart.eq(reference.verse_start as i32))
            .filter(bible_passage::Column::VerseEnd.eq(reference.verse_end as i32))
            .one(&self.db)
            .await?;

        Ok(passage
            .map(|model| to_domain_passage(model, translation.clone()))
            .transpose()?)
    }

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
        draft.validate().map_err(|err| anyhow!(err))?;
        self.insert_osc_settings(draft.clone()).await
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

    async fn insert_osc_settings(&self, draft: OscSettingsDraft) -> anyhow::Result<OscSettings> {
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
        Ok(osc_model_to_domain(model)?)
    }

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
                active.updated_at = sea_orm::ActiveValue::set(Utc::now().into());
                active.update(&self.db).await?;
                model =
                    ableset_settings::Entity::find_by_id(ABLESET_SETTINGS_SINGLETON_ID.to_string())
                        .one(&self.db)
                        .await?
                        .ok_or_else(|| anyhow!("ableset settings missing after migration"))?;
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
        draft.validate().map_err(|err| anyhow!(err))?;
        self.insert_ableset_settings(draft.clone()).await
    }

    async fn insert_ableset_settings(
        &self,
        draft: AbleSetSettingsDraft,
    ) -> anyhow::Result<AbleSetSettings> {
        self.ensure_ableset_settings_table().await?;
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

        Ok(ableset_model_to_domain(model)?)
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
        resolume_model_to_domain(inserted)
    }

    pub async fn create_android_stage_display(
        &self,
        draft: &AndroidStageDisplayDraft,
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
        android_stage_display_model_to_domain(inserted)
    }

    #[instrument(skip_all)]
    pub async fn update_resolume_host(
        &self,
        id: ResolumeHostId,
        draft: &ResolumeHostDraft,
    ) -> anyhow::Result<ResolumeHost> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let existing = resolume_host::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("resolume host not found"))?;

        let mut model = existing.into_active_model();
        model.label = Set(draft.label.trim().to_string());
        model.host = Set(draft.host.trim().to_string());
        model.port = Set(draft.port as i32);
        model.is_enabled = Set(draft.is_enabled);
        model.updated_at = Set(Utc::now().into());

        let updated = model.update(&self.db).await?;
        resolume_model_to_domain(updated)
    }

    pub async fn update_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
        draft: &AndroidStageDisplayDraft,
    ) -> anyhow::Result<AndroidStageDisplay> {
        draft.validate().map_err(|err| anyhow!(err))?;
        let existing = android_stage_display::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("android stage display not found"))?;

        let mut model = existing.into_active_model();
        model.label = Set(draft.label.trim().to_string());
        model.host = Set(draft.host.trim().to_string());
        model.port = Set(draft.port as i32);
        model.launch_component = Set(draft.launch_component.trim().to_string());
        model.is_enabled = Set(draft.is_enabled);
        model.updated_at = Set(Utc::now().into());

        let updated = model.update(&self.db).await?;
        android_stage_display_model_to_domain(updated)
    }

    #[instrument(skip_all)]
    pub async fn delete_resolume_host(&self, id: ResolumeHostId) -> anyhow::Result<()> {
        let result = resolume_host::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("resolume host not found"));
        }
        Ok(())
    }

    pub async fn delete_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
    ) -> anyhow::Result<()> {
        let result = android_stage_display::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("android stage display not found"));
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
            updated_at: Set(now.into()),
        };

        stage_state::Entity::insert(model)
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
                        timers::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;

        Ok(())
    }
}
