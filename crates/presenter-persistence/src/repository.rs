use crate::entities::{
    ableset_settings, bible_passage, bible_translation, library, library_favorite, osc_settings,
    playlist, playlist_entry, playlist_favorite, presentation as presentation_entity,
    resolume_host, slide as slide_entity, stage_state, timers,
};
use anyhow::{anyhow, Context};
use chrono::{DateTime, Duration, Utc};
use presenter_core::{
    bible::BibleIngestionBatch,
    playlist::{MidiBinding, PlaylistEntryKind},
    search::{fold_query, query_tokens},
    AbleSetSettings, AbleSetSettingsDraft, BiblePassage, BibleReference, BibleTranslation,
    CountdownTimer, Library, LibraryId, LibrarySummary, OscSettings, OscSettingsDraft, Playlist,
    PlaylistEntry, PlaylistEntryId, PlaylistId, PreachTimer, Presentation, PresentationId,
    PresentationSummary, ResolumeHost, ResolumeHostDraft, ResolumeHostId, SearchMatchField,
    SearchResult, SearchResultKind, Slide, SlideContent, SlideGroup, SlideId, SlideText,
    StageState, TimerState, TimersState, VelocityMode,
};
use presenter_migration::{Migrator, MigratorTrait};
use sea_orm::{
    sea_query::{Expr, OnConflict},
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, Database, DatabaseConnection,
    EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Schema,
    Set, TransactionTrait,
};
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
};
use thiserror::Error;
use tracing::instrument;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Repository {
    db: DatabaseConnection,
}

