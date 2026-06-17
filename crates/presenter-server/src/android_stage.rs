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
use tracing::{debug, error, warn};

const ADB_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

const COMMAND_CHANNEL_CAPACITY: usize = 8;
const RETRY_INTERVAL: Duration = Duration::from_secs(20);

/// Server-wide env var carrying the stage URL the launcher opens on every
/// Android stage display (e.g. `http://10.77.9.205/stage`). Set per environment
/// in the deploy systemd units. When unset/empty the launcher skips launching.
const STAGE_URL_ENV: &str = "PRESENTER_ANDROID_STAGE_URL";

/// Validate that a configured stage URL is a well-formed `http(s)://` URL before
/// it is spliced into the `am start -a VIEW -d <url>` adb args (which reach the
/// device's `/system/bin/sh`). Returns the normalized URL string on success, or
/// `None` when the value is empty or malformed. Defense-in-depth: a malformed
/// value is treated the same as unset (warn + skip) rather than passed through.
fn validate_stage_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Reject any whitespace or shell metacharacters anywhere in the value.
    // `Url::parse` is lenient and folds such characters into the path (e.g.
    // `http://host/stage; rm -rf /` parses fine), so a scheme/host check alone
    // is not enough to keep the value safe to splice into the on-device
    // `am start -a VIEW -d <url>` command. A legitimate stage URL never contains
    // these characters.
    const FORBIDDEN: &[char] = &[
        ' ', '\t', '\n', '\r', ';', '&', '|', '`', '$', '(', ')', '<', '>', '"', '\'', '\\',
    ];
    if trimmed
        .chars()
        .any(|c| c.is_control() || FORBIDDEN.contains(&c))
    {
        return None;
    }
    match reqwest::Url::parse(trimmed) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.has_host() => {
            Some(trimmed.to_string())
        }
        _ => None,
    }
}

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
    /// The stage URL opened on every display, read once from
    /// `PRESENTER_ANDROID_STAGE_URL` at construction. `None` when unset/empty —
    /// the launcher then warns and skips rather than firing a broken intent.
    stage_url: Arc<Option<String>>,
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
        let raw_stage_url = env::var(STAGE_URL_ENV).ok();
        let stage_url = raw_stage_url.as_deref().and_then(validate_stage_url);
        match (&stage_url, raw_stage_url.as_deref().map(str::trim)) {
            (Some(url), _) => {
                debug!(env = STAGE_URL_ENV, %url, "android stage launcher URL configured");
            }
            (None, Some(raw)) if !raw.is_empty() => {
                warn!(
                    env = STAGE_URL_ENV,
                    raw,
                    "android stage launcher URL is malformed (not a well-formed http(s):// URL) — \
                     treating as unset; displays will not be launched until it is corrected"
                );
            }
            _ => {
                warn!(
                    env = STAGE_URL_ENV,
                    "android stage launcher URL is unset — displays will not be launched until it is set"
                );
            }
        }
        Self {
            adb_path,
            stage_url: Arc::new(stage_url),
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
        let stage_url = Arc::clone(&self.stage_url);
        let status_clone = Arc::clone(&status);
        let config_clone = display.clone();
        let handle = tokio::spawn(async move {
            if let Err(err) =
                run_device_worker(adb_path, stage_url, config_clone, status_clone, command_rx).await
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
    stage_url: Arc<Option<String>>,
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
                    if let Err(err) = connect_and_launch(&adb_path, &stage_url, &config, &status).await {
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
                            if let Err(err) = connect_and_launch(&adb_path, &stage_url, &config, &status).await {
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

/// Disconnect any stale entry then `adb connect <serial>`, returning an error
/// (without recording status) on timeout, exec failure, or a connect error.
///
/// The disconnect clears stale offline entries ADB leaves after a TV power
/// cycle, which otherwise make subsequent `-s serial` commands fail until the
/// daemon restarts. Its result is intentionally ignored — the typical case is
/// "not connected", a non-zero exit we don't care about.
async fn adb_connect(adb_bin: &std::ffi::OsStr, serial: &str) -> anyhow::Result<()> {
    let _ = timeout(
        ADB_COMMAND_TIMEOUT,
        Command::new(adb_bin).arg("disconnect").arg(serial).output(),
    )
    .await;

    let connect_result = timeout(
        ADB_COMMAND_TIMEOUT,
        Command::new(adb_bin).arg("connect").arg(serial).output(),
    )
    .await;

    let connect_output = match connect_result {
        Ok(Ok(output)) => output,
        Ok(Err(io_err)) => {
            return Err(anyhow!("failed to execute adb for {}: {}", serial, io_err));
        }
        Err(_elapsed) => {
            return Err(anyhow!("adb connect timed out for {}", serial));
        }
    };

    if let Err(msg) = ensure_success(&connect_output) {
        return Err(anyhow!("adb connect error for {}: {}", serial, msg));
    }
    Ok(())
}

async fn connect_and_launch(
    adb_path: &Arc<OsString>,
    stage_url: &Arc<Option<String>>,
    config: &AndroidStageDisplay,
    status: &Arc<RwLock<AndroidStageDisplayStatusSnapshot>>,
) -> anyhow::Result<()> {
    let serial = format!("{}:{}", config.host, config.port);

    // Build the launch args BEFORE touching the device. If no stage URL is
    // configured, warn and skip — firing `am start` with no data URI would
    // open a broken page, so we deliberately do nothing and mark an error so
    // the operator dashboard surfaces the misconfiguration.
    let Some(launch_args) = build_launch_args(&config.launch_component, stage_url.as_deref())
    else {
        warn!(
            display = %config.label,
            env = STAGE_URL_ENV,
            "skipping android stage launch — stage URL not configured"
        );
        let msg = format!("{STAGE_URL_ENV} not configured — launch skipped");
        record_error(status, msg.clone()).await;
        return Err(anyhow!(msg));
    };

    let attempt_started = Utc::now();
    {
        let mut guard = status.write().await;
        guard.state = AndroidStageDisplayState::Connecting;
        guard.last_attempt = Some(attempt_started);
        guard.last_error = None;
    }

    let adb_bin = adb_path.as_os_str();

    if let Err(err) = adb_connect(adb_bin, &serial).await {
        record_error(status, err.to_string()).await;
        return Err(err);
    }

    {
        let mut guard = status.write().await;
        guard.state = AndroidStageDisplayState::Launching;
    }

    if let Err(err) = adb_launch(adb_bin, &serial, &launch_args).await {
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

/// Run `adb -s <serial> shell <launch_args>` (the `am start` VIEW intent),
/// returning an error (without recording status) on timeout, exec failure, or
/// a non-success `am start` result.
async fn adb_launch(
    adb_bin: &std::ffi::OsStr,
    serial: &str,
    launch_args: &[String],
) -> anyhow::Result<()> {
    let launch_result = timeout(
        ADB_COMMAND_TIMEOUT,
        Command::new(adb_bin)
            .arg("-s")
            .arg(serial)
            .arg("shell")
            .args(launch_args)
            .output(),
    )
    .await;

    let launch_output = match launch_result {
        Ok(Ok(output)) => output,
        Ok(Err(io_err)) => {
            return Err(anyhow!(
                "failed to execute adb shell for {}: {}",
                serial,
                io_err
            ));
        }
        Err(_elapsed) => {
            return Err(anyhow!("adb shell am start timed out for {}", serial));
        }
    };

    if let Err(msg) = ensure_success(&launch_output) {
        return Err(anyhow!("adb shell error for {}: {}", serial, msg));
    }
    Ok(())
}

/// Extract the Android PACKAGE from a stored `launch_component`. New rows store
/// a bare package (`com.tcl.browser`); legacy rows may store
/// `package/activity` — in that case we take the substring before the first
/// `/`. Surrounding whitespace is trimmed.
fn launch_package(launch_component: &str) -> &str {
    let trimmed = launch_component.trim();
    match trimmed.split_once('/') {
        Some((package, _activity)) => package,
        None => trimmed,
    }
}

/// Build the `am start` argument vector for an `adb shell` launch.
///
/// Returns `None` when no stage URL is configured (unset/empty) — the caller
/// must then skip launching rather than fire a broken intent. Otherwise emits a
/// `VIEW` intent carrying the stage URL targeted at the browser package:
/// `am start -a android.intent.action.VIEW -d <url> <package>`.
fn build_launch_args(launch_component: &str, stage_url: Option<&str>) -> Option<Vec<String>> {
    let url = stage_url.map(str::trim).filter(|u| !u.is_empty())?;
    let package = launch_package(launch_component);
    Some(vec![
        "am".to_string(),
        "start".to_string(),
        "-a".to_string(),
        "android.intent.action.VIEW".to_string(),
        "-d".to_string(),
        url.to_string(),
        package.to_string(),
    ])
}

/// Decide whether the periodic keep-alive should (re)launch the stage browser.
/// Launch ONLY when the configured browser package is NOT the device's current
/// foreground/resumed app. `None` (foreground could not be determined — the adb
/// probe failed) defaults to launching, so a genuinely-down display still
/// recovers and an inconclusive probe never SUPPRESSES a needed launch.
///
/// Re-firing `am start` (a VIEW intent) at an already-foreground com.tcl.browser
/// reloads the page (black blink + spinner) every cycle — the #419 regression.
/// Gating the keep-alive on this check stops the periodic reload while keeping
/// crash/sleep/exit recovery. Explicit/forced launches bypass it entirely.
fn should_launch_stage(foreground_package: Option<&str>, launch_package: &str) -> bool {
    // RED stub (#419): current behavior re-launches unconditionally.
    let _ = (foreground_package, launch_package);
    true
}

/// Parse the resumed-activity PACKAGE from `dumpsys activity activities` output.
/// Finds the `[m]ResumedActivity: ActivityRecord{<hash> u0 <pkg>/<activity> …}`
/// line and returns `<pkg>`. Returns None when no resumed activity is reported
/// (`mResumedActivity: null`) or the line is absent — the caller treats None as
/// "foreground unknown → (re)launch".
fn parse_foreground_package(dumpsys_output: &str) -> Option<String> {
    // RED stub (#419): no foreground awareness yet.
    let _ = dumpsys_output;
    None
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
        assert_eq!(
            args.last().map(String::as_str),
            Some("com.fullykiosk.videokiosk")
        );
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

    #[test]
    fn validate_stage_url_accepts_well_formed_http_urls() {
        // Well-formed http/https URLs pass through (trimmed).
        assert_eq!(
            validate_stage_url("http://10.77.9.205/stage"),
            Some("http://10.77.9.205/stage".to_string()),
        );
        assert_eq!(
            validate_stage_url("https://presenter.lan/stage"),
            Some("https://presenter.lan/stage".to_string()),
        );
        assert_eq!(
            validate_stage_url("  http://10.77.8.134:8080/stage  "),
            Some("http://10.77.8.134:8080/stage".to_string()),
            "surrounding whitespace is trimmed",
        );
    }

    #[test]
    fn validate_stage_url_rejects_malformed_values_as_skip() {
        // A set-but-malformed value is treated as unset (None -> skip launching),
        // so it can never be spliced into the adb VIEW-intent args.
        assert_eq!(validate_stage_url(""), None, "empty -> skip");
        assert_eq!(validate_stage_url("   "), None, "whitespace-only -> skip");
        assert_eq!(
            validate_stage_url("not a url"),
            None,
            "non-URL garbage -> skip",
        );
        assert_eq!(
            validate_stage_url("10.77.9.205/stage"),
            None,
            "missing scheme -> skip",
        );
        assert_eq!(
            validate_stage_url("ftp://10.77.9.205/stage"),
            None,
            "non-http(s) scheme -> skip",
        );
        assert_eq!(
            validate_stage_url("http://"),
            None,
            "scheme without host -> skip",
        );
        assert_eq!(
            validate_stage_url("javascript:alert(1)"),
            None,
            "javascript scheme -> skip",
        );
        // A shell-injection attempt is not a well-formed http URL -> rejected.
        assert_eq!(
            validate_stage_url("http://10.0.0.1/stage; rm -rf /"),
            None,
            "embedded shell metacharacters make it malformed -> skip",
        );
    }

    // ── #419: foreground-aware keep-alive ──────────────────────────────────
    // The 20s keep-alive must NOT re-fire `am start` when com.tcl.browser is
    // already the resumed app (re-firing the VIEW intent reloads the page —
    // the black blink + spinner every ~20s). It SHOULD launch when the browser
    // is not foreground (crash/sleep/exit recovery) or when foreground is
    // unknown (an inconclusive adb probe must never suppress a needed launch).

    #[test]
    fn skip_launch_when_browser_already_foreground() {
        assert!(
            !should_launch_stage(Some("com.tcl.browser"), "com.tcl.browser"),
            "must NOT relaunch when the browser is already the resumed app (#419)",
        );
    }

    #[test]
    fn launch_when_a_different_app_is_foreground() {
        assert!(
            should_launch_stage(Some("com.android.tv.settings"), "com.tcl.browser"),
            "must relaunch when another app is foreground (user left the stage)",
        );
    }

    #[test]
    fn launch_when_foreground_is_unknown() {
        assert!(
            should_launch_stage(None, "com.tcl.browser"),
            "an inconclusive probe must default to launching (recover a down display)",
        );
    }

    #[test]
    fn parse_foreground_reads_resumed_activity_package() {
        let out = "  ResumedActivity: ActivityRecord{deadbeef u0 com.tcl.browser/.portal.browse.activity.BrowsePageActivity t1}\n    mResumedActivity: ActivityRecord{5372874 u0 com.tcl.browser/.portal.browse.activity.BrowsePageActivity t17469}\n";
        assert_eq!(
            parse_foreground_package(out).as_deref(),
            Some("com.tcl.browser"),
        );
    }

    #[test]
    fn parse_foreground_reads_legacy_fully_kiosk_package() {
        let out = "    mResumedActivity: ActivityRecord{abc u0 com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity t9}";
        assert_eq!(
            parse_foreground_package(out).as_deref(),
            Some("com.fullykiosk.videokiosk"),
        );
    }

    #[test]
    fn parse_foreground_none_when_no_resumed_activity() {
        assert_eq!(parse_foreground_package("    mResumedActivity: null"), None);
        assert_eq!(parse_foreground_package(""), None);
        assert_eq!(
            parse_foreground_package("some unrelated dumpsys text"),
            None
        );
    }
}
