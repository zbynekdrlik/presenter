//! #874: the Prepared tab's order and its archive. Every write publishes
//! `LiveEvent::BibleSlidesChanged`, the event every open list already
//! re-fetches on; a reorder names no single presentation, so it sends an
//! empty id (no cache holds slides under it).

use presenter_core::{BiblePresentationId, BiblePresentationSummary};
use tracing::info;

use super::AppState;
use crate::live::LiveEvent;

impl AppState {
    pub async fn list_archived_bible_presentations(
        &self,
    ) -> anyhow::Result<Vec<BiblePresentationSummary>> {
        self.repository
            .list_archived_bible_presentation_summaries()
            .await
    }

    pub async fn reorder_bible_presentations(
        &self,
        ids: &[BiblePresentationId],
    ) -> anyhow::Result<()> {
        self.repository.reorder_bible_presentations(ids).await?;
        info!(
            target: "presenter::bible::prepared",
            count = ids.len(),
            "bible presentations reordered"
        );
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: String::new(),
        });
        Ok(())
    }

    pub async fn archive_bible_presentation(&self, id: BiblePresentationId) -> anyhow::Result<()> {
        self.repository.archive_bible_presentation(id).await?;
        info!(target: "presenter::bible::prepared", presentation_id = %id, "bible presentation archived");
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: id.to_string(),
        });
        Ok(())
    }

    pub async fn restore_bible_presentation(&self, id: BiblePresentationId) -> anyhow::Result<()> {
        self.repository.restore_bible_presentation(id).await?;
        info!(target: "presenter::bible::prepared", presentation_id = %id, "bible presentation restored");
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: id.to_string(),
        });
        Ok(())
    }
}
