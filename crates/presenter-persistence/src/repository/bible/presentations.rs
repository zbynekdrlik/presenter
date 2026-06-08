use anyhow::anyhow;
use chrono::Utc;
use sea_orm::sea_query::{Expr, Expr as SeaExpr};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
};
use tracing::instrument;
use uuid::Uuid;

use crate::entities::{bible_presentation, bible_slide};
use presenter_core::{
    search::fold_query, slide::BibleSlideMetadata, BiblePresentation, BiblePresentationId,
    BiblePresentationSlide, BiblePresentationSummary, BibleSlideId, SlideText,
};

use crate::repository::util::RepositoryError;
use crate::repository::Repository;

impl Repository {
    // ── Bible presentations ────────────────────────────────────────

    #[instrument(skip_all)]
    pub async fn list_bible_presentation_summaries(
        &self,
    ) -> anyhow::Result<Vec<BiblePresentationSummary>> {
        let presentations = bible_presentation::Entity::find()
            .order_by_asc(bible_presentation::Column::Name)
            .all(&self.db)
            .await?;

        #[derive(FromQueryResult)]
        struct CountRow {
            presentation_id: String,
            slide_count: i64,
        }

        let count_rows: Vec<CountRow> = bible_slide::Entity::find()
            .select_only()
            .column(bible_slide::Column::PresentationId)
            .column_as(Expr::col(bible_slide::Column::Id).count(), "slide_count")
            .group_by(bible_slide::Column::PresentationId)
            .into_model::<CountRow>()
            .all(&self.db)
            .await?;

        let counts: std::collections::HashMap<String, usize> = count_rows
            .into_iter()
            .map(|r| (r.presentation_id, r.slide_count as usize))
            .collect();

        let mut summaries = Vec::with_capacity(presentations.len());
        for model in presentations {
            let slide_count = counts.get(&model.id).copied().unwrap_or(0);
            let uuid = Uuid::parse_str(&model.id)
                .map_err(|_| RepositoryError::InvalidUuid(model.id.clone()))?;
            summaries.push(BiblePresentationSummary {
                id: BiblePresentationId::from_uuid(uuid),
                name: model.name,
                slide_count,
            });
        }
        Ok(summaries)
    }

    #[instrument(skip_all)]
    pub async fn fetch_bible_presentation(
        &self,
        id: BiblePresentationId,
    ) -> anyhow::Result<Option<BiblePresentation>> {
        let id_str = id.to_string();
        let Some(model) = bible_presentation::Entity::find_by_id(id_str.clone())
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };

        let slide_models = bible_slide::Entity::find()
            .filter(bible_slide::Column::PresentationId.eq(id_str))
            .order_by_asc(bible_slide::Column::SlideOrder)
            .all(&self.db)
            .await?;

        let mut slides = Vec::with_capacity(slide_models.len());
        for slide_model in slide_models {
            slides.push(model_to_bible_slide(slide_model)?);
        }

