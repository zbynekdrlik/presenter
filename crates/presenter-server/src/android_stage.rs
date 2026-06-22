use anyhow::anyhow;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use presenter_core::{AndroidStageDisplay, AndroidStageDisplayId};
use serde::Serialize;
use std::{
    collections::HashMap,
    env,
    ffi::{OsStr, OsString},
    process::Output,
    sync::Arc,
    time::Duration,
};
use tokio::{
    process::Command,
    sync::{mpsc, RwLock},
    task::JoinHandle,
    time::{interval, timeout, MissedTickBehavior},
};
use tracing::{debug, error, info, warn};

const ADB_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// Injection seam for adb invocation (#421). All device I/O goes through this
/// trait so the keep-alive wiring (`run_device_worker` → `connect_and_launch`
/// → the adb helpers) is testable without a real `adb` binary or device: the
/// production impl (`ProcessAdbRunner`) spawns `adb`, while tests inject a fake
/// that records the invocations and returns canned `Output`.
///
/// `args` is the full adb argument vector (e.g. `["-s", serial, "shell", …]`).
/// The implementation is responsible for applying `ADB_COMMAND_TIMEOUT`.
#[async_trait]
pub trait AdbRunner: Send + Sync {
    async fn run(&self, args: &[OsString]) -> std::io::Result<Output>;
}

/// Production [`AdbRunner`]: spawns the configured `adb` binary with a timeout.
/// A timeout maps to an `io::Error` of kind `TimedOut` so callers handle it
/// identically to a spawn failure.
struct ProcessAdbRunner {
    adb_bin: Arc<OsString>,
}

#[async_trait]
impl AdbRunner for ProcessAdbRunner {
    async fn run(&self, args: &[OsString]) -> std::io::Result<Output> {
        match timeout(
            ADB_COMMAND_TIMEOUT,
            Command::new(self.adb_bin.as_os_str()).args(args).output(),
        )
        .await
        {
            Ok(result) => result,
            Err(_elapsed) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "adb command timed out",
            )),
        }
    }
}