const TIMERS_SINGLETON_ID: &str = "timers";
const STAGE_STATE_SINGLETON_ID: &str = "stage-state";
const OSC_SETTINGS_SINGLETON_ID: &str = "osc";
const ABLESET_SETTINGS_SINGLETON_ID: &str = "ableset";

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
    pub async fn upsert_library(&self, library: &Library) -> anyhow::Result<()> {
        let mut txn = self.db.begin().await?;

        library::Entity::delete_by_id(library.id.to_string())
            .exec(&mut txn)
            .await?;

        let lib_model = library::ActiveModel {
            id: Set(library.id.to_string()),
            name: Set(library.name.clone()),
            search_name: Set(fold_query(&library.name)),
            created_at: Set(Utc::now().into()),
        };
        library::Entity::insert(lib_model).exec(&mut txn).await?;

        for presentation in &library.presentations {
            let pres_model = presentation_entity::ActiveModel {
                id: Set(presentation.id.to_string()),
                library_id: Set(library.id.to_string()),
                name: Set(presentation.name.clone()),
                search_name: Set(fold_query(&presentation.name)),
                created_at: Set(Utc::now().into()),
            };
            presentation_entity::Entity::insert(pres_model)
                .exec(&mut txn)
                .await?;

            for slide in &presentation.slides {
                let slide_model = slide_entity::ActiveModel {
                    id: Set(slide.id.to_string()),
                    presentation_id: Set(presentation.id.to_string()),
                    position: Set(slide.order as i32),
                    main_text: Set(slide.content.main.value().to_owned()),
                    main_text_search: Set(fold_query(slide.content.main.value())),
                    translation_text: Set(slide.content.translation.value().to_owned()),
                    translation_text_search: Set(fold_query(slide.content.translation.value())),
                    stage_text: Set(slide.content.stage.value().to_owned()),
                    stage_text_search: Set(fold_query(slide.content.stage.value())),
                    group_name: Set(slide.content.group.as_ref().map(|g| g.name().to_owned())),
                    created_at: Set(Utc::now().into()),
                };
                slide_entity::Entity::insert(slide_model)
                    .exec(&mut txn)
                    .await?;
            }
        }

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn create_library(&self, name: &str) -> anyhow::Result<Library> {
        let mut txn = self.db.begin().await?;
        let id = LibraryId::new();

        let model = library::ActiveModel {
            id: Set(id.to_string()),
            name: Set(name.to_string()),
            search_name: Set(fold_query(name)),
            created_at: Set(Utc::now().into()),
        };

        library::Entity::insert(model).exec(&mut txn).await?;
        txn.commit().await?;

        let library = Library::new(name.to_string(), Vec::new())?.with_id(id);
        Ok(library)
    }

    #[instrument(skip_all)]
    pub async fn rename_library(&self, library_id: LibraryId, name: &str) -> anyhow::Result<()> {
        let id = library_id.to_string();
        let mut model = library::Entity::find_by_id(id.clone())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("library not found"))?
            .into_active_model();
        model.name = Set(name.to_string());
        model.search_name = Set(fold_query(name));
        model.update(&self.db).await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn delete_library(&self, library_id: LibraryId) -> anyhow::Result<()> {
        let id = library_id.to_string();
        let result = library::Entity::delete_by_id(id).exec(&self.db).await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("library not found"));
        }
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
        let mut txn = self.db.begin().await?;

        presentation_entity::Entity::insert(presentation_entity::ActiveModel {
            id: Set(presentation_uuid.to_string()),
            library_id: Set(library_uuid.clone()),
            name: Set(name.to_string()),
            search_name: Set(fold_query(name)),
            created_at: Set(Utc::now().into()),
        })
        .exec(&mut txn)
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
        })
        .exec(&mut txn)
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
        let mut txn = self.db.begin().await?;

        slide_entity::Entity::delete_many().exec(&mut txn).await?;
        presentation_entity::Entity::delete_many()
            .exec(&mut txn)
            .await?;
        library::Entity::delete_many().exec(&mut txn).await?;
        stage_state::Entity::delete_by_id(STAGE_STATE_SINGLETON_ID.to_string())
            .exec(&mut txn)
            .await?;

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn fetch_libraries(&self) -> anyhow::Result<Vec<Library>> {
        let libraries = library::Entity::find()
            .order_by_asc(library::Column::Name)
            .all(&self.db)
            .await?;

        let mut results = Vec::with_capacity(libraries.len());
        for lib in libraries {
            let presentations = presentation_entity::Entity::find()
                .filter(presentation_entity::Column::LibraryId.eq(lib.id.clone()))
                .order_by_asc(presentation_entity::Column::Name)
                .all(&self.db)
                .await?;

            let mut presentation_models = Vec::with_capacity(presentations.len());
            for pres in presentations {
                let slides = slide_entity::Entity::find()
                    .filter(slide_entity::Column::PresentationId.eq(pres.id.clone()))
                    .order_by_asc(slide_entity::Column::Position)
                    .all(&self.db)
                    .await?;

                let slide_models = slides
                    .into_iter()
                    .map(|slide| to_domain_slide(slide))
                    .collect::<Result<Vec<_>, RepositoryError>>()?;

                let presentation = Presentation::new(pres.name.clone(), slide_models)?
                    .with_id(PresentationId::from_uuid(parse_uuid(&pres.id)?));
                presentation_models.push(presentation);
            }

            let library_domain = Library::new(lib.name.clone(), presentation_models)?
                .with_id(LibraryId::from_uuid(parse_uuid(&lib.id)?));
            results.push(library_domain);
        }

        Ok(results)
    }

    #[instrument(skip_all)]
    pub async fn list_library_favorites(&self) -> anyhow::Result<Vec<LibraryId>> {
        let favorites = library_favorite::Entity::find()
            .order_by_asc(library_favorite::Column::LibraryId)
            .all(&self.db)
            .await?;
        let mut ids = Vec::with_capacity(favorites.len());
        for fav in favorites {
            let uuid = parse_uuid(&fav.library_id)?;
            ids.push(LibraryId::from_uuid(uuid));
        }
        Ok(ids)
    }

    #[instrument(skip_all)]
    pub async fn set_library_favorite(
        &self,
        library_id: LibraryId,
        favorite: bool,
    ) -> anyhow::Result<()> {
        let mut txn = self.db.begin().await?;
        let id_string = library_id.to_string();

        if favorite {
            library_favorite::Entity::insert(library_favorite::ActiveModel {
                library_id: Set(id_string.clone()),
            })
            .on_conflict(
                sea_orm::sea_query::OnConflict::column(library_favorite::Column::LibraryId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(&mut txn)
            .await?;
        } else {
            library_favorite::Entity::delete_by_id(id_string.clone())
                .exec(&mut txn)
                .await?;
        }

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn list_library_summaries(
        &self,
        filter: Option<&str>,
    ) -> anyhow::Result<Vec<LibrarySummary>> {
        let mut query = library::Entity::find();
        if let Some(filter) = filter {
            let pattern = format!("%{}%", filter);
            query = query.filter(library::Column::Name.like(pattern));
        }

        let libraries = query
            .order_by_asc(library::Column::Name)
            .all(&self.db)
            .await?;

        let mut summaries = Vec::with_capacity(libraries.len());
        for lib in libraries {
            let presentation_models = presentation_entity::Entity::find()
                .filter(presentation_entity::Column::LibraryId.eq(lib.id.clone()))
                .order_by_asc(presentation_entity::Column::Name)
                .all(&self.db)
                .await?;

            let mut presentation_summaries = Vec::with_capacity(presentation_models.len());
            for pres in &presentation_models {
                let pres_id = PresentationId::from_uuid(parse_uuid(&pres.id)?);
                presentation_summaries.push(PresentationSummary::new(pres_id, pres.name.clone()));
            }

            let library_id = LibraryId::from_uuid(parse_uuid(&lib.id)?);
            let summary = LibrarySummary::new(
                library_id,
                lib.name.clone(),
                presentation_summaries.len(),
                presentation_summaries,
            );
            summaries.push(summary);
        }

        Ok(summaries)
    }

    #[instrument(skip_all)]
    pub async fn search_presenter(
        &self,
        query: &str,
        limit: u64,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let tokens = query_tokens(trimmed);
        let has_tokens = !tokens.is_empty();
        let folded = fold_query(trimmed);
        let has_folded = !folded.is_empty();
        let query_lower = trimmed.to_lowercase();

        let cap = limit.clamp(1, 100) as usize;
        let mut results = Vec::with_capacity(cap);
        let mut library_names: HashMap<String, String> = HashMap::new();
        let mut seen_library_ids: HashSet<String> = HashSet::new();
        let mut seen_presentation_ids: HashSet<String> = HashSet::new();
        let mut seen_slide_ids: HashSet<String> = HashSet::new();
        let mut matched_library_ids: HashSet<String> = HashSet::new();

        let mut remaining = cap;

        if remaining == 0 {
            return Ok(results);
        }

        let mut library_condition = Condition::any();
        if !trimmed.is_empty() {
            library_condition = library_condition.add(library::Column::Name.contains(trimmed));
        }
        if has_tokens {
            let mut token_condition = Condition::any();
            for token in &tokens {
                token_condition =
                    token_condition.add(library::Column::SearchName.contains(token.clone()));
            }
            library_condition = library_condition.add(token_condition);
        }

        let library_models = library::Entity::find()
            .filter(library_condition)
            .order_by_asc(library::Column::Name)
            .limit(remaining as u64)
            .all(&self.db)
            .await?;

        for model in library_models {
            if !seen_library_ids.insert(model.id.clone()) {
                continue;
            }
            let tokens_in_name = if has_tokens {
                let haystack = fold_query(&model.name);
                tokens
                    .iter()
                    .filter(|token| haystack.contains(*token))
                    .count()
            } else {
                0
            };
            if tokens_in_name > 0 {
                matched_library_ids.insert(model.id.clone());
            }
            if has_tokens && tokens_in_name < tokens.len() {
                continue;
            }
            let library_id = LibraryId::from_uuid(parse_uuid(&model.id)?);
            library_names.insert(model.id.clone(), model.name.clone());
            results.push(SearchResult {
                kind: SearchResultKind::Library,
                library_id,
                library_name: model.name.clone(),
                presentation_id: None,
                presentation_name: None,
                slide_id: None,
                match_field: SearchMatchField::LibraryName,
                snippet: None,
            });
            if results.len() >= cap {
                return Ok(results);
            }
        }

        let matched_library_vec: Vec<String> = matched_library_ids.iter().cloned().collect();
        let mut matched_presentation_ids: HashSet<String> = HashSet::new();
        if !matched_library_vec.is_empty() {
            let matched_presentations = presentation_entity::Entity::find()
                .filter(presentation_entity::Column::LibraryId.is_in(matched_library_vec.clone()))
                .all(&self.db)
                .await?;
            for model in matched_presentations {
                matched_presentation_ids.insert(model.id.clone());
            }
        }
        let matched_presentation_vec: Vec<String> =
            matched_presentation_ids.iter().cloned().collect();

        if results.len() < cap {
            remaining = cap - results.len();
            if remaining > 0 {
                let mut presentation_condition = Condition::any();
                if !trimmed.is_empty() {
                    presentation_condition = presentation_condition
                        .add(presentation_entity::Column::Name.contains(trimmed));
                }
                if has_tokens {
                    let mut token_branch = Condition::all();
                    for token in &tokens {
                        token_branch = token_branch
                            .add(presentation_entity::Column::SearchName.contains(token.clone()));
                    }
                    presentation_condition = presentation_condition.add(token_branch);
                }
                if !matched_library_vec.is_empty() {
                    presentation_condition = presentation_condition.add(
                        presentation_entity::Column::LibraryId.is_in(matched_library_vec.clone()),
                    );
                }

                let presentation_rows = presentation_entity::Entity::find()
                    .filter(presentation_condition)
                    .order_by_asc(presentation_entity::Column::Name)
                    .limit(remaining as u64)
                    .find_also_related(library::Entity)
                    .all(&self.db)
                    .await?;

                for (presentation_model, library_model_opt) in presentation_rows {
                    let library_model = match library_model_opt {
                        Some(model) => model,
                        None => continue,
                    };
                    if !seen_presentation_ids.insert(presentation_model.id.clone()) {
                        continue;
                    }
                    if has_tokens {
                        let combined = fold_query(&format!(
                            "{} {}",
                            presentation_model.name, library_model.name
                        ));
                        if !tokens.iter().all(|token| combined.contains(token)) {
                            continue;
                        }
                    }
                    let presentation_id =
                        PresentationId::from_uuid(parse_uuid(&presentation_model.id)?);
                    let library_uuid = parse_uuid(&library_model.id)?;
                    let library_id = LibraryId::from_uuid(library_uuid);
                    library_names
                        .entry(library_model.id.clone())
                        .or_insert_with(|| library_model.name.clone());
                    results.push(SearchResult {
                        kind: SearchResultKind::Presentation,
                        library_id,
                        library_name: library_model.name.clone(),
                        presentation_id: Some(presentation_id),
                        presentation_name: Some(presentation_model.name.clone()),
                        slide_id: None,
                        match_field: SearchMatchField::PresentationName,
                        snippet: None,
                    });
                    if results.len() >= cap {
                        return Ok(results);
                    }
                }
            }
        }

        if results.len() < cap {
            remaining = cap - results.len();
            if remaining > 0 {
                let mut slide_condition = Condition::any();
                if !trimmed.is_empty() {
                    slide_condition = slide_condition
                        .add(slide_entity::Column::MainText.contains(trimmed))
                        .add(slide_entity::Column::TranslationText.contains(trimmed))
                        .add(slide_entity::Column::StageText.contains(trimmed));
                }
                if has_tokens {
                    let mut token_condition = Condition::all();
                    for token in &tokens {
                        let per_token = Condition::any()
                            .add(slide_entity::Column::MainTextSearch.contains(token.clone()))
                            .add(
                                slide_entity::Column::TranslationTextSearch.contains(token.clone()),
                            )
                            .add(slide_entity::Column::StageTextSearch.contains(token.clone()));
                        token_condition = token_condition.add(per_token);
                    }
                    slide_condition = slide_condition.add(token_condition);
                }
                if !matched_presentation_vec.is_empty() {
                    slide_condition = slide_condition.add(
                        slide_entity::Column::PresentationId
                            .is_in(matched_presentation_vec.clone()),
                    );
                }

                let slide_rows = slide_entity::Entity::find()
                    .filter(slide_condition)
                    .order_by_asc(slide_entity::Column::Position)
                    .limit(remaining as u64)
                    .find_also_related(presentation_entity::Entity)
                    .all(&self.db)
                    .await?;

                let mut pending = Vec::new();
                let mut missing_library_ids: HashSet<String> = HashSet::new();

                for (slide_model, presentation_model_opt) in slide_rows {
                    let presentation_model = match presentation_model_opt {
                        Some(model) => model,
                        None => continue,
                    };
                    if !seen_slide_ids.insert(slide_model.id.clone()) {
                        continue;
                    }
                    if !library_names.contains_key(&presentation_model.library_id) {
                        missing_library_ids.insert(presentation_model.library_id.clone());
                    }
                    pending.push((slide_model, presentation_model));
                }

                if !missing_library_ids.is_empty() {
                    let ids: Vec<String> = missing_library_ids.into_iter().collect();
                    let missing = library::Entity::find()
                        .filter(library::Column::Id.is_in(ids.clone()))
                        .all(&self.db)
                        .await?;
                    for model in missing {
                        library_names.insert(model.id.clone(), model.name.clone());
                    }
                }

                for (slide_model, presentation_model) in pending {
                    if results.len() >= cap {
                        break;
                    }
                    let library_name = library_names
                        .get(&presentation_model.library_id)
                        .cloned()
                        .unwrap_or_default();
                    let library_id =
                        LibraryId::from_uuid(parse_uuid(&presentation_model.library_id)?);
                    let presentation_id =
                        PresentationId::from_uuid(parse_uuid(&presentation_model.id)?);
                    let slide_id = SlideId::from_uuid(parse_uuid(&slide_model.id)?);

                    if has_tokens {
                        let combined = fold_query(&format!(
                            "{} {} {} {} {}",
                            library_name,
                            presentation_model.name,
                            slide_model.main_text,
                            slide_model.translation_text,
                            slide_model.stage_text
                        ));
                        if !tokens.iter().all(|token| combined.contains(token)) {
                            continue;
                        }
                    }

                    let main_lower = slide_model.main_text.to_lowercase();
                    let translation_lower = slide_model.translation_text.to_lowercase();
                    let stage_lower = slide_model.stage_text.to_lowercase();

                    let folded_ref = folded.as_str();
                    let main_matches = (!query_lower.is_empty()
                        && main_lower.contains(&query_lower))
                        || (has_folded && slide_model.main_text_search.contains(folded_ref))
                        || (has_tokens
                            && tokens
                                .iter()
                                .any(|token| slide_model.main_text_search.contains(token)));
                    let translation_matches = (!query_lower.is_empty()
                        && translation_lower.contains(&query_lower))
                        || (has_folded && slide_model.translation_text_search.contains(folded_ref))
                        || (has_tokens
                            && tokens
                                .iter()
                                .any(|token| slide_model.translation_text_search.contains(token)));
                    let stage_matches = (!query_lower.is_empty()
                        && stage_lower.contains(&query_lower))
                        || (has_folded && slide_model.stage_text_search.contains(folded_ref))
                        || (has_tokens
                            && tokens
                                .iter()
                                .any(|token| slide_model.stage_text_search.contains(token)));

                    if !main_matches && !translation_matches && !stage_matches {
                        continue;
                    }

                    let (match_field, source_text) = if main_matches {
                        (SearchMatchField::MainText, slide_model.main_text.as_str())
                    } else if translation_matches {
                        (
                            SearchMatchField::TranslationText,
                            slide_model.translation_text.as_str(),
                        )
                    } else {
                        (SearchMatchField::StageText, slide_model.stage_text.as_str())
                    };

                    let snippet = Self::build_snippet(source_text, trimmed);

                    results.push(SearchResult {
                        kind: SearchResultKind::Slide,
                        library_id,
                        library_name: library_name.clone(),
                        presentation_id: Some(presentation_id),
                        presentation_name: Some(presentation_model.name.clone()),
                        slide_id: Some(slide_id),
                        match_field,
                        snippet,
                    });
                }
            }
        }

        Ok(results)
    }

    #[instrument(skip_all)]
    pub async fn list_playlists(&self) -> anyhow::Result<Vec<Playlist>> {
        let models = playlist::Entity::find()
            .order_by_asc(playlist::Column::CreatedAt)
            .all(&self.db)
            .await?;
        let favorites = playlist_favorite::Entity::find().all(&self.db).await?;
        let favorite_ids: std::collections::HashSet<_> =
            favorites.into_iter().map(|fav| fav.playlist_id).collect();
        let mut playlists = Vec::with_capacity(models.len());
        for model in models {
            let entries_models = playlist_entry::Entity::find()
                .filter(playlist_entry::Column::PlaylistId.eq(model.id.clone()))
                .order_by_asc(playlist_entry::Column::Position)
                .all(&self.db)
                .await?;
            let entries = entries_models
                .into_iter()
                .map(to_domain_playlist_entry)
                .collect::<Result<Vec<_>, RepositoryError>>()?;
            let playlist = to_domain_playlist(&favorite_ids, model, entries)?;
            playlists.push(playlist);
        }
        Ok(playlists)
    }

    #[instrument(skip_all)]
    pub async fn create_playlist(
        &self,
        name: &str,
        show_in_dashboard: bool,
    ) -> anyhow::Result<Playlist> {
        let id = Uuid::new_v4();
        let playlist_id = PlaylistId::from_uuid(id);
        let active = playlist::ActiveModel {
            id: Set(id.to_string()),
            name: Set(name.to_string()),
            created_at: Set(Utc::now().into()),
        };
        playlist::Entity::insert(active).exec(&self.db).await?;
        if show_in_dashboard {
            self.set_playlist_favorite_internal(playlist_id, true)
                .await?;
        }
        let playlist =
            Playlist::from_parts(playlist_id, name.to_string(), show_in_dashboard, vec![])?;
        Ok(playlist)
    }

    #[instrument(skip_all)]
    pub async fn rename_playlist(&self, playlist_id: PlaylistId, name: &str) -> anyhow::Result<()> {
        playlist::Entity::update_many()
            .col_expr(playlist::Column::Name, Expr::value(name))
            .filter(playlist::Column::Id.eq(playlist_id.to_string()))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn list_playlist_favorites(&self) -> anyhow::Result<Vec<PlaylistId>> {
        let favorites = playlist_favorite::Entity::find().all(&self.db).await?;
        let mut ids = Vec::with_capacity(favorites.len());
        for fav in favorites {
            let uuid = parse_uuid(&fav.playlist_id)?;
            ids.push(PlaylistId::from_uuid(uuid));
        }
        Ok(ids)
    }

    #[instrument(skip_all)]
    pub async fn set_playlist_favorite(
        &self,
        playlist_id: PlaylistId,
        favorite: bool,
    ) -> anyhow::Result<()> {
        self.ensure_playlist_exists(playlist_id).await?;
        self.set_playlist_favorite_internal(playlist_id, favorite)
            .await
    }

    async fn set_playlist_favorite_internal(
        &self,
        playlist_id: PlaylistId,
        favorite: bool,
    ) -> anyhow::Result<()> {
        let mut txn = self.db.begin().await?;
        let id_string = playlist_id.to_string();
        if favorite {
            playlist_favorite::Entity::insert(playlist_favorite::ActiveModel {
                playlist_id: Set(id_string.clone()),
            })
            .on_conflict(
                sea_orm::sea_query::OnConflict::column(playlist_favorite::Column::PlaylistId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(&mut txn)
            .await?;
        } else {
            playlist_favorite::Entity::delete_by_id(id_string)
                .exec(&mut txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    async fn ensure_playlist_exists(&self, playlist_id: PlaylistId) -> anyhow::Result<()> {
        let count = playlist::Entity::find()
            .filter(playlist::Column::Id.eq(playlist_id.to_string()))
            .count(&self.db)
            .await?;
        if count > 0 {
            Ok(())
        } else {
            Err(anyhow!("playlist not found"))
        }
    }

    #[instrument(skip_all)]
    pub async fn delete_playlist(&self, playlist_id: PlaylistId) -> anyhow::Result<()> {
        playlist::Entity::delete_by_id(playlist_id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn replace_playlist_entries(
        &self,
        playlist_id: PlaylistId,
        entries: &[PlaylistEntry],
    ) -> anyhow::Result<()> {
        let mut txn = self.db.begin().await?;

        playlist_entry::Entity::delete_many()
            .filter(playlist_entry::Column::PlaylistId.eq(playlist_id.to_string()))
            .exec(&mut txn)
            .await?;

        for (index, entry) in entries.iter().enumerate() {
            let (entry_type, presentation_id, midi_note, label) = match &entry.kind {
                PlaylistEntryKind::Presentation {
                    presentation_id,
                    midi_binding,
                } => (
                    "presentation".to_string(),
                    Some(presentation_id.to_string()),
                    midi_binding.map(|binding| binding.note() as i32),
                    None,
                ),
                PlaylistEntryKind::Separator { name } => {
                    ("separator".to_string(), None, None, Some(name.to_string()))
                }
            };
            let entry_id = entry.id.into_uuid();
            let active = playlist_entry::ActiveModel {
                id: Set(entry_id.to_string()),
                playlist_id: Set(playlist_id.to_string()),
                entry_type: Set(entry_type),
                presentation_id: Set(presentation_id),
                position: Set(index as i32),
                midi_note: Set(midi_note),
                label: Set(label),
            };
            playlist_entry::Entity::insert(active)
                .exec(&mut txn)
                .await?;
        }

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
            .map(|slide| to_domain_slide(slide))
            .collect::<Result<Vec<_>, RepositoryError>>()?;

        let presentation = Presentation::new(pres_model.name.clone(), slide_models)?
            .with_id(PresentationId::from_uuid(parse_uuid(&pres_model.id)?));

        let library_id = LibraryId::from_uuid(parse_uuid(&pres_model.library_id)?);
        let library_name = library::Entity::find_by_id(pres_model.library_id.clone())
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
        let mut txn = self.db.begin().await?;

        slide_entity::Entity::delete_many()
            .filter(slide_entity::Column::PresentationId.eq(presentation_id.to_string()))
            .exec(&mut txn)
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
            };
            slide_entity::Entity::insert(active).exec(&mut txn).await?;
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
        let mut txn = self.db.begin().await?;

        bible_translation::Entity::delete_by_id(translation.code.clone())
            .exec(&mut txn)
            .await?;

        let translation_model = bible_translation::ActiveModel {
            code: Set(translation.code.clone()),
            name: Set(translation.name.clone()),
            language: Set(translation.language.clone()),
            source: Set(translation.source.clone()),
            created_at: Set(Utc::now().into()),
        };

        bible_translation::Entity::insert(translation_model)
            .exec(&mut txn)
            .await?;

        let mut chunk = Vec::with_capacity(BIBLE_INSERT_CHUNK);
        for passage in passages {
            let reference = &passage.reference;
            let model = bible_passage::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                translation_code: Set(translation.code.clone()),
                book: Set(reference.book.clone()),
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
                    .exec(&mut txn)
                    .await?;
            }
        }

        if !chunk.is_empty() {
            bible_passage::Entity::insert_many(chunk)
                .exec(&mut txn)
                .await?;
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
                sea_orm::sea_query::OnConflict::column(stage_state::Column::Id)
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
                sea_orm::sea_query::OnConflict::column(timers::Column::Id)
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

    fn build_snippet(text: &str, query: &str) -> Option<String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }

        let chars: Vec<char> = trimmed.chars().collect();
        if chars.len() <= 160 {
            return Some(trimmed.to_string());
        }

        let lower_text = trimmed.to_lowercase();
        let lower_query = query.to_lowercase();
        let match_char_index = lower_text
            .find(&lower_query)
            .map(|byte_index| trimmed[..byte_index].chars().count())
            .unwrap_or(0);

        let start = match_char_index.saturating_sub(20);
        let end = (start + 160).min(chars.len());

        let mut snippet: String = chars[start..end].iter().collect();
        if start > 0 {
            snippet.insert(0, '…');
        }
        if end < chars.len() {
            snippet.push('…');
        }
        Some(snippet)
    }
}

#[derive(Debug, Error)]
enum RepositoryError {
    #[error("invalid uuid stored in database: {0}")]
    InvalidUuid(String),
    #[error(transparent)]
    Domain(#[from] presenter_core::presentation::PresentationError),
    #[error(transparent)]
    Slide(#[from] presenter_core::slide::SlideValidationError),
    #[error(transparent)]
    Library(#[from] presenter_core::library::LibraryError),
    #[error(transparent)]
    Reference(#[from] presenter_core::bible::BibleReferenceError),
    #[error(transparent)]
    Playlist(#[from] presenter_core::playlist::PlaylistError),
    #[error("unknown timer state '{0}' in persistence layer")]
    UnknownTimerState(String),
    #[error("unknown playlist entry type '{0}' in persistence layer")]
    UnknownPlaylistEntryType(String),
    #[error("osc port {0} out of range")]
    InvalidOscPort(i32),
    #[error("ableset port {0} out of range")]
    InvalidAbleSetPort(i32),
    #[error("ableset song prefix length {0} out of range")]
    InvalidAbleSetPrefix(i32),
    #[error("unknown osc velocity mode '{0}' in persistence layer")]
    UnknownOscVelocityMode(String),
}

fn parse_uuid(id: &str) -> Result<Uuid, RepositoryError> {
    Uuid::parse_str(id).map_err(|_| RepositoryError::InvalidUuid(id.to_string()))
}

fn to_domain_slide(model: slide_entity::Model) -> Result<Slide, RepositoryError> {
    let content = SlideContent::new(
        SlideText::new(model.main_text)?,
        SlideText::new(model.translation_text)?,
        SlideText::new(model.stage_text)?,
        model.group_name.map(SlideGroup::new),
    );
    let slide = Slide::new(model.position as u32, content)
        .with_id(SlideId::from_uuid(parse_uuid(&model.id)?));
    Ok(slide)
}

fn to_domain_playlist_entry(
    model: playlist_entry::Model,
) -> Result<PlaylistEntry, RepositoryError> {
    let id = PlaylistEntryId::from_uuid(parse_uuid(&model.id)?);
    let entry_type = model.entry_type.to_lowercase();
    let kind = match entry_type.as_str() {
        "separator" => {
            let name = model
                .label
                .unwrap_or_else(|| "Separator".to_string())
                .trim()
                .to_string();
            PlaylistEntryKind::Separator { name }
        }
        "presentation" | "song" | "item" | "default" | "" => {
            let presentation_id = model
                .presentation_id
                .as_ref()
                .ok_or_else(|| RepositoryError::UnknownPlaylistEntryType(entry_type.clone()))
                .and_then(|id| Ok(PresentationId::from_uuid(parse_uuid(id)?)))?;
            let midi_binding = match model.midi_note {
                Some(note) => {
                    if !(0..=127).contains(&note) {
                        return Err(presenter_core::playlist::PlaylistError::MidiOutOfRange(
                            note.clamp(0, 127) as u8,
                        )
                        .into());
                    }
                    Some(MidiBinding::new(note as u8)?)
                }
                None => None,
            };
            PlaylistEntryKind::Presentation {
                presentation_id,
                midi_binding,
            }
        }
        other => return Err(RepositoryError::UnknownPlaylistEntryType(other.to_string())),
    };
    Ok(PlaylistEntry { id, kind })
}

fn to_domain_playlist(
    favorite_ids: &HashSet<String>,
    model: playlist::Model,
    entries: Vec<PlaylistEntry>,
) -> Result<Playlist, RepositoryError> {
    let id = PlaylistId::from_uuid(parse_uuid(&model.id)?);
    let show_in_dashboard = favorite_ids.contains(&model.id);
    Ok(Playlist::from_parts(
        id,
        model.name,
        show_in_dashboard,
        entries,
    )?)
}

fn ableset_model_to_domain(
    model: ableset_settings::Model,
) -> Result<AbleSetSettings, RepositoryError> {
    let osc_port = cast_ableset_port(model.osc_port)?;
    let http_port = cast_ableset_port(model.http_port)?;
    let prefix_length = cast_prefix_length(model.song_prefix_length)?;
    Ok(AbleSetSettings::new(
        model.enabled,
        model.host,
        osc_port,
        http_port,
        model.library_name,
        prefix_length,
        model.created_at.into(),
        model.updated_at.into(),
    ))
}

fn cast_ableset_port(value: i32) -> Result<u16, RepositoryError> {
    if value <= 0 || value > u16::MAX as i32 {
        return Err(RepositoryError::InvalidAbleSetPort(value));
    }
    Ok(value as u16)
}

fn cast_prefix_length(value: i32) -> Result<u8, RepositoryError> {
    if value <= 0 || value > u8::MAX as i32 {
        return Err(RepositoryError::InvalidAbleSetPrefix(value));
    }
    Ok(value as u8)
}

fn osc_model_to_domain(model: osc_settings::Model) -> Result<OscSettings, RepositoryError> {
    let mode = parse_velocity_mode(&model.velocity_mode)?;
    let port_i32 = model.listen_port;
    if port_i32 <= 0 || port_i32 > u16::MAX as i32 {
        return Err(RepositoryError::InvalidOscPort(port_i32));
    }
    let port = port_i32 as u16;
    Ok(OscSettings::new(
        model.enabled,
        port,
        model.address_pattern,
        mode,
        model.created_at.into(),
        model.updated_at.into(),
    ))
}

fn velocity_mode_to_string(mode: VelocityMode) -> &'static str {
    match mode {
        VelocityMode::ZeroBased => "zero_based",
        VelocityMode::OneBased => "one_based",
    }
}

fn parse_velocity_mode(value: &str) -> Result<VelocityMode, RepositoryError> {
    match value.to_lowercase().as_str() {
        "zero_based" | "zero-based" | "zero" | "0" => Ok(VelocityMode::ZeroBased),
        "one_based" | "one-based" | "one" | "1" => Ok(VelocityMode::OneBased),
        other => Err(RepositoryError::UnknownOscVelocityMode(other.to_string())),
    }
}

fn to_domain_translation(model: bible_translation::Model) -> BibleTranslation {
    let mut translation = BibleTranslation::new(
        model.code.clone(),
        model.name.clone(),
        model.language.clone(),
    );
    if let Some(source) = model.source.clone() {
        translation = translation.with_source(source);
    }
    translation
}

fn to_domain_passage(
    model: bible_passage::Model,
    translation: bible_translation::Model,
) -> Result<BiblePassage, RepositoryError> {
    let reference = BibleReference::new(
        model.book,
        model.chapter as u16,
        model.verse_start as u16,
        model.verse_end as u16,
    )?;
    let translation = to_domain_translation(translation);
    Ok(BiblePassage::new(reference, translation, model.content))
}

fn resolume_model_to_domain(model: resolume_host::Model) -> anyhow::Result<ResolumeHost> {
    let id = ResolumeHostId::from_uuid(
        Uuid::parse_str(&model.id).map_err(|_| anyhow!("invalid resolume host id"))?,
    );
    let port = u16::try_from(model.port).map_err(|_| anyhow!("resolume host port out of range"))?;
    if port == 0 {
        return Err(anyhow!("resolume host port cannot be zero"));
    }
    let created_at: DateTime<Utc> = model.created_at.into();
    let updated_at: DateTime<Utc> = model.updated_at.into();
    Ok(ResolumeHost::new(
        id,
        model.label,
        model.host,
        port,
        model.is_enabled,
        created_at,
        updated_at,
    ))
}

fn timer_state_to_string(state: TimerState) -> String {
    match state {
        TimerState::Idle => "idle".to_string(),
        TimerState::Running => "running".to_string(),
        TimerState::Paused => "paused".to_string(),
        TimerState::Completed => "completed".to_string(),
    }
}

fn stage_state_model_to_state(model: stage_state::Model) -> Result<StageState, RepositoryError> {
    let presentation_id = match model.presentation_id {
        Some(value) => Some(PresentationId::from_uuid(parse_uuid(&value)?)),
        None => None,
    };
    let current_slide_id = match model.current_slide_id {
        Some(value) => Some(SlideId::from_uuid(parse_uuid(&value)?)),
        None => None,
    };
    let next_slide_id = match model.next_slide_id {
        Some(value) => Some(SlideId::from_uuid(parse_uuid(&value)?)),
        None => None,
    };
    Ok(StageState::new(
        presentation_id,
        current_slide_id,
        next_slide_id,
    ))
}

fn timers_model_to_state(model: timers::Model) -> Result<TimersState, RepositoryError> {
    let countdown_state = parse_timer_state(&model.countdown_state)?;
    let preach_state = parse_timer_state(&model.preach_state)?;

    let countdown = CountdownTimer {
        target: model.countdown_target.into(),
        state: countdown_state,
    };

    let preach = PreachTimer::from_parts(
        preach_state,
        model.preach_started_at.map(Into::into),
        Duration::seconds(model.preach_accumulated_seconds),
    );

    Ok(TimersState::new(countdown, preach))
}

fn parse_timer_state(value: &str) -> Result<TimerState, RepositoryError> {
    match value {
        "idle" => Ok(TimerState::Idle),
        "running" => Ok(TimerState::Running),
        "paused" => Ok(TimerState::Paused),
        "completed" => Ok(TimerState::Completed),
        other => Err(RepositoryError::UnknownTimerState(other.to_string())),
    }
}

const BIBLE_INSERT_CHUNK: usize = 500;

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::{
        bible::BibleIngestionBatch, playlist::PlaylistEntryKind, BiblePassage, BibleReference,
        BibleTranslation, PlaylistEntryId, ResolumeHostDraft, SearchResultKind, StageState,
    };
    fn sample_library() -> Library {
        let presentation = Presentation::new(
            "Welcome",
            vec![
                Slide::new(
                    0,
                    SlideContent::new(
                        SlideText::new("Nádej v Pánovi").unwrap(),
                        SlideText::new("Hope in the Lord").unwrap(),
                        SlideText::new("Stage").unwrap(),
                        Some(SlideGroup::new("Intro")),
                    ),
                )
                .with_id(SlideId::new()),
                Slide::new(
                    1,
                    SlideContent::new(
                        SlideText::new("Second").unwrap(),
                        SlideText::new("Druga").unwrap(),
                        SlideText::new("Stage 2").unwrap(),
                        None,
                    ),
                )
                .with_id(SlideId::new()),
            ],
        )
        .unwrap()
        .with_id(PresentationId::new());
        Library::new("Default", vec![presentation])
            .unwrap()
            .with_id(LibraryId::new())
    }

    #[tokio::test]
    async fn round_trip_library() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        repo.upsert_library(&library).await.unwrap();
        let libraries = repo.fetch_libraries().await.unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].name, "Default");
        assert_eq!(libraries[0].presentations.len(), 1);
        assert_eq!(libraries[0].presentations[0].slides.len(), 2);
    }

    #[tokio::test]
    async fn fetch_first_presentation_detail_returns_sample() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        repo.upsert_library(&library).await.unwrap();

        let detail = repo.fetch_first_presentation_detail().await.unwrap();
        assert!(detail.is_some());
        let (_, name, presentation) = detail.unwrap();
        assert_eq!(name, "Default");
        assert_eq!(presentation.slides.len(), 2);
    }

    #[tokio::test]
    async fn update_slide_content_persists_changes() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        let presentation = library.presentations[0].clone();
        let slide = presentation.slides[0].clone();
        repo.upsert_library(&library).await.unwrap();

        let new_content = SlideContent::new(
            SlideText::new("Updated main").unwrap(),
            SlideText::new("Updated translation").unwrap(),
            SlideText::new("Updated stage").unwrap(),
            Some(SlideGroup::new("Updated Group")),
        );

        repo.update_slide_content(presentation.id, slide.id, &new_content)
            .await
            .unwrap();

        let detail = repo
            .fetch_presentation_detail(presentation.id)
            .await
            .unwrap()
            .expect("presentation detail");
        let updated = detail
            .2
            .slides
            .iter()
            .find(|candidate| candidate.id == slide.id)
            .expect("slide present");

        assert_eq!(updated.content.main.value(), "Updated main");
        assert_eq!(updated.content.translation.value(), "Updated translation");
        assert_eq!(updated.content.stage.value(), "Updated stage");
        assert_eq!(
            updated.content.group.as_ref().map(|group| group.name()),
            Some("Updated Group")
        );
    }

    #[tokio::test]
    async fn purge_presentation_content_clears_tables() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        repo.upsert_library(&library).await.unwrap();

        let presentation = &library.presentations[0];
        let current = presentation.slides[0].id;
        let state = StageState::new(Some(presentation.id), Some(current), None);
        repo.upsert_stage_state(&state).await.unwrap();

        repo.purge_presentation_content().await.unwrap();

        let libraries = repo.fetch_libraries().await.unwrap();
        assert!(libraries.is_empty());
        let stage = repo.get_stage_state().await.unwrap();
        assert!(stage.is_none());
    }

    #[tokio::test]
    async fn bible_translation_round_trip() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let translation = BibleTranslation::new("en-kjv", "King James Version", "en")
            .with_source("https://example.org/kjv");
        let reference = BibleReference::new("John", 3, 16, 16).unwrap();
        let passage = BiblePassage::new(
            reference.clone(),
            translation.clone(),
            "For God so loved the world".to_string(),
        );
        let batch = BibleIngestionBatch::new(translation.clone(), vec![passage.clone()]).unwrap();
        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();

        let translations = repo.list_bible_translations().await.unwrap();
        assert_eq!(translations.len(), 1);
        assert_eq!(translations[0], translation);

        let fetched = repo
            .find_bible_passage(&translation.code, &reference)
            .await
            .unwrap()
            .expect("passage to exist");
        assert_eq!(fetched.translation, translation);
        assert_eq!(fetched.reference, reference);
        assert_eq!(fetched.text, passage.text);
    }

    #[tokio::test]
    async fn bible_translation_large_batch_is_persisted() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let translation = BibleTranslation::new("en-load", "Load Test", "en");
        let mut passages = Vec::new();
        for verse in 1..=1_200u16 {
            let reference = BibleReference::new("Psalm", 119, verse, verse).unwrap();
            let passage =
                BiblePassage::new(reference, translation.clone(), format!("Verse {verse}"));
            passages.push(passage);
        }

        let batch = BibleIngestionBatch::new(translation.clone(), passages).unwrap();
        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();

        let tail_reference = BibleReference::new("Psalm", 119, 1_200, 1_200).unwrap();
        let tail = repo
            .find_bible_passage(&translation.code, &tail_reference)
            .await
            .unwrap();
        assert!(tail.is_some());

        let head_reference = BibleReference::new("Psalm", 119, 1, 1).unwrap();
        let head = repo
            .find_bible_passage(&translation.code, &head_reference)
            .await
            .unwrap();
        assert!(head.is_some());
    }

    #[tokio::test]
    async fn osc_settings_default_is_seeded() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let settings = repo.get_osc_settings().await.unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.listen_port, 9000);
        assert_eq!(settings.address_pattern, "/note");
        assert_eq!(settings.velocity_mode, VelocityMode::ZeroBased);
    }

    #[tokio::test]
    async fn osc_settings_upsert_updates_values() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let draft = OscSettingsDraft {
            enabled: true,
            listen_port: 10023,
            address_pattern: "/presenter/trigger".to_string(),
            velocity_mode: VelocityMode::OneBased,
        };
        let updated = repo.upsert_osc_settings(&draft).await.unwrap();
        assert!(updated.enabled);
        assert_eq!(updated.listen_port, 10023);
        assert_eq!(updated.address_pattern, "/presenter/trigger");
        assert_eq!(updated.velocity_mode, VelocityMode::OneBased);

        // upsert idempotency
        let second = repo.get_osc_settings().await.unwrap();
        assert_eq!(second.address_pattern, updated.address_pattern);
        assert_eq!(second.listen_port, updated.listen_port);
    }

    #[tokio::test]
    async fn resolume_host_crud_round_trip() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let draft = ResolumeHostDraft::new("Arena", "resolume.lan", 8090);
        let created = repo.create_resolume_host(&draft).await.unwrap();
        assert_eq!(created.label, "Arena");
        assert_eq!(created.host, "resolume.lan");
        assert_eq!(created.port, 8090);
        assert!(created.is_enabled);

        let hosts = repo.list_resolume_hosts().await.unwrap();
        assert_eq!(hosts.len(), 1);

        let updated_draft =
            ResolumeHostDraft::new("Arena North", "resolume.lan", 8090).with_enabled(false);
        let updated = repo
            .update_resolume_host(created.id, &updated_draft)
            .await
            .unwrap();
        assert_eq!(updated.label, "Arena North");
        assert!(!updated.is_enabled);

        repo.delete_resolume_host(created.id).await.unwrap();
        let after_delete = repo.list_resolume_hosts().await.unwrap();
        assert!(after_delete.is_empty());
    }

    #[tokio::test]
    async fn timers_state_round_trip() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let now = Utc::now();
        let target = now + Duration::minutes(10);
        let countdown = CountdownTimer {
            target,
            state: TimerState::Running,
        };
        let mut preach = PreachTimer::new();
        preach.start(now);
        let pause_instant = now + Duration::minutes(1);
        preach.pause(pause_instant);
        let expected_preach = preach.clone();
        let state = TimersState::new(countdown, preach);

        repo.upsert_timers_state(&state).await.unwrap();

        let stored = repo.get_timers_state().await.unwrap().unwrap();
        assert_eq!(stored.countdown.state, TimerState::Running);
        assert_eq!(stored.countdown.target, target);
        assert_eq!(stored.preach.state, expected_preach.state);
        assert!(stored.preach.elapsed(pause_instant).num_seconds() >= 60);
    }

    #[tokio::test]
    async fn playlist_round_trip_with_separator() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        let presentation = library.presentations[0].clone();
        repo.upsert_library(&library).await.unwrap();

        let playlist = repo
            .create_playlist("Good Fest Friday", true)
            .await
            .unwrap();

        let entries = vec![
            PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Separator {
                    name: "Doors Open".to_string(),
                },
            },
            PlaylistEntry {
                id: PlaylistEntryId::new(),
                kind: PlaylistEntryKind::Presentation {
                    presentation_id: presentation.id,
                    midi_binding: None,
                },
            },
        ];

        repo.replace_playlist_entries(playlist.id, &entries)
            .await
            .unwrap();

        let playlists = repo.list_playlists().await.unwrap();
        assert_eq!(playlists.len(), 1);

        let fetched = &playlists[0];
        assert_eq!(fetched.name, "Good Fest Friday");
        assert!(fetched.show_in_dashboard);
        assert_eq!(fetched.entries.len(), 2);
        assert!(matches!(
            fetched.entries[0].kind,
            PlaylistEntryKind::Separator { .. }
        ));
        assert!(matches!(
            fetched.entries[1].kind,
            PlaylistEntryKind::Presentation {
                presentation_id,
                ..
            } if presentation_id == presentation.id
        ));
    }

    #[tokio::test]
    async fn create_library_persists_new_row() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let created = repo.create_library("Autotest Library").await.unwrap();
        assert_eq!(created.name, "Autotest Library");
        assert!(created.presentations.is_empty());

        let libraries = repo.fetch_libraries().await.unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].id, created.id);
        assert_eq!(libraries[0].name, "Autotest Library");
    }

    #[tokio::test]
    async fn rename_library_updates_name() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let created = repo.create_library("Original").await.unwrap();

        repo.rename_library(created.id, "Renamed").await.unwrap();

        let libraries = repo.fetch_libraries().await.unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].name, "Renamed");
    }

    #[tokio::test]
    async fn delete_library_removes_presentations_and_slides() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        let presentation = library.presentations[0].clone();
        repo.upsert_library(&library).await.unwrap();

        repo.delete_library(library.id).await.unwrap();

        let libraries = repo.fetch_libraries().await.unwrap();
        assert!(libraries.is_empty());

        let detail = repo
            .fetch_presentation_detail(presentation.id)
            .await
            .unwrap();
        assert!(detail.is_none());
    }

    #[tokio::test]
    async fn replace_presentation_slides_overwrites_rows() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        let presentation = library.presentations[0].clone();
        repo.upsert_library(&library).await.unwrap();

        let mut slides = presentation.slides.clone();
        let new_slide = Slide::new(
            slides.len() as u32,
            SlideContent::new(
                SlideText::new("New main").unwrap(),
                SlideText::new("New translation").unwrap(),
                SlideText::new("New stage").unwrap(),
                None,
            ),
        );
        slides.push(new_slide);
        for (index, slide) in slides.iter_mut().enumerate() {
            slide.order = index as u32;
        }

        repo.replace_presentation_slides(presentation.id, &slides)
            .await
            .unwrap();

        let detail = repo
            .fetch_presentation_detail(presentation.id)
            .await
            .unwrap()
            .expect("detail");
        assert_eq!(detail.2.slides.len(), slides.len());
        assert_eq!(
            detail.2.slides.last().unwrap().content.main.value(),
            "New main",
        );
    }

    #[tokio::test]
    async fn search_presenter_returns_hits() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let mut library = sample_library();
        library.name = "Chvály Nedeľa".to_string();
        repo.upsert_library(&library).await.unwrap();

        let mut other = sample_library();
        other.name = "Youth Set".to_string();
        repo.upsert_library(&other).await.unwrap();

        let accent_slide = Slide::new(
            0,
            SlideContent::new(
                SlideText::new("Ježiš, ja Ťa chválim").unwrap(),
                SlideText::new("Jesus I praise you").unwrap(),
                SlideText::new("Stage cue").unwrap(),
                None,
            ),
        );
        let accent_presentation = Presentation::new("Ježiš, ja", vec![accent_slide]).unwrap();
        let accent_library = Library::new("TYMY Worship", vec![accent_presentation]).unwrap();
        repo.upsert_library(&accent_library).await.unwrap();

        let results = repo.search_presenter("Chvaly", 10).await.unwrap();
        assert!(!results.is_empty());
        assert!(results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Library)
                && result.library_name == "Chvály Nedeľa"));

        let presentation_results = repo.search_presenter("Welcome", 10).await.unwrap();
        assert!(presentation_results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Presentation)));

        let slide_results = repo.search_presenter("Nadej", 10).await.unwrap();
        assert!(slide_results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Slide)));

        let compound_results = repo.search_presenter("jezis, ja tymy", 10).await.unwrap();
        assert!(compound_results.iter().any(|result| {
            matches!(
                result.kind,
                SearchResultKind::Slide | SearchResultKind::Presentation
            ) && result.library_name == "TYMY Worship"
        }));
    }

    #[tokio::test]
    async fn stage_state_round_trip() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let library = sample_library();
        let presentation = library.presentations[0].clone();
        repo.upsert_library(&library).await.unwrap();

        let current = presentation.slides[0].id;
        let next = presentation.slides.get(1).map(|slide| slide.id);
        let state = StageState::new(Some(presentation.id), Some(current), next);

        repo.upsert_stage_state(&state).await.unwrap();

        let stored = repo.get_stage_state().await.unwrap().unwrap();
        assert_eq!(stored.presentation_id, Some(presentation.id));
        assert_eq!(stored.current_slide_id, Some(current));
        assert_eq!(stored.next_slide_id, next);
    }
}
