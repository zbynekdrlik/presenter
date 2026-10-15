//! #852 C1a: `listen.mics` ↔ the receiver. Startup only READS it (never in
//! validate mode); `set_listen_mics` is the only writer (+ a best-effort
//! `settings_audit` row, the `ai-settings` pattern, #661). C1b: the recorder's
//! per-day toggle `listen.recorder` the same way — read at startup, written by
//! `set_listen_recorder`, cleared when the clock steps back before its day.

use super::AppState;
use crate::listen::recorder::RECORDER_TARGET;
use crate::listen::{vban_bind_addr, MIC_TARGET};
use chrono::NaiveDate;
use presenter_core::listen::mics::{validate_mics, MicConfig, LISTEN_MICS_KEY};
use presenter_core::listen::recorder::{RecorderSetting, LISTEN_RECORDER_KEY};
use presenter_core::listen::status::ListenStatus;
use presenter_persistence::SettingsAuditSource;
use std::net::SocketAddr;
use tokio::sync::watch;

/// `settings_audit.setting_table` of the row (as `ai-settings`).
const LISTEN_AUDIT_TABLE: &str = "app_settings";
/// The WARN of a failed audit row, under the target of the setting it is about.
const AUDIT_FAILED: &str =
    "failed to record a listen setting's audit row (the save itself succeeded)";

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
        self.audit_listen_setting(
            LISTEN_MICS_KEY,
            SettingsAuditSource::HttpSetter,
            before,
            after,
            actor,
        )
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
        self.store_listen_recorder(setting).await?;
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
        self.audit_listen_setting(
            LISTEN_RECORDER_KEY,
            SettingsAuditSource::HttpSetter,
            before,
            after,
            actor,
        )
        .await;
        Ok(setting)
    }

    /// Write the toggle row.
    async fn store_listen_recorder(&self, setting: RecorderSetting) -> anyhow::Result<()> {
        let row = serde_json::to_string(&setting)?;
        self.repository
            .set_app_setting(LISTEN_RECORDER_KEY, &row)
            .await
    }

    /// Apply the toggle read at startup (the caller holds the save lock). A day
    /// AFTER today — switched on while the clock was ahead, corrected since —
    /// would start the recorder by itself on that day, with nobody having
    /// switched it on then (spec §19): it stays off, the row is cleared, one WARN.
    async fn apply_stored_recorder(&self, setting: RecorderSetting) {
        let today = self.listen.today();
        let Some(day) = setting.enabled_for.filter(|day| *day > today) else {
            self.listen.apply_recorder(setting.enabled_for);
            return;
        };
        self.listen.apply_recorder(None);
        tracing::warn!(
            target: RECORDER_TARGET,
            %day,
            %today,
            "listen.recorder names a day in the future — the recorder stays off and the row is cleared"
        );
        self.clear_recorder_row(setting).await;
    }

    /// Store `{"enabledFor": null}` over a row that named a day after today and
    /// audit it as the system's own write; best effort (a failure WARNs, the
    /// recorder is off in memory either way). No `SettingsAuditSource` variant
    /// fits a write the server makes on its own at runtime (the others are a
    /// person, a migration or the port drift), so this keeps the startup read's
    /// `StartupDefault` with actor `system`.
    async fn clear_recorder_row(&self, before: RecorderSetting) {
        let cleared = RecorderSetting::default();
        if let Err(err) = self.store_listen_recorder(cleared).await {
            tracing::warn!(
                target: RECORDER_TARGET,
                error = %err,
                "the future day could not be cleared from listen.recorder — it stays off until it is switched again"
            );
            return;
        }
        self.audit_listen_setting(
            LISTEN_RECORDER_KEY,
            SettingsAuditSource::StartupDefault,
            serde_json::to_string(&before).ok(),
            serde_json::to_value(cleared).unwrap_or_default(),
            "system",
        )
        .await;
    }

    /// The recorder's supervisor switched it off in memory because its day lay
    /// after today (NTP stepped the clock back while the server ran): clear the
    /// stored row too, unless a switch made since replaced it. The WARN was the
    /// supervisor's.
    async fn clear_future_recorder_row(&self, day: NaiveDate) {
        let _save = self.listen.save_guard().await;
        match self.listen_recorder().await {
            Ok(stored) if stored.enabled_for == Some(day) => self.clear_recorder_row(stored).await,
            Ok(_) => {}
            Err(err) => tracing::warn!(
                target: RECORDER_TARGET,
                error = %err,
                "listen.recorder cannot be read — the future day stays stored"
            ),
        }
    }

    /// Follow the supervisor's notices, each one the day it switched off.
    fn spawn_recorder_row_cleaner(&self, mut cleared: watch::Receiver<Option<NaiveDate>>) {
        let state = self.clone();
        tokio::spawn(async move {
            while cleared.changed().await.is_ok() {
                let day = *cleared.borrow_and_update();
                if let Some(day) = day {
                    state.clear_future_recorder_row(day).await;
                }
            }
        });
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
                Ok(setting) => self.apply_stored_recorder(setting).await,
                Err(err) => tracing::warn!(
                    target: RECORDER_TARGET,
                    error = %err,
                    "listen.recorder cannot be read — the recorder stays off until it is switched again"
                ),
            }
        }
        // subscribed before the supervisor exists: its first notice is never missed
        let cleared = self.listen.recorder_cleared();
        let started = self.listen.start(bind);
        if started {
            self.spawn_recorder_row_cleaner(cleared);
        }
        started
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
        source: SettingsAuditSource,
        before: Option<String>,
        after: serde_json::Value,
        actor: &str,
    ) {
        let before = before.map(|raw| {
            serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::Value::String(raw))
        });
        let recorded = self
            .repository
            .record_settings_audit(LISTEN_AUDIT_TABLE, key, source, actor, before, after)
            .await;
        if let Err(err) = recorded {
            // a tracing target is a constant: one call per target
            if key == LISTEN_RECORDER_KEY {
                tracing::warn!(target: RECORDER_TARGET, ?err, key, "{AUDIT_FAILED}");
            } else {
                tracing::warn!(target: MIC_TARGET, ?err, key, "{AUDIT_FAILED}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::recorder::RecorderEnv;
    use crate::tracing_capture::Capture;
    use chrono::NaiveDateTime;
    use presenter_core::listen::status::SocketState;
    use serde_json::json;
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

    /// Switched on while the clock was ahead, then NTP corrected it: the stored
    /// day is still to come, and on that day the recorder would start by itself.
    #[tokio::test]
    async fn a_stored_day_in_the_future_never_switches_the_recorder_on() {
        let capture = Capture::install();
        let state = AppState::in_memory().await.unwrap();
        state.listen.set_recorder_env(fixed("2026-10-18T19:42:05"));
        state
            .repository()
            .set_app_setting(LISTEN_RECORDER_KEY, r#"{"enabledFor":"2026-10-21"}"#)
            .await
            .unwrap();
        assert!(state.start_listen("127.0.0.1:0".parse().unwrap()).await);
        let status = state.listen_status().recorder;
        assert_eq!((status.enabled, status.enabled_for), (false, None));
        assert_eq!(
            state
                .repository()
                .get_app_setting(LISTEN_RECORDER_KEY)
                .await
                .unwrap()
                .as_deref(),
            Some(r#"{"enabledFor":null}"#),
            "the row is cleared, never left to switch the recorder on later"
        );
        let warned = capture.events_with_message("day in the future");
        assert_eq!(warned.len(), 1, "{warned:?}");
        assert_eq!(warned[0].target, RECORDER_TARGET);
        assert_eq!(warned[0].level, tracing::Level::WARN);
        assert_eq!(warned[0].field("day"), Some("2026-10-21"));
        let audit = state
            .repository()
            .list_settings_audit(Some("app_settings"), Some(LISTEN_RECORDER_KEY), None, 10)
            .await
            .unwrap();
        assert_eq!(audit.len(), 1, "one audit row for the clearing");
        assert_eq!(audit[0].actor, "system");
        assert_eq!(audit[0].source, SettingsAuditSource::StartupDefault);
        assert_eq!(
            audit[0].before_json,
            Some(json!({"enabledFor": "2026-10-21"}))
        );
        assert_eq!(audit[0].after_json, json!({"enabledFor": null}));
        state.listen.set_recorder_env(fixed("2026-10-21T08:00:00"));
        assert!(
            !state.listen_status().recorder.enabled,
            "the day the old row named arrives: still off"
        );
    }

    /// One microphone, as the settings card saves it.
    const GUEST_MIC: &str = r#"[{"id": "guest", "name": "Hosť", "streamName": "GUEST_EN",
        "senderIp": null, "channel": "left", "language": "en", "role": "guest"}]"#;

    async fn stored_recorder(state: &AppState) -> Option<String> {
        state
            .repository()
            .get_app_setting(LISTEN_RECORDER_KEY)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn switching_on_stores_todays_local_date_and_switching_off_stores_null() {
        let state = AppState::in_memory().await.unwrap();
        state.listen.set_recorder_env(fixed("2026-10-18T19:42:05"));
        let on = state.set_listen_recorder(true, "test").await.unwrap();
        assert_eq!(
            on,
            RecorderSetting {
                enabled_for: Some("2026-10-18".parse().unwrap())
            }
        );
        assert_eq!(
            stored_recorder(&state).await.as_deref(),
            Some(r#"{"enabledFor":"2026-10-18"}"#)
        );
        assert!(state.listen_status().recorder.enabled);
        let off = state.set_listen_recorder(false, "test").await.unwrap();
        assert_eq!(off, RecorderSetting::default());
        assert_eq!(
            stored_recorder(&state).await.as_deref(),
            Some(r#"{"enabledFor":null}"#)
        );
        assert!(!state.listen_status().recorder.enabled);
    }

    #[tokio::test]
    async fn a_corrupt_recorder_row_warns_once_and_the_mics_still_load() {
        let capture = Capture::install();
        let state = AppState::in_memory().await.unwrap();
        state
            .repository()
            .set_app_setting(LISTEN_RECORDER_KEY, "not a row")
            .await
            .unwrap();
        state
            .repository()
            .set_app_setting(LISTEN_MICS_KEY, GUEST_MIC)
            .await
            .unwrap();
        assert!(state.start_listen("127.0.0.1:0".parse().unwrap()).await);
        let warned = capture.events_with_message("listen.recorder cannot be read");
        assert_eq!(warned.len(), 1, "{warned:?}");
        assert_eq!(warned[0].target, RECORDER_TARGET);
        assert_eq!(warned[0].level, tracing::Level::WARN);
        let status = state.listen_status();
        let ids: Vec<&str> = status
            .mics
            .iter()
            .map(|mic| mic.config.id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["guest"],
            "a broken recorder row never blocks the mics"
        );
        assert!(!status.recorder.enabled);
    }

    /// A failed audit row never fails the save; its WARN carries the target of
    /// the setting it is about, so grepping `presenter::listen::recorder` finds it.
    #[tokio::test]
    async fn a_failed_audit_row_is_warned_under_the_target_of_its_own_setting() {
        let capture = Capture::install();
        let state = AppState::in_memory().await.unwrap();
        let conn = state.repository().connection();
        sea_orm::ConnectionTrait::execute(
            conn,
            sea_orm::Statement::from_string(
                sea_orm::ConnectionTrait::get_database_backend(conn),
                "DROP TABLE settings_audit".to_string(),
            ),
        )
        .await
        .unwrap();
        state.set_listen_recorder(true, "test").await.unwrap();
        let mics = serde_json::from_str(GUEST_MIC).unwrap();
        state.set_listen_mics(mics, "test").await.unwrap();
        let audit_warnings = |target: &str| {
            capture
                .events(target)
                .iter()
                .filter(|event| {
                    event.level == tracing::Level::WARN
                        && event
                            .field("message")
                            .is_some_and(|message| message.contains("audit row"))
                })
                .count()
        };
        assert_eq!(audit_warnings(RECORDER_TARGET), 1, "the recorder's own");
        assert_eq!(audit_warnings(MIC_TARGET), 1, "the microphones' own");
    }

    /// A settable local clock, and the handle that moves it.
    fn settable(at: &str) -> (RecorderEnv, Arc<std::sync::Mutex<NaiveDateTime>>) {
        let now = Arc::new(std::sync::Mutex::new(at.parse::<NaiveDateTime>().unwrap()));
        let read = Arc::clone(&now);
        let env = RecorderEnv {
            root: std::env::temp_dir().join("presenter-recorder-never-written"),
            clock: Arc::new(move || *read.lock().unwrap()),
            first_pass_delay: Duration::ZERO,
        };
        (env, now)
    }

    /// NTP steps the clock back WHILE the server runs: the stored day, switched
    /// on while the clock was ahead, is now after today. The supervisor switches
    /// the recorder off, the row is cleared and audited as the system's write.
    #[tokio::test]
    async fn a_day_that_falls_after_today_while_running_is_cleared_from_the_row() {
        let capture = Capture::install();
        let state = AppState::in_memory().await.unwrap();
        let (env, now) = settable("2026-10-21T08:00:00");
        state.listen.set_recorder_env(env);
        assert!(state.start_listen("127.0.0.1:0".parse().unwrap()).await);
        state.set_listen_recorder(true, "test").await.unwrap(); // the clock is 3 days ahead
        assert_eq!(
            stored_recorder(&state).await.as_deref(),
            Some(r#"{"enabledFor":"2026-10-21"}"#)
        );
        *now.lock().unwrap() = "2026-10-18T19:42:05".parse().unwrap(); // NTP corrects it
        state.listen.apply(Vec::new()); // wakes the supervisor, as its minute tick would
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while stored_recorder(&state).await.as_deref() != Some(r#"{"enabledFor":null}"#) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the future day stayed stored"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let status = state.listen_status().recorder;
        assert_eq!((status.enabled, status.enabled_for), (false, None));
        let audit = state
            .repository()
            .list_settings_audit(Some("app_settings"), Some(LISTEN_RECORDER_KEY), None, 10)
            .await
            .unwrap();
        let cleared: Vec<_> = audit.iter().filter(|row| row.actor == "system").collect();
        assert_eq!(cleared.len(), 1, "one system audit row: {audit:?}");
        assert_eq!(cleared[0].source, SettingsAuditSource::StartupDefault);
        assert_eq!(
            cleared[0].before_json,
            Some(json!({"enabledFor": "2026-10-21"}))
        );
        assert_eq!(cleared[0].after_json, json!({"enabledFor": null}));
        let warned = capture.events_with_message("names a day in the future");
        assert_eq!(warned.len(), 1, "{warned:?}");
        assert_eq!(warned[0].target, RECORDER_TARGET);
        assert_eq!(warned[0].level, tracing::Level::WARN);
    }

    /// The supervisor's notice can arrive after the operator switched the
    /// recorder on again: the row is cleared only while it still names the day.
    #[tokio::test]
    async fn a_switch_made_since_is_never_cleared_with_the_old_future_day() {
        let state = AppState::in_memory().await.unwrap();
        state.listen.set_recorder_env(fixed("2026-10-18T19:42:05"));
        state.set_listen_recorder(true, "test").await.unwrap();
        state
            .clear_future_recorder_row("2026-10-21".parse().unwrap())
            .await;
        assert_eq!(
            stored_recorder(&state).await.as_deref(),
            Some(r#"{"enabledFor":"2026-10-18"}"#),
            "today's switch stays"
        );
        let audit = state
            .repository()
            .list_settings_audit(Some("app_settings"), Some(LISTEN_RECORDER_KEY), None, 10)
            .await
            .unwrap();
        assert_eq!(audit.len(), 1, "only the operator's own write: {audit:?}");
        assert!(state.listen_status().recorder.enabled);
    }
}
