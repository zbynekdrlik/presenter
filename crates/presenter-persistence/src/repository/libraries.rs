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
use tracing::instrument;

use crate::entities::{
    library, library_favorite, presentation as presentation_entity, slide as slide_entity,
    stage_state,
};

use super::{parse_uuid, to_domain_slide, Repository, RepositoryError, STAGE_STATE_SINGLETON_ID};

impl Repository {
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
                OnConflict::column(library_favorite::Column::LibraryId)
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
}
