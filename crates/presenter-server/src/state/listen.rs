//! #852 C1a: `listen.mics` ↔ the receiver. Startup only READS it (never in
//! validate mode); `set_listen_mics` is the only writer (+ a best-effort
//! `settings_audit` row, the `ai-settings` pattern, #661). C1b: the recorder's
//! per-day toggle `listen.recorder` the same way — read at startup, written by
//! `set_listen_recorder`.

use super::AppState;
use crate::listen::recorder::RECORDER_TARGET;
use crate::listen::{vban_bind_addr, MIC_TARGET};
use presenter_core::listen::mics::{validate_mics, MicConfig, LISTEN_MICS_KEY};
use presenter_core::listen::recorder::{RecorderSetting, LISTEN_RECORDER_KEY};
use presenter_core::listen::status::ListenStatus;
use presenter_persistence::SettingsAuditSource;
use std::net::SocketAddr;

/// `settings_audit.setting_table` of the row (as `ai-settings`).
const LISTEN_AUDIT_TABLE: &str = "app_settings";

/// A stored `listen.mics` that is not a valid list: a 500, never replaced silently.
#[derive(Debug, thiserror::Error)]
#[error("Uložené nastavenie mikrofónov (listen.mics) je poškodené: {0}")]
struct StoredMicsCorrupt(String);

/// A stored `listen.recorder` that is not a valid row: the recorder stays off.
#[derive(Debug, thiserror::Error)]
#[error("Uložené nastavenie nahrávania (listen.recorder) je poškodené: {0}")]
struct StoredRecorderCorrupt(String);

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
        let after = serde_json::to_value(&mics).unwrap_or_default();
        self.audit_listen_setting(LISTEN_MICS_KEY, before, after, actor)
            .await;
        Ok(mics)
    }

    /// The stored toggle (off when never saved).
    async fn listen_recorder(&self) -> anyhow::Result<RecorderSetting> {
        let Some(raw) = self.repository.get_app_setting(LISTEN_RECORDER_KEY).await? else {
            return Ok(RecorderSetting::default());
        };
        serde_json::from_str(&raw)
            .map_err(|err| anyhow::Error::new(StoredRecorderCorrupt(err.to_string())))
    }

    /// `PUT /listen/recorder`: on = today's local date, off = none; stored,
    /// applied, audited.
    pub(crate) async fn set_listen_recorder(
        &self,
        enabled: bool,
        actor: &str,
    ) -> anyhow::Result<RecorderSetting> {
        let setting = RecorderSetting {
            enabled_for: enabled.then(|| self.listen.today()),
        };
        let _save = self.listen.save_guard().await;
        let before = self.repository.get_app_setting(LISTEN_RECORDER_KEY).await?;
        self.repository
            .set_app_setting(LISTEN_RECORDER_KEY, &serde_json::to_string(&setting)?)
            .await?;
        self.listen.apply_recorder(setting.enabled_for);
        match setting.enabled_for {
            Some(day) => tracing::info!(
                target: RECORDER_TARGET,
                %day,
                actor,
                "listen recorder switched on for today"
            ),
            None => tracing::info!(target: RECORDER_TARGET, actor, "listen recorder switched off"),
        }
        let after = serde_json::to_value(setting).unwrap_or_default();
        self.audit_listen_setting(LISTEN_RECORDER_KEY, before, after, actor)
            .await;
        Ok(setting)
    }

    /// `GET /listen/status`.
    pub(crate) fn listen_status(&self) -> ListenStatus {
        self.listen.status()
    }

    /// Load `listen.mics` and the recorder toggle into the receiver and start its
    /// socket task (once).
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
            match self.listen_recorder().await {
                Ok(setting) => self.listen.apply_recorder(setting.enabled_for),
                Err(err) => tracing::warn!(
                    target: RECORDER_TARGET,
                    error = %err,
                    "listen.recorder cannot be read — the recorder stays off until it is switched again"
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
    async fn audit_listen_setting(
        &self,
        key: &str,
        before: Option<String>,
        after: serde_json::Value,
        actor: &str,
    ) {
        let before = before.map(|raw| {
            serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::Value::String(raw))
        });
        let recorded = self
            .repository
            .record_settings_audit(
                LISTEN_AUDIT_TABLE,
                key,
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
                key,
                "failed to record a listen setting's audit row (the save itself succeeded)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::recorder::RecorderEnv;
    use chrono::NaiveDateTime;
    use presenter_core::listen::status::SocketState;
    use std::sync::Arc;
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

    fn fixed(at: &'static str) -> RecorderEnv {
        let now: NaiveDateTime = at.parse().unwrap();
        RecorderEnv {
            root: std::env::temp_dir().join("presenter-recorder-never-written"),
            clock: Arc::new(move || now),
            first_pass_delay: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn the_startup_resumes_a_recorder_switched_on_today_but_not_yesterdays() {
        let row = r#"{"enabledFor":"2026-10-18"}"#;
        let same_day = AppState::in_memory().await.unwrap();
        same_day
            .listen
            .set_recorder_env(fixed("2026-10-18T19:42:05"));
        same_day
            .repository()
            .set_app_setting(LISTEN_RECORDER_KEY, row)
            .await
            .unwrap();
        assert!(same_day.start_listen("127.0.0.1:0".parse().unwrap()).await);
        assert!(
            same_day.listen_status().recorder.enabled,
            "a restart the same day (an auto-deploy mid-sermon) records on"
        );
        let next_day = AppState::in_memory().await.unwrap();
        next_day
            .listen
            .set_recorder_env(fixed("2026-10-19T08:00:00"));
        next_day
            .repository()
            .set_app_setting(LISTEN_RECORDER_KEY, row)
            .await
            .unwrap();
        assert!(next_day.start_listen("127.0.0.1:0".parse().unwrap()).await);
        let status = next_day.listen_status().recorder;
        assert!(!status.enabled, "yesterday's switch never records today");
        assert_eq!(status.enabled_for, Some("2026-10-18".parse().unwrap()));
    }

    #[tokio::test]
    async fn a_corrupt_recorder_row_keeps_the_recorder_off() {
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .set_app_setting(LISTEN_RECORDER_KEY, "not a row")
            .await
            .unwrap();
        assert!(state.start_listen("127.0.0.1:0".parse().unwrap()).await);
        let status = state.listen_status().recorder;
        assert_eq!((status.enabled, status.enabled_for), (false, None));
    }
}
