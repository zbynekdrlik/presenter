//! OSC settings application and accessors for [`AppState`].
//!
//! Extracted from `state/mod.rs` (#486) to keep the central module under the
//! file-size cap. Behaviour is unchanged — these are the same `impl AppState`
//! methods, only relocated.

use super::AppState;
use crate::osc::{OscBridge, OscStatusSnapshot};
use presenter_core::{OscSettings, OscSettingsDraft};
use std::env;

impl AppState {
    pub(super) async fn apply_osc_settings(
        state: &Self,
        osc_bridge: &OscBridge,
    ) -> anyhow::Result<()> {
        let mut osc_settings = state.repository.get_osc_settings().await?;
        if let Ok(port_raw) = env::var("PRESENTER_OSC_LISTEN_PORT") {
            match port_raw.parse::<u16>() {
                Ok(port) if port != 0 && port != osc_settings.listen_port => {
                    let draft = OscSettingsDraft {
                        enabled: osc_settings.enabled,
                        listen_port: port,
                        address_pattern: osc_settings.address_pattern.clone(),
                        velocity_mode: osc_settings.velocity_mode,
                    };
                    osc_settings = state
                        .repository
                        .upsert_osc_settings(
                            &draft,
                            presenter_persistence::SettingsAuditSource::StartupDefault,
                            "system",
                        )
                        .await?;
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(value = %port_raw, ?err, "invalid PRESENTER_OSC_LISTEN_PORT")
                }
            }
        }
        if let Err(err) = osc_bridge
            .apply_settings(osc_settings.clone(), state.clone())
            .await
        {
            tracing::warn!(?err, "failed to initialise OSC listener");
        }
        Ok(())
    }

    // OSC methods
    pub async fn osc_settings(&self) -> anyhow::Result<OscSettings> {
        self.repository.get_osc_settings().await
    }

    pub async fn update_osc_settings(
        &self,
        draft: OscSettingsDraft,
        source: presenter_persistence::SettingsAuditSource,
        actor: &str,
    ) -> anyhow::Result<OscSettings> {
        let settings = self
            .repository
            .upsert_osc_settings(&draft, source, actor)
            .await?;
        self.osc_bridge
            .apply_settings(settings.clone(), self.clone())
            .await?;
        Ok(settings)
    }

    pub async fn osc_status_snapshot(&self) -> OscStatusSnapshot {
        self.osc_bridge.status().await
    }
}
