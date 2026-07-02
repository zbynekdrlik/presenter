//! Per-slide stage-layout marker persistence (#515).
//!
//! One row per marked slide: triggering that slide switches the stage display
//! to the stored layout code. Slides without a row leave the layout untouched.

use crate::entities::slide_stage_layout;
use anyhow::Context;
use presenter_core::{PresentationId, SlideId};
use sea_orm::{sea_query::OnConflict, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::collections::HashMap;
use tracing::instrument;

use super::Repository;

impl Repository {
    /// Layout marker for one slide, if any.
    #[instrument(skip_all)]
    pub async fn get_slide_stage_layout(
        &self,
        slide_id: SlideId,
    ) -> anyhow::Result<Option<String>> {
        let row = slide_stage_layout::Entity::find_by_id(slide_id.to_string())
            .one(&self.db)
            .await
            .context("failed to load slide stage layout")?;
        Ok(row.map(|r| r.layout_code))
    }

    /// Upsert the layout marker for a slide.
    #[instrument(skip_all)]
    pub async fn set_slide_stage_layout(
        &self,
        presentation_id: PresentationId,
        slide_id: SlideId,
        layout_code: &str,
    ) -> anyhow::Result<()> {
        let model = slide_stage_layout::ActiveModel {
            slide_id: Set(slide_id.to_string()),
            presentation_id: Set(presentation_id.to_string()),
            layout_code: Set(layout_code.to_string()),
        };
        slide_stage_layout::Entity::insert(model)
            .on_conflict(
                OnConflict::column(slide_stage_layout::Column::SlideId)
                    .update_columns([
                        slide_stage_layout::Column::PresentationId,
                        slide_stage_layout::Column::LayoutCode,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await
            .context("failed to upsert slide stage layout")?;
        Ok(())
    }

    /// Remove a slide's layout marker (no-op when none exists).
    #[instrument(skip_all)]
    pub async fn clear_slide_stage_layout(&self, slide_id: SlideId) -> anyhow::Result<()> {
        slide_stage_layout::Entity::delete_by_id(slide_id.to_string())
            .exec(&self.db)
            .await
            .context("failed to clear slide stage layout")?;
        Ok(())
    }

    /// All layout markers of a presentation as `slide_id → layout_code`
    /// (for the operator UI's per-slide indicators).
    #[instrument(skip_all)]
    pub async fn list_slide_stage_layouts(
        &self,
        presentation_id: PresentationId,
    ) -> anyhow::Result<HashMap<String, String>> {
        let rows = slide_stage_layout::Entity::find()
            .filter(slide_stage_layout::Column::PresentationId.eq(presentation_id.to_string()))
            .all(&self.db)
            .await
            .context("failed to list slide stage layouts")?;
        Ok(rows
            .into_iter()
            .map(|r| (r.slide_id, r.layout_code))
            .collect())
    }

    /// Remove all markers of a presentation (called when it is deleted).
    #[instrument(skip_all)]
    pub async fn clear_slide_stage_layouts_for_presentation(
        &self,
        presentation_id: PresentationId,
    ) -> anyhow::Result<()> {
        slide_stage_layout::Entity::delete_many()
            .filter(slide_stage_layout::Column::PresentationId.eq(presentation_id.to_string()))
            .exec(&self.db)
            .await
            .context("failed to clear presentation slide stage layouts")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::repository::Repository;
    use presenter_core::{PresentationId, SlideId};

    fn ids() -> (PresentationId, SlideId) {
        (PresentationId::new(), SlideId::new())
    }

    #[tokio::test]
    async fn get_returns_none_without_marker() {
        let repo = Repository::connect_in_memory().await.expect("db");
        let (_, slide) = ids();
        assert_eq!(repo.get_slide_stage_layout(slide).await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_then_get_roundtrips() {
        let repo = Repository::connect_in_memory().await.expect("db");
        let (pres, slide) = ids();
        repo.set_slide_stage_layout(pres, slide, "fulltext")
            .await
            .unwrap();
        assert_eq!(
            repo.get_slide_stage_layout(slide).await.unwrap(),
            Some("fulltext".to_string())
        );
    }

    #[tokio::test]
    async fn set_overwrites_existing_marker() {
        let repo = Repository::connect_in_memory().await.expect("db");
        let (pres, slide) = ids();
        repo.set_slide_stage_layout(pres, slide, "fulltext")
            .await
            .unwrap();
        repo.set_slide_stage_layout(pres, slide, "timer")
            .await
            .unwrap();
        assert_eq!(
            repo.get_slide_stage_layout(slide).await.unwrap(),
            Some("timer".to_string())
        );
    }

    #[tokio::test]
    async fn clear_removes_marker_and_is_idempotent() {
        let repo = Repository::connect_in_memory().await.expect("db");
        let (pres, slide) = ids();
        repo.set_slide_stage_layout(pres, slide, "fulltext")
            .await
            .unwrap();
        repo.clear_slide_stage_layout(slide).await.unwrap();
        assert_eq!(repo.get_slide_stage_layout(slide).await.unwrap(), None);
        // Clearing again must not error.
        repo.clear_slide_stage_layout(slide).await.unwrap();
    }

    #[tokio::test]
    async fn list_returns_only_presentation_markers() {
        let repo = Repository::connect_in_memory().await.expect("db");
        let (pres_a, slide_a) = ids();
        let (pres_b, slide_b) = ids();
        repo.set_slide_stage_layout(pres_a, slide_a, "fulltext")
            .await
            .unwrap();
        repo.set_slide_stage_layout(pres_b, slide_b, "timer")
            .await
            .unwrap();

        let map = repo.list_slide_stage_layouts(pres_a).await.unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(
            map.get(&slide_a.to_string()).map(String::as_str),
            Some("fulltext")
        );
    }

    #[tokio::test]
    async fn clear_for_presentation_removes_all_its_markers() {
        let repo = Repository::connect_in_memory().await.expect("db");
        let (pres_a, slide_a) = ids();
        let (pres_b, slide_b) = ids();
        repo.set_slide_stage_layout(pres_a, slide_a, "fulltext")
            .await
            .unwrap();
        repo.set_slide_stage_layout(pres_b, slide_b, "timer")
            .await
            .unwrap();

        repo.clear_slide_stage_layouts_for_presentation(pres_a)
            .await
            .unwrap();
        assert!(repo
            .list_slide_stage_layouts(pres_a)
            .await
            .unwrap()
            .is_empty());
        // Other presentations' markers are untouched.
        assert_eq!(
            repo.get_slide_stage_layout(slide_b).await.unwrap(),
            Some("timer".to_string())
        );
    }
}
