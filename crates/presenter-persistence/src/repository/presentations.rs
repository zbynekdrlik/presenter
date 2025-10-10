use anyhow::anyhow;
use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, TransactionTrait};
use tracing::instrument;

use crate::entities::{library, presentation as presentation_entity, slide as slide_entity};
use presenter_core::{
    search::fold_query, LibraryId, Presentation, PresentationId, Slide, SlideContent, SlideGroup,
    SlideId, SlideText,
};
use sea_orm::Set;

use super::util::{parse_uuid, to_domain_slide, RepositoryError};
use super::Repository;

impl Repository {
    #[instrument(skip_all)]
    pub async fn create_presentation(
        &self,
        library_id: LibraryId,
        name: &str,
    ) -> anyhow::Result<(LibraryId, String, Presentation)> {
        let presentation_uuid = uuid::Uuid::new_v4();
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
            id: Set(uuid::Uuid::new_v4().to_string()),
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
}
