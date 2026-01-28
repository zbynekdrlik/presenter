use crate::entities::{
    library, library_favorite, presentation as presentation_entity, slide as slide_entity,
};
use anyhow::anyhow;
use chrono::Utc;
use presenter_core::{
    search::fold_query, Library, LibraryId, LibrarySummary, Presentation, PresentationId,
    PresentationSummary,
};
use sea_orm::{
    sea_query::OnConflict, ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, Set, TransactionTrait,
};
use std::collections::HashMap;
use tracing::instrument;

use super::helpers::{parse_uuid, to_domain_slide, RepositoryError};
use super::Repository;

impl Repository {
    #[instrument(skip_all)]
    pub async fn upsert_library(&self, library: &Library) -> anyhow::Result<()> {
        let txn = self.db.begin().await?;

        library::Entity::delete_by_id(library.id.to_string())
            .exec(&txn)
            .await?;

        let lib_model = library::ActiveModel {
            id: Set(library.id.to_string()),
            name: Set(library.name.clone()),
            search_name: Set(fold_query(&library.name)),
            created_at: Set(Utc::now().into()),
        };
        library::Entity::insert(lib_model).exec(&txn).await?;

        for presentation in &library.presentations {
            let pres_model = presentation_entity::ActiveModel {
                id: Set(presentation.id.to_string()),
                library_id: Set(library.id.to_string()),
                name: Set(presentation.name.clone()),
                search_name: Set(fold_query(&presentation.name)),
                created_at: Set(Utc::now().into()),
            };
            presentation_entity::Entity::insert(pres_model)
                .exec(&txn)
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
                    metadata_json: Set(slide
                        .metadata
                        .as_ref()
                        .and_then(|m| serde_json::to_string(m).ok())),
                };
                slide_entity::Entity::insert(slide_model).exec(&txn).await?;
            }
        }

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn create_library(&self, name: &str) -> anyhow::Result<Library> {
        let txn = self.db.begin().await?;
        let id = LibraryId::new();

        let model = library::ActiveModel {
            id: Set(id.to_string()),
            name: Set(name.to_string()),
            search_name: Set(fold_query(name)),
            created_at: Set(Utc::now().into()),
        };

        library::Entity::insert(model).exec(&txn).await?;
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

    /// Fetches all libraries with presentations and slides using batch queries.
    /// Optimized to use 3 queries total instead of 1 + n + (n*m) queries.
    #[instrument(skip_all)]
    pub async fn fetch_libraries(&self) -> anyhow::Result<Vec<Library>> {
        // Query 1: Fetch all libraries
        let libraries = library::Entity::find()
            .order_by_asc(library::Column::Name)
            .all(&self.db)
            .await?;

        if libraries.is_empty() {
            return Ok(Vec::new());
        }

        let library_ids: Vec<String> = libraries.iter().map(|lib| lib.id.clone()).collect();

        // Query 2: Batch fetch all presentations for these libraries
        let all_presentations = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::LibraryId.is_in(library_ids))
            .order_by_asc(presentation_entity::Column::Name)
            .all(&self.db)
            .await?;

        let presentation_ids: Vec<String> =
            all_presentations.iter().map(|p| p.id.clone()).collect();

        // Query 3: Batch fetch all slides for these presentations
        let all_slides = if presentation_ids.is_empty() {
            Vec::new()
        } else {
            slide_entity::Entity::find()
                .filter(slide_entity::Column::PresentationId.is_in(presentation_ids))
                .order_by_asc(slide_entity::Column::Position)
                .all(&self.db)
                .await?
        };

        // Group slides by presentation_id in memory
        let mut slides_by_presentation: HashMap<String, Vec<slide_entity::Model>> =
            HashMap::with_capacity(all_presentations.len());
        for slide in all_slides {
            slides_by_presentation
                .entry(slide.presentation_id.clone())
                .or_default()
                .push(slide);
        }

        // Group presentations by library_id in memory
        let mut presentations_by_library: HashMap<String, Vec<presentation_entity::Model>> =
            HashMap::with_capacity(libraries.len());
        for pres in all_presentations {
            presentations_by_library
                .entry(pres.library_id.clone())
                .or_default()
                .push(pres);
        }

        // Build domain models
        let mut results = Vec::with_capacity(libraries.len());
        for lib in libraries {
            let presentations = presentations_by_library.remove(&lib.id).unwrap_or_default();

            let mut presentation_models = Vec::with_capacity(presentations.len());
            for pres in presentations {
                let slides = slides_by_presentation.remove(&pres.id).unwrap_or_default();

                let slide_models = slides
                    .into_iter()
                    .map(to_domain_slide)
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
        let txn = self.db.begin().await?;
        let id_string = library_id.to_string();

        if favorite {
            library_favorite::Entity::insert(library_favorite::ActiveModel {
                library_id: Set(id_string.clone()),
            })
            .on_conflict(
                OnConflict::column(library_favorite::Column::LibraryId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        } else {
            library_favorite::Entity::delete_by_id(id_string.clone())
                .exec(&txn)
                .await?;
        }

        txn.commit().await?;
        Ok(())
    }

    /// Lists library summaries with presentation counts using batch queries.
    /// Optimized to use 2 queries total instead of 1 + n queries.
    #[instrument(skip_all)]
    pub async fn list_library_summaries(
        &self,
        filter: Option<&str>,
    ) -> anyhow::Result<Vec<LibrarySummary>> {
        // Query 1: Fetch libraries (with optional filter)
        let mut query = library::Entity::find();
        if let Some(filter) = filter {
            let pattern = format!("%{}%", filter);
            query = query.filter(library::Column::Name.like(pattern));
        }

        let libraries = query
            .order_by_asc(library::Column::Name)
            .all(&self.db)
            .await?;

        if libraries.is_empty() {
            return Ok(Vec::new());
        }

        let library_ids: Vec<String> = libraries.iter().map(|lib| lib.id.clone()).collect();

        // Query 2: Batch fetch all presentations for these libraries
        let all_presentations = presentation_entity::Entity::find()
            .filter(presentation_entity::Column::LibraryId.is_in(library_ids))
            .order_by_asc(presentation_entity::Column::Name)
            .all(&self.db)
            .await?;

        // Group presentations by library_id in memory
        let mut presentations_by_library: HashMap<String, Vec<presentation_entity::Model>> =
            HashMap::with_capacity(libraries.len());
        for pres in all_presentations {
            presentations_by_library
                .entry(pres.library_id.clone())
                .or_default()
                .push(pres);
        }

        // Build summaries
        let mut summaries = Vec::with_capacity(libraries.len());
        for lib in libraries {
            let presentations = presentations_by_library.remove(&lib.id).unwrap_or_default();

            let mut presentation_summaries = Vec::with_capacity(presentations.len());
            for pres in &presentations {
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
}
