//! Stage display methods for `AppState`.

use super::stage::build_stage_snapshot;
use super::AppState;
use crate::live::LiveEvent;
use presenter_core::{StageDisplayLayout, StageDisplaySnapshot, API_STAGE_LAYOUT_CODE};

impl AppState {
    // Stage display methods
    pub async fn stage_display_snapshot(
        &self,
        layout_code: &str,
    ) -> anyhow::Result<Option<StageDisplaySnapshot>> {
        if layout_code == "api" {
            return Ok(Some(self.api_stage_snapshot().await));
        }
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == layout_code);
        let Some(layout) = layout else {
            return Ok(None);
        };
        let Some(context) = self.build_stage_context().await? else {
            return Ok(None);
        };
        let context = self.enrich_stage_context(&context).await;
        Ok(Some(build_stage_snapshot(layout, &context)))
    }

    pub async fn selected_stage_display_snapshot(
        &self,
    ) -> anyhow::Result<Option<StageDisplaySnapshot>> {
        let code = {
            let guard = self.stage_layout.read().await;
            guard.clone()
        };
        if code == "api" {
            return Ok(Some(self.api_stage_snapshot().await));
        }
        self.stage_display_snapshot(&code).await
    }

    pub async fn stage_layout_code(&self) -> String {
        self.stage_layout.read().await.clone()
    }

    pub async fn set_stage_layout_code(&self, code: &str) -> anyhow::Result<StageDisplayLayout> {
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == code)
            .ok_or_else(|| anyhow::anyhow!("unknown stage layout: {code}"))?;
        {
            let mut guard = self.stage_layout.write().await;
            if *guard == layout.code {
                return Ok(layout);
            }
            *guard = layout.code.clone();
        }
        self.live_hub.publish(LiveEvent::StageLayout {
            code: layout.code.clone(),
        });
        if layout.code == API_STAGE_LAYOUT_CODE {
            // Issue #281: when switching TO api, publish the stored
            // api_stage snapshot so the operator preview reflects the
            // most recent PUT instead of waiting for the next one.
            // `broadcast_stage_snapshots` short-circuits on api layout
            // anyway (see broadcasting.rs::publish_stage_context), so we
            // replace it with the api snapshot publish here.
            let snapshot = self.api_stage_snapshot().await;
            self.live_hub.publish(LiveEvent::Stage { snapshot });
        } else {
            self.broadcast_stage_snapshots().await?;
        }
        Ok(layout)
    }

    pub async fn stage_displays(&self) -> anyhow::Result<Vec<StageDisplayLayout>> {
        Ok(StageDisplayLayout::built_in())
    }
}