/// Convenience for building an adb argument vector from string-ish parts.
fn adb_args<I, S>(parts: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    parts
        .into_iter()
        .map(|p| p.as_ref().to_os_string())
        .collect()
}

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
    /// adb invocation seam (#421). Production wraps `Command::new(adb)`; tests
    /// inject a fake. Shared by every spawned device worker.
    runner: Arc<dyn AdbRunner>,
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
        let adb_bin = env::var_os("PRESENTER_ANDROID_ADB_BIN")
            .map(Arc::from)
            .unwrap_or_else(|| Arc::new(OsString::from("adb")));
        let runner: Arc<dyn AdbRunner> = Arc::new(ProcessAdbRunner { adb_bin });
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
            runner,
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
        let runner = Arc::clone(&self.runner);
        let stage_url = Arc::clone(&self.stage_url);
        let status_clone = Arc::clone(&status);
        let config_clone = display.clone();
        let handle = tokio::spawn(async move {
            if let Err(err) =
                run_device_worker(runner, stage_url, config_clone, status_clone, command_rx).await
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
    runner: Arc<dyn AdbRunner>,
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
                    // Periodic keep-alive: foreground-aware (#419) — only
                    // relaunches when the browser is not already up.
                    if let Err(err) = connect_and_launch(runner.as_ref(), &stage_url, &config, &status, false).await {
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
                            // Explicit/forced launch (startup, config change,
                            // launch-now endpoint): always (re)launch (#419).
                            if let Err(err) = connect_and_launch(runner.as_ref(), &stage_url, &config, &status, true).await {
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
async fn adb_connect(runner: &dyn AdbRunner, serial: &str) -> anyhow::Result<()> {
    let _ = runner.run(&adb_args(["disconnect", serial])).await;

    let connect_output = match runner.run(&adb_args(["connect", serial])).await {
        Ok(output) => output,
        Err(io_err) => {
            return Err(anyhow!("failed to execute adb for {}: {}", serial, io_err));
        }
    };

    if let Err(msg) = ensure_success(&connect_output) {
        return Err(anyhow!("adb connect error for {}: {}", serial, msg));
    }
    Ok(())
}

async fn connect_and_launch(
    runner: &dyn AdbRunner,
    stage_url: &Arc<Option<String>>,
    config: &AndroidStageDisplay,
    status: &Arc<RwLock<AndroidStageDisplayStatusSnapshot>>,
    force_launch: bool,
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

    if let Err(err) = adb_connect(runner, &serial).await {
        record_error(status, err.to_string()).await;
        return Err(err);
    }

    // #419: foreground-aware keep-alive. On the periodic tick (force_launch =
    // false) skip the am-start when the browser is ALREADY the resumed app —
    // re-firing the VIEW intent reloads com.tcl.browser (black blink + spinner)
    // every cycle. An inconclusive probe (None) falls through to launching so a
    // genuinely-down display still recovers. Explicit/forced launches (config
    // change, launch-now endpoint) bypass the gate and always relaunch.
    if !force_launch {
        let launch_pkg = launch_package(&config.launch_component);
        let foreground = adb_foreground_component(runner, &serial).await;
        if !should_launch_stage(foreground.as_deref(), launch_pkg) {
            debug!(
                display = %config.label,
                package = launch_pkg,
                foreground = foreground.as_deref().unwrap_or("<unknown>"),
                "android stage keep-alive: stage page already foreground — skipping relaunch (#419)"
            );
            // A confirmed-foreground probe IS a liveness success: refresh
            // last_success so a healthy display that skips relaunch every cycle
            // does not show an ever-aging "last success" on the operator
            // dashboard (#419 review).
            let mut guard = status.write().await;
            guard.state = AndroidStageDisplayState::Running;
            guard.last_success = Some(Utc::now());
            guard.last_error = None;
            return Ok(());
        }
        info!(
            display = %config.label,
            foreground = foreground.as_deref().unwrap_or("<unknown>"),
            "android stage keep-alive: browser not foreground — relaunching (#419)"
        );
    }

    {
        let mut guard = status.write().await;
        guard.state = AndroidStageDisplayState::Launching;
    }

    if let Err(err) = adb_launch(runner, &serial, &launch_args).await {
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
    runner: &dyn AdbRunner,
    serial: &str,
    launch_args: &[String],
) -> anyhow::Result<()> {
    let mut args = adb_args(["-s", serial, "shell"]);
    args.extend(launch_args.iter().map(OsString::from));

    let launch_output = match runner.run(&args).await {
        Ok(output) => output,
        Err(io_err) => {
            return Err(anyhow!(
                "failed to execute adb shell for {}: {}",
                serial,
                io_err
            ));
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

/// The browser's resumed component (`<pkg>/<activity>`) suffix that means the
/// STAGE PAGE is genuinely showing — `com.tcl.browser`'s content/browse
/// activity. When the foreground component ends with this, the stage is loaded
/// and the keep-alive must NOT relaunch (re-firing the VIEW intent would reload
/// the page — the #419 black blink). Any OTHER activity of the same package
/// (notably the home portal `.portal.home.activity.StartActivity` the TV opens
/// to at power-on — #447) is NOT the stage and MUST be (re)launched.
const STAGE_CONTENT_ACTIVITY_SUFFIX: &str = "BrowsePageActivity";

/// Decide whether the periodic keep-alive should (re)launch the stage browser,
/// given the device's current foreground/resumed COMPONENT (`<pkg>/<activity>`).
///
/// Skip the relaunch ONLY when the stage page is genuinely showing — i.e. the
/// foreground component's package is the configured browser package AND its
/// activity is the content/browse activity (`…BrowsePageActivity`). Otherwise
/// launch:
///   - the browser is on its HOME PORTAL (`…StartActivity`) — the same package
///     but the stage is NOT loaded (the #447 power-on-to-home-portal case);
///   - another app is foreground (the user left the stage);
///   - `None` (the adb probe failed / no resumed activity) — never suppress a
///     possibly-needed launch.
///
/// Comparing only the package (the pre-#447 logic) wrongly skipped when the
/// browser sat on its home portal, because home-portal and stage-page are the
/// SAME package. Gating on the ACTIVITY fixes that while preserving #419's
/// no-periodic-reload guarantee for a genuinely-loaded stage. Explicit/forced
/// launches bypass this gate entirely.
fn should_launch_stage(foreground_component: Option<&str>, launch_package: &str) -> bool {
    match foreground_component {
        Some(component) => {
            let (package, activity) = match component.split_once('/') {
                Some(parts) => parts,
                // A package-only component (no activity) cannot be confirmed as
                // the stage page → (re)launch rather than leave a blank browser.
                None => return true,
            };
            // Skip ONLY when the configured browser is foreground AND it is on
            // its content/browse activity — i.e. the stage page is genuinely
            // showing. The home portal (`…StartActivity`) is the same package
            // but a different activity, so it correctly relaunches (#447).
            let stage_is_showing =
                package == launch_package && activity.ends_with(STAGE_CONTENT_ACTIVITY_SUFFIX);
            !stage_is_showing
        }
        // Foreground could not be determined → don't suppress a possibly-needed
        // launch; (re)launch and let the device sort it out.
        None => true,
    }
}

/// Parse the resumed-activity COMPONENT (`<pkg>/<activity>`) from
/// `dumpsys activity activities` output. Finds the
/// `[m]ResumedActivity: ActivityRecord{<hash> u0 <pkg>/<activity> …}` line and
/// returns `<pkg>/<activity>`. Returns None when no resumed activity is reported
/// (`mResumedActivity: null`) or the line is absent — the caller treats None as
/// "foreground unknown → (re)launch".
///
/// The component (package AND activity) is required by [`should_launch_stage`]
/// to tell the loaded stage page (`…BrowsePageActivity`) from the home portal
/// (`…StartActivity`), which share the `com.tcl.browser` package (#447).
fn parse_foreground_component(dumpsys_output: &str) -> Option<String> {
    // Match either `mResumedActivity:` or `ResumedActivity:` (label varies by
    // Android version); both carry the same `<pkg>/<activity>` component token.
    let line = dumpsys_output
        .lines()
        .find(|l| l.contains("ResumedActivity"))?;
    // The component is the first whitespace token shaped `<pkg>/<activity>`;
    // the package part always contains a dot and never a `{` (which excludes
    // the `ActivityRecord{<hash>` token).
    line.split_whitespace().find_map(|tok| {
        let (pkg, _activity) = tok.split_once('/')?;
        (pkg.contains('.') && !pkg.contains('{')).then(|| tok.to_string())
    })
}

/// Query the device's currently-resumed COMPONENT (`<pkg>/<activity>`) via
/// `adb -s <serial> shell dumpsys activity activities`. Returns the resumed
/// component, or None on any adb error/timeout/non-success or when no resumed
/// activity is reported — the caller treats None as "foreground unknown →
/// (re)launch". Read-only: the dumpsys probe never disturbs the running browser.
async fn adb_foreground_component(runner: &dyn AdbRunner, serial: &str) -> Option<String> {
    let output = runner
        .run(&adb_args([
            "-s",
            serial,
            "shell",
            "dumpsys",
            "activity",
            "activities",
        ]))
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_foreground_component(&String::from_utf8_lossy(&output.stdout))
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

    // Component shapes the TCL browser reports for the stage page vs the home
    // portal — the same package, different activity (the #447 distinction).
    const STAGE_PAGE_COMPONENT: &str = "com.tcl.browser/.portal.browse.activity.BrowsePageActivity";
    const HOME_PORTAL_COMPONENT: &str = "com.tcl.browser/.portal.home.activity.StartActivity";

    #[test]
    fn skip_launch_when_stage_page_already_foreground() {
        // #419: the stage page (browse activity) is genuinely showing → no reload.
        assert!(
            !should_launch_stage(Some(STAGE_PAGE_COMPONENT), "com.tcl.browser"),
            "must NOT relaunch when the stage page (BrowsePageActivity) is foreground (#419)",
        );
    }

    #[test]
    fn launch_when_browser_on_home_portal() {
        // #447: the TV powered on to the browser's home portal (StartActivity).
        // Same package, but the stage is NOT loaded → MUST relaunch.
        assert!(
            should_launch_stage(Some(HOME_PORTAL_COMPONENT), "com.tcl.browser"),
            "must relaunch when the browser sits on its home portal — the stage is not loaded (#447)",
        );
    }

    #[test]
    fn launch_when_a_different_app_is_foreground() {
        assert!(
            should_launch_stage(
                Some("com.android.tv.settings/.MainActivity"),
                "com.tcl.browser"
            ),
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
    fn launch_when_browser_foreground_but_activity_unknown() {
        // A package-only component (no `/activity`) cannot be confirmed as the
        // loaded stage page → relaunch rather than risk leaving a blank browser.
        assert!(
            should_launch_stage(Some("com.tcl.browser"), "com.tcl.browser"),
            "a bare package (activity unknown) must not be treated as the loaded stage (#447)",
        );
    }

    #[test]
    fn parse_foreground_reads_resumed_activity_component() {
        let out = "  ResumedActivity: ActivityRecord{deadbeef u0 com.tcl.browser/.portal.browse.activity.BrowsePageActivity t1}\n    mResumedActivity: ActivityRecord{5372874 u0 com.tcl.browser/.portal.browse.activity.BrowsePageActivity t17469}\n";
        assert_eq!(
            parse_foreground_component(out).as_deref(),
            Some("com.tcl.browser/.portal.browse.activity.BrowsePageActivity"),
        );
    }

    #[test]
    fn parse_foreground_reads_home_portal_component() {
        // #447: at power-on the TV resumes the browser's HOME PORTAL activity.
        let out = "    mResumedActivity: ActivityRecord{5372874 u0 com.tcl.browser/.portal.home.activity.StartActivity t9}";
        assert_eq!(
            parse_foreground_component(out).as_deref(),
            Some("com.tcl.browser/.portal.home.activity.StartActivity"),
        );
    }

    #[test]
    fn parse_foreground_reads_legacy_fully_kiosk_component() {
        let out = "    mResumedActivity: ActivityRecord{abc u0 com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity t9}";
        assert_eq!(
            parse_foreground_component(out).as_deref(),
            Some("com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity"),
        );
    }

    #[test]
    fn parse_foreground_none_when_no_resumed_activity() {
        assert_eq!(
            parse_foreground_component("    mResumedActivity: null"),
            None
        );
        assert_eq!(parse_foreground_component(""), None);
        assert_eq!(
            parse_foreground_component("some unrelated dumpsys text"),
            None
        );
    }

    #[test]
    fn parse_foreground_skips_activity_record_token_that_has_dot_and_slash() {
        // The component picker keeps the FIRST whitespace token shaped
        // `<pkg>/<activity>` whose pkg part contains a dot AND no `{` — the
        // `&&` guard at parse_foreground_component:618 rejects the
        // `ActivityRecord{<hash>` wrapper token (which carries the `{`). This
        // input puts an `ActivityRecord{…}` token BEFORE the real component AND
        // makes that wrapper token itself contain both a `.` and a `/` in its
        // pkg part. With the correct `&&` (pkg has a dot AND no `{`) the wrapper
        // is rejected and the real component is returned; the surviving mutant
        // (`&&` → `||`, "pkg has a dot OR no `{`") would WRONGLY accept the
        // wrapper token and return `ActivityRecord{ab.cd/x`. So this test passes
        // with `&&` and FAILS with `||` — killing the mutant.
        let out = "  mResumedActivity: ActivityRecord{ab.cd/x u0 com.tcl.browser/.portal.browse.activity.BrowsePageActivity t1}";
        assert_eq!(
            parse_foreground_component(out).as_deref(),
            Some("com.tcl.browser/.portal.browse.activity.BrowsePageActivity"),
            "must skip the ActivityRecord wrapper token (carries `{{`) and keep the real component",
        );
    }

    // ── #421: keep-alive wiring integration tests (fake AdbRunner) ──────────
    //
    // These exercise the WIRING — the force_launch dispatch + the foreground
    // gate consulted only when !force_launch — with a fake adb runner that
    // records every invocation and returns canned output, so no real `adb`
    // binary or device is needed. The pure helpers are unit-tested above; this
    // proves the helpers are wired together correctly:
    //   (a) a periodic tick (force_launch=false) fires NO `am start` when the
    //       browser is already foreground,
    //   (b) a tick fires `am start` when the device is backgrounded/unknown,
    //   (c) LaunchNow (force_launch=true) always fires `am start`, regardless
    //       of foreground state and WITHOUT probing it.

    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    const TEST_STAGE_URL: &str = "http://10.77.9.205/stage";
    const TEST_PKG: &str = "com.tcl.browser";

    fn ok_output(stdout: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    /// Whether `dumpsys activity activities` reports the stage browser with its
    /// stage page genuinely showing (`StagePage`), the browser sitting on its
    /// home portal (`HomePortal` — same package, stage NOT loaded, the #447
    /// power-on case), another app, or a failed (inconclusive) probe.
    #[derive(Clone, Copy)]
    enum Foreground {
        StagePage,
        HomePortal,
        OtherApp,
        ProbeFails,
    }

    fn err_output(stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(1 << 8), // non-zero exit (wait-status)
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    /// Fake [`AdbRunner`] recording every invocation as a single space-joined
    /// string, and answering `dumpsys` per the configured foreground state.
    struct FakeAdbRunner {
        foreground: Foreground,
        /// When true, `adb connect <serial>` returns a failed/non-success
        /// Output so `adb_connect` errors (modeling an unreachable device).
        connect_fails: bool,
        calls: Mutex<Vec<String>>,
    }

    impl FakeAdbRunner {
        fn new(foreground: Foreground) -> Self {
            Self {
                foreground,
                connect_fails: false,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn with_connect_failure(foreground: Foreground) -> Self {
            Self {
                foreground,
                connect_fails: true,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn invocations(&self) -> Vec<String> {
            self.calls.lock().expect("calls mutex poisoned").clone()
        }

        fn am_start_calls(&self) -> usize {
            self.invocations()
                .iter()
                .filter(|c| c.contains("am start"))
                .count()
        }

        fn connect_calls(&self) -> usize {
            self.invocations()
                .iter()
                .filter(|c| c.starts_with("connect "))
                .count()
        }

        fn dumpsys_calls(&self) -> usize {
            self.invocations()
                .iter()
                .filter(|c| c.contains("dumpsys activity activities"))
                .count()
        }
    }

    #[async_trait]
    impl AdbRunner for FakeAdbRunner {
        async fn run(&self, args: &[OsString]) -> std::io::Result<Output> {
            let joined = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            self.calls
                .lock()
                .expect("calls mutex poisoned")
                .push(joined.clone());

            if joined.contains("dumpsys activity activities") {
                return match self.foreground {
                    // Stage page genuinely showing → no relaunch (#419).
                    Foreground::StagePage => Ok(ok_output(&format!(
                        "    mResumedActivity: ActivityRecord{{abc u0 {TEST_PKG}/.portal.browse.activity.BrowsePageActivity t1}}"
                    ))),
                    // Same package, but the home portal — stage NOT loaded (#447)
                    // → MUST relaunch.
                    Foreground::HomePortal => Ok(ok_output(&format!(
                        "    mResumedActivity: ActivityRecord{{abc u0 {TEST_PKG}/.portal.home.activity.StartActivity t1}}"
                    ))),
                    Foreground::OtherApp => Ok(ok_output(
                        "    mResumedActivity: ActivityRecord{def u0 com.android.tv.settings/.Main t2}",
                    )),
                    // An inconclusive probe = adb exec failure (None foreground).
                    Foreground::ProbeFails => {
                        Err(std::io::Error::other("simulated dumpsys failure"))
                    }
                };
            }

            // `adb connect <serial>` fails when the device is unreachable.
            if self.connect_fails && joined.starts_with("connect ") {
                return Ok(err_output("error: failed to connect to 'host:port'"));
            }

            // connect / disconnect / `am start` all succeed cleanly.
            Ok(ok_output(""))
        }
    }

    fn test_display() -> AndroidStageDisplay {
        let now = Utc::now();
        AndroidStageDisplay::new(
            AndroidStageDisplayId::from_uuid(Uuid::new_v4()),
            "Stage TV".to_string(),
            "10.0.0.42".to_string(),
            5555,
            TEST_PKG.to_string(),
            true,
            now,
            now,
        )
    }

    fn test_status() -> Arc<RwLock<AndroidStageDisplayStatusSnapshot>> {
        Arc::new(RwLock::new(AndroidStageDisplayStatusSnapshot {
            state: AndroidStageDisplayState::Connecting,
            last_attempt: None,
            last_success: None,
            last_error: None,
        }))
    }

    // (a) Periodic keep-alive, stage page already foreground → NO `am start`.
    #[tokio::test]
    async fn tick_skips_am_start_when_stage_page_foreground() {
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();

        // force_launch=false models the periodic tick.
        let result = connect_and_launch(&runner, &stage_url, &config, &status, false).await;
        assert!(
            result.is_ok(),
            "keep-alive on a healthy display must succeed"
        );

        assert_eq!(
            runner.connect_calls(),
            1,
            "connect_and_launch MUST `adb connect` the device before probing/launching",
        );
        assert_eq!(
            runner.dumpsys_calls(),
            1,
            "the tick MUST consult the foreground gate (dumpsys) when !force_launch",
        );
        assert_eq!(
            runner.am_start_calls(),
            0,
            "the tick must NOT re-fire `am start` when the stage page is already foreground (#419 wiring)",
        );
        assert_eq!(
            status.read().await.state,
            AndroidStageDisplayState::Running,
            "a confirmed-stage-page probe is a liveness success → Running",
        );
    }

    // (a') #447: Periodic keep-alive, browser on its HOME PORTAL (same package,
    // stage NOT loaded) → MUST fire `am start`. This is the power-on-to-portal
    // bug: the pre-#447 package-only gate wrongly skipped here forever.
    #[tokio::test]
    async fn tick_fires_am_start_when_browser_on_home_portal() {
        let runner = FakeAdbRunner::new(Foreground::HomePortal);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();

        let result = connect_and_launch(&runner, &stage_url, &config, &status, false).await;
        assert!(
            result.is_ok(),
            "a TV stuck on the home portal must relaunch and succeed (#447)"
        );

        assert_eq!(
            runner.dumpsys_calls(),
            1,
            "the tick MUST consult the foreground gate when !force_launch",
        );
        assert_eq!(
            runner.am_start_calls(),
            1,
            "the tick MUST fire `am start` when the browser sits on its home portal — the stage is not loaded (#447)",
        );
    }

    // (b) Periodic keep-alive, another app foreground → DOES fire `am start`.
    #[tokio::test]
    async fn tick_fires_am_start_when_backgrounded() {
        let runner = FakeAdbRunner::new(Foreground::OtherApp);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();

        let result = connect_and_launch(&runner, &stage_url, &config, &status, false).await;
        assert!(
            result.is_ok(),
            "a backgrounded display must relaunch and succeed"
        );

        assert_eq!(
            runner.dumpsys_calls(),
            1,
            "the tick MUST consult the foreground gate when !force_launch",
        );
        assert_eq!(
            runner.am_start_calls(),
            1,
            "the tick MUST fire `am start` when the browser is not foreground (recovery wiring)",
        );
    }

    // (b') Periodic keep-alive, foreground probe inconclusive → still launches.
    #[tokio::test]
    async fn tick_fires_am_start_when_foreground_unknown() {
        let runner = FakeAdbRunner::new(Foreground::ProbeFails);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();

        let result = connect_and_launch(&runner, &stage_url, &config, &status, false).await;
        assert!(result.is_ok());

        assert_eq!(runner.dumpsys_calls(), 1, "the gate is consulted on a tick");
        assert_eq!(
            runner.am_start_calls(),
            1,
            "an inconclusive foreground probe must NOT suppress a needed launch",
        );
    }

    // (c) LaunchNow / forced launch → always `am start`, NEVER probes foreground.
    #[tokio::test]
    async fn launch_now_always_fires_am_start_without_probing() {
        // Even with the browser already foreground, force_launch=true must
        // relaunch and must NOT consult the gate.
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();

        let result = connect_and_launch(&runner, &stage_url, &config, &status, true).await;
        assert!(result.is_ok(), "a forced launch must succeed");

        assert_eq!(
            runner.dumpsys_calls(),
            0,
            "a forced launch (force_launch=true) MUST bypass the foreground gate",
        );
        assert_eq!(
            runner.am_start_calls(),
            1,
            "a forced launch MUST always fire `am start` regardless of foreground state",
        );
    }

    // A failing `adb connect` MUST abort the launch: connect_and_launch errors,
    // records the error, and fires NO `am start` (and never probes foreground).
    // This pins the adb_connect call into the launch path — a no-op connect that
    // always succeeded would let the launch proceed against an unreachable
    // device.
    #[tokio::test]
    async fn launch_aborts_when_adb_connect_fails() {
        let runner = FakeAdbRunner::with_connect_failure(Foreground::StagePage);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();

        let result = connect_and_launch(&runner, &stage_url, &config, &status, true).await;
        assert!(
            result.is_err(),
            "a failed `adb connect` MUST abort the launch with an error",
        );

        assert_eq!(
            runner.connect_calls(),
            1,
            "connect_and_launch MUST attempt `adb connect` before launching",
        );
        assert_eq!(
            runner.am_start_calls(),
            0,
            "no `am start` may fire once `adb connect` has failed",
        );
        assert_eq!(
            status.read().await.state,
            AndroidStageDisplayState::Error,
            "a connect failure must surface as Error on the operator dashboard",
        );
    }

    // The DISPATCH wiring end-to-end: a LaunchNow command through the worker
    // fires `am start` even when the browser is already foreground (proving the
    // worker dispatches force_launch=true for LaunchNow, never consulting the
    // gate). Drives run_device_worker directly so the command→force_launch
    // mapping is exercised, not just connect_and_launch.
    #[tokio::test]
    async fn worker_launch_now_command_forces_am_start() {
        // Keep a concrete Arc to read recorded calls after the worker exits;
        // hand a coerced `Arc<dyn AdbRunner>` clone to the worker (no downcast).
        let fake = Arc::new(FakeAdbRunner::new(Foreground::StagePage));
        let runner: Arc<dyn AdbRunner> = Arc::clone(&fake) as Arc<dyn AdbRunner>;
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();

        let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        // LaunchNow then Shutdown so the worker processes the command and exits
        // promptly (the 20s tick never fires within this test window).
        tx.try_send(DeviceCommand::LaunchNow).unwrap();
        tx.try_send(DeviceCommand::Shutdown).unwrap();

        run_device_worker(runner, stage_url, config, status, rx)
            .await
            .expect("worker should exit cleanly on Shutdown");

        assert_eq!(
            fake.dumpsys_calls(),
            0,
            "LaunchNow must dispatch force_launch=true → the gate is never consulted",
        );
        assert_eq!(
            fake.am_start_calls(),
            1,
            "LaunchNow must fire `am start` even when the browser is already foreground",
        );
    }
}