        Ok(Some(BiblePresentation {
            id,
            name: model.name,
            slides,
            created_at: model.created_at.into(),
        }))
    }

    #[instrument(skip_all)]
    pub async fn create_bible_presentation(&self, name: &str) -> anyhow::Result<BiblePresentation> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("bible presentation name cannot be empty"));
        }
        let id = BiblePresentationId::new();
        let now = Utc::now();
        let active = bible_presentation::ActiveModel {
            id: Set(id.to_string()),
            name: Set(trimmed.to_string()),
            created_at: Set(now.into()),
        };
        bible_presentation::Entity::insert(active)
            .exec(&self.db)
            .await?;

        Ok(BiblePresentation {
            id,
            name: trimmed.to_string(),
            slides: Vec::new(),
            created_at: now,
        })
    }

    #[instrument(skip_all)]
    pub async fn rename_bible_presentation(
        &self,
        id: BiblePresentationId,
        name: &str,
    ) -> anyhow::Result<()> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("bible presentation name cannot be empty"));
        }
        let result = bible_presentation::Entity::update_many()
            .col_expr(bible_presentation::Column::Name, SeaExpr::value(trimmed))
            .filter(bible_presentation::Column::Id.eq(id.to_string()))
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("bible presentation not found"));
        }
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn delete_bible_presentation(&self, id: BiblePresentationId) -> anyhow::Result<()> {
        let result = bible_presentation::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            return Err(anyhow!("bible presentation not found"));
        }
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn replace_bible_presentation_slides(
        &self,
        id: BiblePresentationId,
        slides: &[BiblePresentationSlide],
    ) -> anyhow::Result<()> {
        let id_str = id.to_string();
        let txn = self.db.begin().await?;

        if bible_presentation::Entity::find_by_id(id_str.clone())
            .one(&txn)
            .await?
            .is_none()
        {
            return Err(anyhow!("bible presentation not found"));
        }

        bible_slide::Entity::delete_many()
            .filter(bible_slide::Column::PresentationId.eq(id_str.clone()))
            .exec(&txn)
            .await?;

        for (index, slide) in slides.iter().enumerate() {
            let mut normalized = slide.clone();
            normalized.order = index as u32;
            let active = bible_slide_to_active_model(&normalized, &id_str)?;
            bible_slide::Entity::insert(active).exec(&txn).await?;
        }

        txn.commit().await?;
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn append_bible_presentation_slides(
        &self,
        id: BiblePresentationId,
        slides: &[BiblePresentationSlide],
    ) -> anyhow::Result<BiblePresentation> {
        let id_str = id.to_string();
        let txn = self.db.begin().await?;

        if bible_presentation::Entity::find_by_id(id_str.clone())
            .one(&txn)
            .await?
            .is_none()
        {
            return Err(anyhow!("bible presentation not found"));
        }

        let existing_count = bible_slide::Entity::find()
            .filter(bible_slide::Column::PresentationId.eq(id_str.clone()))
            .count(&txn)
            .await? as u32;

        for (index, slide) in slides.iter().enumerate() {
            let mut normalized = slide.clone();
            normalized.order = existing_count + index as u32;
            let active = bible_slide_to_active_model(&normalized, &id_str)?;
            bible_slide::Entity::insert(active).exec(&txn).await?;
        }

        txn.commit().await?;

        self.fetch_bible_presentation(id)
            .await?
            .ok_or_else(|| anyhow!("bible presentation disappeared after append"))
    }
}

fn model_to_bible_slide(model: bible_slide::Model) -> anyhow::Result<BiblePresentationSlide> {
    let uuid =
        Uuid::parse_str(&model.id).map_err(|_| RepositoryError::InvalidUuid(model.id.clone()))?;
    let metadata = match model.metadata_json.as_deref() {
        Some(raw) if !raw.trim().is_empty() => {
            match serde_json::from_str::<BibleSlideMetadata>(raw) {
                Ok(meta) => Some(meta),
                Err(err) => {
                    tracing::warn!(
                        slide_id = %model.id,
                        error = %err,
                        raw = %raw,
                        "failed to deserialize bible slide metadata, returning None"
                    );
                    None
                }
            }
        }
        _ => None,
    };
    let main = SlideText::new(model.main_text).map_err(RepositoryError::from)?;
    let secondary = SlideText::new(model.secondary_text).map_err(RepositoryError::from)?;
    Ok(BiblePresentationSlide {
        id: BibleSlideId::from_uuid(uuid),
        order: model.slide_order.max(0) as u32,
        main,
        main_reference: model.main_reference,
        secondary,
        secondary_reference: model.secondary_reference,
        metadata,
    })
}

fn bible_slide_to_active_model(
    slide: &BiblePresentationSlide,
    presentation_id: &str,
) -> anyhow::Result<bible_slide::ActiveModel> {
    let metadata_json = match slide.metadata.as_ref() {
        Some(meta) => Some(serde_json::to_string(meta)?),
        None => None,
    };
    Ok(bible_slide::ActiveModel {
        id: Set(slide.id.to_string()),
        presentation_id: Set(presentation_id.to_string()),
        slide_order: Set(slide.order as i32),
        main_text: Set(slide.main.value().to_owned()),
        main_search: Set(fold_query(slide.main.value())),
        main_reference: Set(slide.main_reference.clone()),
        secondary_text: Set(slide.secondary.value().to_owned()),
        secondary_search: Set(fold_query(slide.secondary.value())),
        secondary_reference: Set(slide.secondary_reference.clone()),
        metadata_json: Set(metadata_json),
    })
}
