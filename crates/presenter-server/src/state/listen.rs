//! #852 C1a: `listen.mics` ↔ the receiver. Startup only READS it (never in
//! validate mode); `set_listen_mics` is the only writer (+ a best-effort
//! `settings_audit` row, the `ai-settings` pattern, #661).

use super::AppState;
use crate::listen::{vban_bind_addr, MIC_TARGET};
use presenter_core::listen::mics::{validate_mics, MicConfig, LISTEN_MICS_KEY};
use presenter_core::listen::status::ListenStatus;
use presenter_persistence::SettingsAuditSource;
use std::net::SocketAddr;

/// `settings_audit.setting_table` of the row (as `ai-settings`).
const LISTEN_AUDIT_TABLE: &str = "app_settings";

/// A stored `listen.mics` that is not a valid list: a 500, never replaced silently.
#[derive(Debug, thiserror::Error)]
#[error("Uložené nastavenie mikrofónov (listen.mics) je poškodené: {0}")]
struct StoredMicsCorrupt(String);

impl AppState {
    /// The stored list (empty when never saved).
    pub(crate) async fn listen_mics(&self) -> anyhow::Result<Vec<MicConfig>> {
        let Some(raw) = self.repository.get_app_setting(LISTEN_MICS_KEY).await? else {
            return Ok(Vec::new());
        };
        let mics: Vec<MicConfig> =
            serde_json::from_str(&raw).map_err(|err| StoredMicsCorrupt(err.to_string()))?;
        validate_mics(mics).map_err(|err| anyhow::Error::new(StoredMicsCorrupt(err.to_string())))
    }

    /// `PUT /listen/mics`: validate (→ `MicsValidationError`, 422), store, apply, audit.
    pub(crate) async fn set_listen_mics(
        &self,
        mics: Vec<MicConfig>,
        actor: &str,
    ) -> anyhow::Result<Vec<MicConfig>> {
        let mics = validate_mics(mics)?;
        let _save = self.listen.save_guard().await;
        let before = self.repository.get_app_setting(LISTEN_MICS_KEY).await?;
        self.repository
            .set_app_setting(LISTEN_MICS_KEY, &serde_json::to_string(&mics)?)
            .await?;
        self.listen.apply(mics.clone());
        tracing::info!(
            target: MIC_TARGET,
            mics = mics.len(),
            actor,
            "listen mics saved"
        );
        self.audit_listen_mics(before, &mics, actor).await;
        Ok(mics)
    }

    /// `GET /listen/status`.
    pub(crate) fn listen_status(&self) -> ListenStatus {
        self.listen.status()
    }

    /// Load `listen.mics` into the receiver and start its socket task (once).
    /// The read and the apply run under the save lock: a save that comes
    /// during startup waits and is applied after it, never overwritten by
    /// this older read (review 1a).
    pub(crate) async fn start_listen(&self, bind: SocketAddr) -> bool {
        {
            let _save = self.listen.save_guard().await;
            match self.listen_mics().await {
                Ok(mics) => self.listen.apply(mics),
                Err(err) => tracing::warn!(
                    target: MIC_TARGET,
                    error = %err,
                    "listen.mics cannot be read — no microphone is received until it is saved again"
                ),
            }
        }
        self.listen.start(bind)
    }

    /// From `spawn_background_tasks` (never in validate mode).
    pub(super) fn spawn_listen_task(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            state.start_listen(vban_bind_addr()).await;
        });
    }

    /// Best effort: the write already succeeded, an audit failure only WARNs.
    async fn audit_listen_mics(&self, before: Option<String>, after: &[MicConfig], actor: &str) {
        let before = before.map(|raw| {
            serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::Value::String(raw))
        });
        let after = serde_json::to_value(after).unwrap_or_default();
        let recorded = self
            .repository
            .record_settings_audit(
                LISTEN_AUDIT_TABLE,
                LISTEN_MICS_KEY,
                SettingsAuditSource::HttpSetter,
                actor,
                before,
                after,
            )
            .await;
        if let Err(err) = recorded {
            tracing::warn!(
                target: MIC_TARGET,
                ?err,
                "failed to record the listen.mics audit row (the save itself succeeded)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::listen::status::SocketState;
    use std::time::Duration;

    #[tokio::test]
    async fn the_startup_read_waits_for_a_save_in_progress() {
        let state = AppState::in_memory().await.unwrap();
        let save = state.listen.save_guard().await;
        let starter = {
            let state = state.clone();
            tokio::spawn(async move { state.start_listen("127.0.0.1:0".parse().unwrap()).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            state.listen_status().socket.state,
            SocketState::NotStarted,
            "start_listen waits for the save lock before it reads listen.mics"
        );
        drop(save);
        let started = tokio::time::timeout(Duration::from_secs(5), starter)
            .await
            .unwrap()
            .unwrap();
        assert!(started);
        assert_eq!(
            state.listen_status().socket.bind_addr.as_deref(),
            Some("127.0.0.1:0")
        );
    }
}
