//! Per-slide stage-layout marker persistence (#515).
//!
//! One row per marked slide: triggering that slide switches the stage display
//! to the stored layout code. Slides without a row leave the layout untouched.

use crate::entities::slide_stage_layout;
use anyhow::Context;
use presenter_core::{PresentationId, SlideId};
use sea_orm::{sea_query::OnConflict, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
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

    /// Sweep marker rows whose slide no longer exists — library deletes
    /// cascade slides away without touching this table, and library
    /// re-imports assign fresh slide UUIDs. Called after library delete /
    /// re-import so orphan rows never accumulate (#515).
    #[instrument(skip_all)]
    pub async fn prune_orphan_slide_stage_layouts(&self) -> anyhow::Result<u64> {
        let backend = self.db.get_database_backend();
        let result = self
            .db
            .execute(sea_orm::Statement::from_string(
                backend,
                "DELETE FROM slide_stage_layouts \
                 WHERE slide_id NOT IN (SELECT id FROM slides)",
            ))
            .await
            .context("failed to prune orphan slide stage layouts")?;
        Ok(result.rows_affected())
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
    use presenter_core::{PresentationId, Slide, SlideContent, SlideGroup, SlideId, SlideText};

    fn sample_slide(order: u32) -> Slide {
        Slide::new(
            order,
            SlideContent::new(
                SlideText::new("Main").unwrap(),
                SlideText::new("Translation").unwrap(),
                SlideText::new("Stage").unwrap(),
                Some(SlideGroup::new("Intro")),
            ),
        )
        .with_id(SlideId::new())
    }

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

    /// #515 review finding: markers must not survive as orphans on the
    /// library-delete path (FK cascade removes the slides; nothing else
    /// touches slide_stage_layouts) …
    #[tokio::test]
    async fn delete_library_prunes_its_slide_markers() {
        let repo = Repository::connect_in_memory().await.expect("db");
        let library = repo.create_library("Marker Lib").await.unwrap();
        let slides = [sample_slide(0)];
        let (_, _, presentation) = repo
            .create_presentation(library.id, "Marked", Some(&slides))
            .await
            .unwrap();
        let slide_id = presentation.slides[0].id;
        repo.set_slide_stage_layout(presentation.id, slide_id, "fulltext")
            .await
            .unwrap();

        repo.delete_library(library.id).await.unwrap();

        assert_eq!(repo.get_slide_stage_layout(slide_id).await.unwrap(), None);
    }

    /// … and the Import-Data --purge path clears the whole marker table
    /// inside the same transaction that purges the slides.
    #[tokio::test]
    async fn purge_presentation_content_clears_all_markers() {
        let repo = Repository::connect_in_memory().await.expect("db");
        let library = repo.create_library("Purge Lib").await.unwrap();
        let slides = [sample_slide(0)];
        let (_, _, presentation) = repo
            .create_presentation(library.id, "Marked", Some(&slides))
            .await
            .unwrap();
        let slide_id = presentation.slides[0].id;
        repo.set_slide_stage_layout(presentation.id, slide_id, "timer")
            .await
            .unwrap();

        repo.purge_presentation_content().await.unwrap();

        assert_eq!(repo.get_slide_stage_layout(slide_id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn prune_removes_rows_whose_slide_no_longer_exists() {
        let repo = Repository::connect_in_memory().await.expect("db");
        // A marker pointing at a slide id that exists in NO slides row (the
        // in-memory DB seeds no slides) is an orphan by definition.
        let (pres, slide) = ids();
        repo.set_slide_stage_layout(pres, slide, "fulltext")
            .await
            .unwrap();

        let pruned = repo.prune_orphan_slide_stage_layouts().await.unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(repo.get_slide_stage_layout(slide).await.unwrap(), None);
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
