//! Stage display methods for `AppState`.

use super::stage::build_stage_snapshot;
use super::AppState;
use crate::live::LiveEvent;
use presenter_core::{
    StageDisplayLayout, StageDisplaySnapshot, API_STAGE_LAYOUT_CODE, DEFAULT_STAGE_LAYOUT_CODE,
};

/// `app_settings` key under which the operator-selected stage layout code is
/// persisted so it survives a server restart/deploy (issue #384). Stored via
/// the same generic k/v mechanism as the Companion feature flags
/// (`feature.companion.*`), which intentionally bypass `settings_audit` — only
/// the typed singleton settings tables (osc/ableset/resolume/video_source) are
/// audited. Reading this key on startup is a pure read; on an unchanged DB it
/// writes nothing, preserving the second-startup-no-audit invariant.
pub(crate) const STAGE_LAYOUT_KEY: &str = "feature.stage.layout";

impl AppState {
    /// Load the persisted stage layout code from the database, falling back to
    /// [`DEFAULT_STAGE_LAYOUT_CODE`] when none is stored or the stored value no
    /// longer matches a built-in layout. Pure read — never writes.
    pub(crate) async fn load_persisted_stage_layout(&self) -> String {
        match self.repository().get_app_setting(STAGE_LAYOUT_KEY).await {
            Ok(Some(code)) => {
                // Guard against a stored code that no longer exists (e.g. a
                // layout removed in a later release) — fall back to default
                // rather than serve an unknown layout.
                let known = code == API_STAGE_LAYOUT_CODE
                    || StageDisplayLayout::built_in()
                        .into_iter()
                        .any(|layout| layout.code == code);
                if known {
                    code
                } else {
                    tracing::warn!(
                        stored = %code,
                        "persisted stage layout no longer recognized — falling back to default"
                    );
                    DEFAULT_STAGE_LAYOUT_CODE.to_string()
                }
            }
            Ok(None) => DEFAULT_STAGE_LAYOUT_CODE.to_string(),
            Err(err) => {
                tracing::warn!(
                    ?err,
                    "failed to load persisted stage layout — falling back to default"
                );
                DEFAULT_STAGE_LAYOUT_CODE.to_string()
            }
        }
    }

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
        if code == "camera-crew" {
            return Err(anyhow::anyhow!(
                "'camera-crew' is not an operator-selectable layout; it is served only at /ui/camera"
            ));
        }
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
        // Persist the new layout so it survives a restart/deploy (#384). The
        // in-memory RwLock above is the live source of truth for broadcasts;
        // the DB write only seeds the RwLock on the next startup. A failed
        // write must NOT abort the live layout switch, so log and continue.
        if let Err(err) = self
            .repository()
            .set_app_setting(STAGE_LAYOUT_KEY, &layout.code)
            .await
        {
            tracing::warn!(
                ?err,
                code = %layout.code,
                "failed to persist stage layout — it will reset to default on next restart"
            );
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
        // camera-crew is published always but is not user-selectable from the
        // operator UI — it's accessed via /ui/camera only. Hide it from the
        // layout picker.
        Ok(StageDisplayLayout::built_in()
            .into_iter()
            .filter(|l| l.code != "camera-crew")
            .collect())
    }
}
