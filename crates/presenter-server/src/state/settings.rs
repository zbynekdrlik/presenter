use std::sync::atomic::Ordering;

use anyhow::anyhow;
use serde::Serialize;

use super::{AppState, COMPANION_FEATURE_KEY, COMPANION_PORT_KEY};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlags {
    pub companion_enabled: bool,
    pub companion_port: u16,
}

impl AppState {
    pub fn companion_enabled(&self) -> bool {
        self.companion_enabled.load(Ordering::SeqCst)
    }

    pub fn companion_port(&self) -> u16 {
        self.companion_port.load(Ordering::SeqCst)
    }

    pub fn feature_flags(&self) -> FeatureFlags {
        FeatureFlags {
            companion_enabled: self.companion_enabled(),
            companion_port: self.companion_port(),
        }
    }

    pub async fn set_companion_settings(&self, enabled: bool, port: u16) -> anyhow::Result<()> {
        if port == 0 {
            return Err(anyhow!("companion port must be between 1 and 65535"));
        }
        let previous_enabled = self.companion_enabled();
        let previous_port = self.companion_port();

        self.configure_companion_service(enabled, port).await?;

        if let Err(err) = self
            .repository
            .set_app_setting(COMPANION_PORT_KEY, &port.to_string())
            .await
        {
            let _ = self
                .configure_companion_service(previous_enabled, previous_port)
                .await;
            return Err(err);
        }

        if let Err(err) = self
            .repository
            .set_app_setting(COMPANION_FEATURE_KEY, if enabled { "1" } else { "0" })
            .await
        {
            let _ = self
                .repository
                .set_app_setting(COMPANION_PORT_KEY, &previous_port.to_string())
                .await;
            let _ = self
                .configure_companion_service(previous_enabled, previous_port)
                .await;
            return Err(err);
        }

        self.companion_enabled.store(enabled, Ordering::SeqCst);
        self.companion_port.store(port, Ordering::SeqCst);
        Ok(())
    }
}
