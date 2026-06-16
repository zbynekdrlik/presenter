use anyhow::anyhow;
use chrono::{DateTime, Utc};
use presenter_core::{AndroidStageDisplay, AndroidStageDisplayId};
use serde::Serialize;
use std::{collections::HashMap, env, ffi::OsString, process::Output, sync::Arc, time::Duration};
use tokio::{
    process::Command,
    sync::{mpsc, RwLock},
    task::JoinHandle,
    time::{interval, timeout, MissedTickBehavior},
};
use tracing::{debug, error};

const ADB_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

const COMMAND_CHANNEL_CAPACITY: usize = 8;
const RETRY_INTERVAL: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AndroidStageDisplayState {
    Disabled,
    Connecting,
    Launching,
    Running,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidStageDisplayStatusSnapshot {
    pub state: AndroidStageDisplayState,
    pub last_attempt: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl AndroidStageDisplayStatusSnapshot {
    pub fn disabled() -> Self {
        Self {
            state: AndroidStageDisplayState::Disabled,
            last_attempt: None,
            last_success: None,
            last_error: None,
        }
    }
}

#[derive(Clone)]
pub struct AndroidStageRegistry {
    adb_path: Arc<OsString>,
    displays: Arc<RwLock<HashMap<AndroidStageDisplayId, DeviceEntry>>>,
}

struct DeviceEntry {
    config: AndroidStageDisplay,
    status: Arc<RwLock<AndroidStageDisplayStatusSnapshot>>,
    command_tx: mpsc::Sender<DeviceCommand>,
    handle: JoinHandle<()>,
}

#[derive(Debug)]
enum DeviceCommand {
    RefreshConfig(AndroidStageDisplay),
    LaunchNow,
    Shutdown,
}

impl AndroidStageRegistry {
    pub fn new() -> Self {
        let adb_path = env::var_os("PRESENTER_ANDROID_ADB_BIN")
            .map(Arc::from)
            .unwrap_or_else(|| Arc::new(OsString::from("adb")));
        Self {
            adb_path,
            displays: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn set_displays(&self, displays: Vec<AndroidStageDisplay>) {
        let mut guard = self.displays.write().await;
        let mut desired: HashMap<AndroidStageDisplayId, AndroidStageDisplay> = displays
            .into_iter()
            .map(|display| (display.id, display))
            .collect();

        let existing_ids: Vec<_> = guard.keys().copied().collect();
        for id in existing_ids {
            if !desired.contains_key(&id) {
                if let Some(entry) = guard.remove(&id) {
                    let _ = entry.command_tx.try_send(DeviceCommand::Shutdown);
                    entry.handle.abort();
                }
            }
        }

        for (id, display) in desired.drain() {
            match guard.get_mut(&id) {
                Some(entry) => {
                    entry.config = display.clone();
                    if let Err(err) = entry
                        .command_tx
                        .try_send(DeviceCommand::RefreshConfig(display.clone()))
                    {
                        debug!(%id, ?err, "android stage display command queue full during refresh");
                    }
                    if display.is_enabled {
                        let _ = entry.command_tx.try_send(DeviceCommand::LaunchNow);
                    }
                }
                None => {
                    let entry = self.spawn_display(display);
                    guard.insert(id, entry);
                }
            }
        }
    }

    pub async fn snapshot(
        &self,
    ) -> HashMap<AndroidStageDisplayId, AndroidStageDisplayStatusSnapshot> {
        let guard = self.displays.read().await;
        let mut result = HashMap::with_capacity(guard.len());
        for (id, entry) in guard.iter() {
            let snapshot = entry.status.read().await.clone();
            result.insert(*id, snapshot);
        }
        result
    }

    pub async fn snapshot_for(
        &self,
        id: AndroidStageDisplayId,
    ) -> AndroidStageDisplayStatusSnapshot {
        let guard = self.displays.read().await;
        if let Some(entry) = guard.get(&id) {
            entry.status.read().await.clone()
        } else {
            AndroidStageDisplayStatusSnapshot::disabled()
        }
    }

    /// Tell the worker for `id` to run a launch immediately, bypassing the
    /// 20-second tick. Returns an error if no such display exists or if the
    /// display is currently disabled. The launch runs asynchronously — the
    /// caller should poll `snapshot_for(id)` to observe the state change.
    pub async fn launch_now(&self, id: AndroidStageDisplayId) -> anyhow::Result<()> {
        let guard = self.displays.read().await;
        let entry = guard
            .get(&id)
            .ok_or_else(|| anyhow!("unknown android stage display {id}"))?;
        if !entry.config.is_enabled {
            return Err(anyhow!("android stage display {id} is disabled"));
        }
        entry
            .command_tx
            .try_send(DeviceCommand::LaunchNow)
            .map_err(|err| anyhow!("failed to enqueue launch for {id}: {err}"))?;
        Ok(())
    }

    fn spawn_display(&self, display: AndroidStageDisplay) -> DeviceEntry {
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let status = Arc::new(RwLock::new(if display.is_enabled {
            AndroidStageDisplayStatusSnapshot {
                state: AndroidStageDisplayState::Connecting,
                last_attempt: None,
                last_success: None,
                last_error: None,
            }
        } else {
            AndroidStageDisplayStatusSnapshot::disabled()
        }));
        let adb_path = Arc::clone(&self.adb_path);
        let status_clone = Arc::clone(&status);
        let config_clone = display.clone();
        let handle = tokio::spawn(async move {
            if let Err(err) =
                run_device_worker(adb_path, config_clone, status_clone, command_rx).await
            {
                error!(?err, "android stage display worker exited");
            }
        });
        if display.is_enabled {
            let _ = command_tx.try_send(DeviceCommand::LaunchNow);
        }
        DeviceEntry {
            config: display,
            status,
            command_tx,
            handle,
        }
    }
}

async fn run_device_worker(
    adb_path: Arc<OsString>,
    mut config: AndroidStageDisplay,
    status: Arc<RwLock<AndroidStageDisplayStatusSnapshot>>,
    mut command_rx: mpsc::Receiver<DeviceCommand>,
) -> anyhow::Result<()> {
    let mut ticker = interval(RETRY_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if config.is_enabled {
                    if let Err(err) = connect_and_launch(&adb_path, &config, &status).await {
                        debug!(display = %config.label, ?err, "android stage display launch attempt failed");
                    }
                } else {
                    mark_disabled(&status).await;
                }
            }
            Some(command) = command_rx.recv() => {
                match command {
                    DeviceCommand::RefreshConfig(new_config) => {
                        config = new_config;
                        if !config.is_enabled {
                            mark_disabled(&status).await;
                        }
                    }
                    DeviceCommand::LaunchNow => {
                        if config.is_enabled {
                            if let Err(err) = connect_and_launch(&adb_path, &config, &status).await {
                                debug!(display = %config.label, ?err, "android stage display manual launch failed");
                            }
                        }
                    }
                    DeviceCommand::Shutdown => {
                        mark_disabled(&status).await;
                        break;
                    }
                }
            }
            else => break,
        }
    }
    Ok(())
}

async fn connect_and_launch(
    adb_path: &Arc<OsString>,
    config: &AndroidStageDisplay,
    status: &Arc<RwLock<AndroidStageDisplayStatusSnapshot>>,
) -> anyhow::Result<()> {
    let serial = format!("{}:{}", config.host, config.port);
    let attempt_started = Utc::now();
    {
        let mut guard = status.write().await;
        guard.state = AndroidStageDisplayState::Connecting;
        guard.last_attempt = Some(attempt_started);
        guard.last_error = None;
    }

    let adb_bin = adb_path.as_os_str();

    // Clear any stale offline device entry from a previous attempt.
    // ADB leaves stale entries after TV power cycles which then cause
    // subsequent `-s serial` commands to fail until the daemon is restarted.
    // Errors are intentionally ignored — the typical case is "not connected"
    // which returns a non-zero exit code we don't care about.
    let _ = timeout(
        ADB_COMMAND_TIMEOUT,
        Command::new(adb_bin)
            .arg("disconnect")
            .arg(&serial)
            .output(),
    )
    .await;

    // Run adb connect
    let connect_result = timeout(
        ADB_COMMAND_TIMEOUT,
        Command::new(adb_bin).arg("connect").arg(&serial).output(),
    )
    .await;

    let connect_output = match connect_result {
        Ok(Ok(output)) => output,
        Ok(Err(io_err)) => {
            let err = anyhow!("failed to execute adb for {}: {}", serial, io_err);
            record_error(status, err.to_string()).await;
            return Err(err);
        }
        Err(_elapsed) => {
            let err = anyhow!("adb connect timed out for {}", serial);
            record_error(status, err.to_string()).await;
            return Err(err);
        }
    };

    if let Err(msg) = ensure_success(&connect_output) {
        let err = anyhow!("adb connect error for {}: {}", serial, msg);
        record_error(status, err.to_string()).await;
        return Err(err);
    }

    {
        let mut guard = status.write().await;
        guard.state = AndroidStageDisplayState::Launching;
    }

    // Run adb shell am start
    let launch_result = timeout(
        ADB_COMMAND_TIMEOUT,
        Command::new(adb_bin)
            .arg("-s")
            .arg(&serial)
            .arg("shell")
            .arg("am")
            .arg("start")
            .arg("-n")
            .arg(&config.launch_component)
            .output(),
    )
    .await;

    let launch_output = match launch_result {
        Ok(Ok(output)) => output,
        Ok(Err(io_err)) => {
            let err = anyhow!("failed to execute adb shell for {}: {}", serial, io_err);
            record_error(status, err.to_string()).await;
            return Err(err);
        }
        Err(_elapsed) => {
            let err = anyhow!("adb shell am start timed out for {}", serial);
            record_error(status, err.to_string()).await;
            return Err(err);
        }
    };

    if let Err(msg) = ensure_success(&launch_output) {
        let err = anyhow!("adb shell error for {}: {}", serial, msg);
        record_error(status, err.to_string()).await;
        return Err(err);
    }

    let success = Utc::now();
    let mut guard = status.write().await;
    guard.state = AndroidStageDisplayState::Running;
    guard.last_success = Some(success);
    guard.last_error = None;
    Ok(())
}

fn ensure_success(output: &Output) -> Result<(), String> {
    if !output.status.success() {
        return Err(format_command_failure(output));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    if stdout.contains("unable to connect")
        || stdout.contains("failed to connect")
        || stdout.contains("error:")
        || stderr.contains("unable to connect")
        || stderr.contains("failed to connect")
        || stderr.contains("error:")
    {
        return Err(format_command_failure(output));
    }
    Ok(())
}

fn format_command_failure(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!(
        "status: {} stdout: {} stderr: {}",
        output.status,
        stdout.trim(),
        stderr.trim()
    )
}

async fn mark_disabled(status: &Arc<RwLock<AndroidStageDisplayStatusSnapshot>>) {
    let mut guard = status.write().await;
    guard.state = AndroidStageDisplayState::Disabled;
    guard.last_error = None;
}

async fn record_error(status: &Arc<RwLock<AndroidStageDisplayStatusSnapshot>>, message: String) {
    let mut guard = status.write().await;
    guard.state = AndroidStageDisplayState::Error;
    guard.last_error = Some(truncate_error(&message));
}

fn truncate_error(message: &str) -> String {
    const MAX_LEN: usize = 280;
    if message.len() <= MAX_LEN {
        message.to_string()
    } else {
        format!("{}…", &message[..MAX_LEN - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::AndroidStageDisplayId;
    use uuid::Uuid;

    #[tokio::test]
    async fn launch_now_errors_on_unknown_id() {
        let registry = AndroidStageRegistry::new();
        let unknown = AndroidStageDisplayId::from_uuid(Uuid::new_v4());
        let err = registry.launch_now(unknown).await;
        assert!(err.is_err(), "launch_now must error on unknown id");
        assert!(
            err.unwrap_err()
                .to_string()
                .contains("unknown android stage display"),
            "error message should identify the unknown-id case",
        );
    }

    #[test]
    fn launch_package_strips_legacy_activity_suffix() {
        // Legacy stored value "package/activity" — the launcher must treat the
        // substring before "/" as the package for the VIEW intent.
        assert_eq!(
            launch_package("com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity"),
            "com.fullykiosk.videokiosk"
        );
        // A bare package (the new default shape) passes through unchanged.
        assert_eq!(launch_package("com.tcl.browser"), "com.tcl.browser");
        // Surrounding whitespace is trimmed.
        assert_eq!(launch_package("  com.tcl.browser  "), "com.tcl.browser");
    }

    #[test]
    fn build_launch_args_emits_view_intent_with_url_and_package() {
        // The proven-working prod launch is a VIEW intent with the stage URL
        // and the browser package — NOT the bare `am start -n <component>`.
        let args = build_launch_args("com.tcl.browser", Some("http://10.77.9.205/stage"))
            .expect("a configured URL must produce launch args");
        assert_eq!(
            args,
            vec![
                "am".to_string(),
                "start".to_string(),
                "-a".to_string(),
                "android.intent.action.VIEW".to_string(),
                "-d".to_string(),
                "http://10.77.9.205/stage".to_string(),
                "com.tcl.browser".to_string(),
            ],
            "launcher must fire a VIEW intent with the stage URL, not `am start -n`",
        );
    }

    #[test]
    fn build_launch_args_extracts_package_from_legacy_component() {
        // Backward compat: a legacy "package/activity" launch_component still
        // yields a VIEW intent targeting just the package.
        let args = build_launch_args(
            "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity",
            Some("http://10.77.8.134:8080/stage"),
        )
        .expect("a configured URL must produce launch args");
        assert_eq!(args.last().map(String::as_str), Some("com.fullykiosk.videokiosk"));
        assert!(
            args.iter().any(|a| a == "android.intent.action.VIEW"),
            "must use a VIEW intent",
        );
        assert!(
            args.iter().any(|a| a == "http://10.77.8.134:8080/stage"),
            "must pass the configured stage URL as the data URI",
        );
    }

    #[test]
    fn build_launch_args_skips_when_url_unset() {
        // No URL configured -> skip launching, do not fire a broken intent.
        assert_eq!(build_launch_args("com.tcl.browser", None), None);
        assert_eq!(build_launch_args("com.tcl.browser", Some("")), None);
        assert_eq!(build_launch_args("com.tcl.browser", Some("   ")), None);
    }
}
