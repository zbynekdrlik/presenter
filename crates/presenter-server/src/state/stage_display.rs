//! Stage display, appearance, and design methods for `AppState`.

use super::stage::build_stage_snapshot;
use super::AppState;
use crate::live::LiveEvent;
use presenter_core::{
    StageDesign, StageDisplayLayout, StageDisplaySnapshot, DEFAULT_STAGE_LAYOUT_CODE,
};

impl AppState {
    // Stage display methods
    pub async fn stage_display_snapshot(
        &self,
        layout_code: &str,
    ) -> anyhow::Result<Option<StageDisplaySnapshot>> {
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == layout_code);
        let Some(layout) = layout else {
            return Ok(None);
        };
        let Some(context) = self.build_stage_context().await? else {
            return Ok(None);
        };
        Ok(Some(build_stage_snapshot(layout, &context)))
    }

    pub async fn selected_stage_display_snapshot(
        &self,
    ) -> anyhow::Result<Option<StageDisplaySnapshot>> {
        let code = {
            let guard = self.stage_layout.read().await;
            guard.clone()
        };
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
        self.broadcast_stage_snapshots().await?;
        Ok(layout)
    }

    pub async fn stage_displays(&self) -> anyhow::Result<Vec<StageDisplayLayout>> {
        Ok(StageDisplayLayout::built_in())
    }

    // Stage appearance settings
    pub async fn get_stage_appearance(
        &self,
        layout: &str,
    ) -> anyhow::Result<crate::router::stage::StageAppearance> {
        let key = format!("stage-appearance:{layout}");
        match self.repository.get_app_setting(&key).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(crate::router::stage::StageAppearance::default_for(layout)),
        }
    }

    pub async fn set_stage_appearance(
        &self,
        layout: &str,
        appearance: crate::router::stage::StageAppearance,
    ) -> anyhow::Result<()> {
        let key = format!("stage-appearance:{layout}");
        let json = serde_json::to_string(&appearance)?;
        self.repository.set_app_setting(&key, &json).await?;
        self.live_hub.publish(LiveEvent::StageAppearance {
            layout: layout.to_string(),
            appearance,
        });
        Ok(())
    }

    // Stage design methods (visual box-based layout)
    pub async fn get_stage_design(&self, layout: &str) -> anyhow::Result<StageDesign> {
        let key = format!("stage-design:{layout}");
        match self.repository.get_app_setting(&key).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(StageDesign::default_for(layout)),
        }
    }

    pub async fn set_stage_design(&self, layout: &str, design: StageDesign) -> anyhow::Result<()> {
        let key = format!("stage-design:{layout}");
        let json = serde_json::to_string(&design)?;
        self.repository.set_app_setting(&key, &json).await?;
        self.live_hub.publish(LiveEvent::StageDesign {
            layout: layout.to_string(),
            design,
        });
        Ok(())
    }

    pub async fn reset_stage_design(&self, layout: &str) -> anyhow::Result<StageDesign> {
        let key = format!("stage-design:{layout}");
        self.repository.delete_app_setting(&key).await?;
        let design = StageDesign::default_for(layout);
        self.live_hub.publish(LiveEvent::StageDesign {
            layout: layout.to_string(),
            design: design.clone(),
        });
        Ok(design)
    }

    /// Get stage design with shared status bar settings from worship-snv.
    /// Status bar elements (clock, live indicator, connection status) are
    /// managed in worship-snv and automatically synced to other layouts.
    pub async fn get_stage_design_with_shared_status(
        &self,
        layout: &str,
    ) -> anyhow::Result<StageDesign> {
        let mut design = self.get_stage_design(layout).await?;

        // If this is already worship-snv, return as-is
        if layout == DEFAULT_STAGE_LAYOUT_CODE {
            return Ok(design);
        }

        // Get master design to extract status bar settings
        let master = self.get_stage_design(DEFAULT_STAGE_LAYOUT_CODE).await?;

        // Remove status bar boxes from current design
        design.boxes.retain(|b| !b.box_type.is_status_bar());

        // Add status bar boxes from master
        design.boxes.extend(
            master
                .boxes
                .into_iter()
                .filter(|b| b.box_type.is_status_bar()),
        );

        Ok(design)
    }
}
