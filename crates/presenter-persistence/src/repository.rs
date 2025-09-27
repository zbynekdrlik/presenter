use crate::entities::{library, presentation as presentation_entity, slide as slide_entity};
use anyhow::Context;
use chrono::Utc;
use presenter_core::{
    Library, LibraryId, Presentation, PresentationId, Slide, SlideContent, SlideGroup, SlideId,
    SlideText,
};
use presenter_migration::{Migrator, MigratorTrait};
use sea_orm::{
    ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
    TransactionTrait,
};
use std::fmt::Debug;
use thiserror::Error;
use tracing::instrument;
use uuid::Uuid;

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
    pub async fn upsert_library(&self, library: &Library) -> anyhow::Result<()> {
        let mut txn = self.db.begin().await?;

        library::Entity::delete_by_id(library.id.to_string())
            .exec(&mut txn)
            .await?;

        let lib_model = library::ActiveModel {
            id: Set(library.id.to_string()),
            name: Set(library.name.clone()),
            created_at: Set(Utc::now().into()),
        };
        library::Entity::insert(lib_model).exec(&mut txn).await?;

        for presentation in &library.presentations {
            let pres_model = presentation_entity::ActiveModel {
                id: Set(presentation.id.to_string()),
                library_id: Set(library.id.to_string()),
                name: Set(presentation.name.clone()),
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
                    translation_text: Set(slide.content.translation.value().to_owned()),
                    stage_text: Set(slide.content.stage.value().to_owned()),
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

#[cfg(test)]
mod tests {
    use super::*;
    fn sample_library() -> Library {
        let presentation = Presentation::new(
            "Welcome",
            vec![
                Slide::new(
                    0,
                    SlideContent::new(
                        SlideText::new("Main line").unwrap(),
                        SlideText::new("Translation").unwrap(),
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
}
