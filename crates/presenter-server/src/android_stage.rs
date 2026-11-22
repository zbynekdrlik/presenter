use anyhow::anyhow;
use chrono::{DateTime, Utc};
use presenter_core::{AndroidStageDisplay, AndroidStageDisplayId, DEFAULT_LAUNCH_PACKAGE};
use serde::Serialize;
use std::{
    collections::HashMap,
    env,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, RwLock},
    task::JoinHandle,
    time::{interval, MissedTickBehavior},
};
use tracing::{debug, error, info, warn};

use crate::stage_connections::StageConnections;

mod adb;
use adb::*;
mod page_liveness;
use page_liveness::*;

const COMMAND_CHANNEL_CAPACITY: usize = 8;
const RETRY_INTERVAL: Duration = Duration::from_secs(20);

/// Server-wide env var carrying the stage URL the launcher opens on every
/// Android stage display (e.g. `http://10.77.9.205/stage`). Set per environment
/// in the deploy systemd units. When unset/empty the launcher skips launching.
const STAGE_URL_ENV: &str = "PRESENTER_ANDROID_STAGE_URL";

/// Server-wide env var pointing at the Presenter Stage APK on disk. The watchdog
/// installs this APK via ADB on any TV whose configured launch package is our
/// own app ([`DEFAULT_LAUNCH_PACKAGE`]) but does not have it installed yet — so
/// the stage runs on ANY Android TV (incl. ones with no browser, e.g. Sharp/
/// MediaTek) without a third-party kiosk browser. Defaults to
/// [`DEFAULT_STAGE_APK_PATH`]; when the file is absent, install is skipped (the
/// TV must already have the app, or a browser package configured).
const STAGE_APK_ENV: &str = "PRESENTER_ANDROID_STAGE_APK";
const DEFAULT_STAGE_APK_PATH: &str = "/opt/presenter/presenter-stage.apk";

/// Per-brand kiosk/browser packages the TV's own system resurfaces to the
/// foreground, fighting our stage app (#477). When our own app is the launch
/// target, the watchdog disables these so OUR app holds. `pm disable-user` is
/// reversible (a later `pm enable` restores them) and these are NOT the TV's
/// HOME launcher (verified on the TCL: Google TV launcher is HOME), so disabling
/// them does not break the device.
const KIOSK_PACKAGES_TO_SUPPRESS: &[&str] = &["com.tcl.browser", "com.fullykiosk.videokiosk"];

/// The `versionCode` of the bundled Presenter Stage APK. MUST be kept in lockstep
/// with `android/presenter-stage/app/build.gradle.kts` `versionCode`. The watchdog
/// compares this against the versionCode installed on a TV and reinstalls when the
/// TV's copy is older — so bumping the app's versionCode (and this constant)
/// actually ships the new APK to TVs that already have an older one. Without this
/// comparison, an already-installed (but stale) app would never be upgraded.
const EXPECTED_STAGE_APK_VERSION_CODE: i64 = 3;

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

/// Resolve the Presenter Stage APK path from [`STAGE_APK_ENV`] (default
/// [`DEFAULT_STAGE_APK_PATH`]). Returns `Some(path)` only when the file actually
/// exists, so the watchdog never attempts an `adb install` of a missing file.
/// Resolve the Presenter Stage APK path from the [`STAGE_APK_ENV`] value: fall
/// back to [`DEFAULT_STAGE_APK_PATH`] when empty/unset, then return the path only
/// if the file actually exists (so the watchdog never tries to install a missing
/// file). Kept as a pure function of its argument so it is testable without
/// touching the process environment; the caller passes `env::var(...).ok()`.
fn resolve_apk_path_from(raw: Option<String>) -> Option<PathBuf> {
    let raw = raw
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_STAGE_APK_PATH.to_string());
    let path = PathBuf::from(raw);
    path.is_file().then_some(path)
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
    /// Path to the Presenter Stage APK to install on TVs that should run our own
    /// app but don't have it yet. `None` when the configured/default APK file is
    /// absent (then install is skipped — see [`STAGE_APK_ENV`]).
    apk_path: Arc<Option<PathBuf>>,
    /// #990: the live stage connections (real client IPs), attached once by
    /// `AppState`. Unset (a registry built without state) = no page check.
    stage_connections: Arc<OnceLock<StageConnections>>,
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
        let runner: Arc<dyn AdbRunner> = adb::make_process_runner();
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
        let apk_path = resolve_apk_path_from(env::var(STAGE_APK_ENV).ok());
        match &apk_path {
            Some(path) => {
                debug!(env = STAGE_APK_ENV, path = %path.display(), "presenter stage APK available for auto-install");
            }
            None => {
                debug!(
                    env = STAGE_APK_ENV,
                    "presenter stage APK not found — TVs must already have the app or a browser package configured"
                );
            }
        }
        Self {
            runner,
            stage_url: Arc::new(stage_url),
            apk_path: Arc::new(apk_path),
            stage_connections: Arc::new(OnceLock::new()),
            displays: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// #990: hand the keep-alive the registry of live stage connections, so
    /// a display whose app is foreground but whose page never connects gets
    /// a fresh page. Set once; workers read it on every tick.
    pub fn attach_stage_connections(&self, connections: StageConnections) {
        if self.stage_connections.set(connections).is_err() {
            debug!("android stage registry: stage connections already attached");
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
        let apk_path = Arc::clone(&self.apk_path);
        let connections = Arc::clone(&self.stage_connections);
        let status_clone = Arc::clone(&status);
        let config_clone = display.clone();
        let handle = tokio::spawn(async move {
            if let Err(err) = run_device_worker(
                runner,
                stage_url,
                apk_path,
                connections,
                config_clone,
                status_clone,
                command_rx,
            )
            .await
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
    apk_path: Arc<Option<PathBuf>>,
    connections: Arc<OnceLock<StageConnections>>,
    mut config: AndroidStageDisplay,
    status: Arc<RwLock<AndroidStageDisplayStatusSnapshot>>,
    mut command_rx: mpsc::Receiver<DeviceCommand>,
) -> anyhow::Result<()> {
    let mut ticker = interval(RETRY_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // #793: this display's adb link memory (stuck-cycle streak) across ticks.
    let mut link = AdbLinkState::default();
    // #990: whether this display's page actually connects.
    let mut page = PageLiveness::new(Instant::now());

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if config.is_enabled {
                    // Periodic keep-alive: foreground-aware (#419) — only
                    // relaunches when the browser is not already up, or (#990)
                    // when its page has not connected for 3 min.
                    if let Err(err) = keep_alive_tick(runner.as_ref(), &stage_url, &apk_path, &config, &status, &mut link, &mut page, connections.get(), Instant::now()).await {
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
                        // The host/port may have changed — start a fresh streak.
                        link = AdbLinkState::default();
                        page = PageLiveness::new(Instant::now());
                        if !config.is_enabled {
                            mark_disabled(&status).await;
                        }
                    }
                    DeviceCommand::LaunchNow => {
                        if config.is_enabled {
                            // Explicit/forced launch (startup, config change,
                            // launch-now endpoint): always (re)launch (#419);
                            // the fresh page gets the full 3 min to connect (#990).
                            if let Err(err) = forced_launch(runner.as_ref(), &stage_url, &apk_path, &config, &status, &mut link, &mut page, Instant::now()).await {
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

/// One adb cycle: connect, keep awake, install/suppress, then launch the
/// stage — on a tick only when the #419 foreground gate says so, always when
/// `force_launch`. `Ok(true)` = the VIEW intent fired, `Ok(false)` = skipped
/// (the stage page is already in front).
async fn connect_and_launch(
    runner: &dyn AdbRunner,
    stage_url: &Arc<Option<String>>,
    apk_path: &Arc<Option<PathBuf>>,
    config: &AndroidStageDisplay,
    status: &Arc<RwLock<AndroidStageDisplayStatusSnapshot>>,
    force_launch: bool,
    link: &mut AdbLinkState,
) -> anyhow::Result<bool> {
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

    if let Err(err) = adb_connect(runner, &serial, link).await {
        record_error(status, err.to_string()).await;
        return Err(err);
    }

    // #481: keep the stage TV awake. After moving off Fully Kiosk (which held the
    // screen on), the TVs dropped to standby (black screen, red LED) — the stage
    // stopped showing even though our app was foreground. The app's
    // FLAG_KEEP_SCREEN_ON only holds while it is the visible foreground and does
    // not override the TV's hardware display-sleep timeout. Disable that timeout
    // on every connect so a stage TV never sleeps (every display, any launcher).
    keep_screen_awake(runner, &serial).await;

    let launch_pkg = launch_package(&config.launch_component);

    if let Err(err) = prepare_our_stage_app(runner, &serial, launch_pkg, apk_path, link).await {
        record_error(status, err.to_string()).await;
        return Err(err);
    }

    // #419: foreground-aware keep-alive. On the periodic tick (force_launch =
    // false) skip the am-start when the browser is ALREADY the resumed app —
    // re-firing the VIEW intent reloads com.tcl.browser (black blink + spinner)
    // every cycle. Explicit/forced launches (config change, launch-now
    // endpoint, a dead page #990) bypass the gate and always relaunch.
    if !force_launch && stage_already_foreground(runner, &serial, launch_pkg, config, status).await
    {
        return Ok(false);
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
    Ok(true)
}

/// The #419 gate: is the stage page already the resumed app? An inconclusive
/// probe (None) answers "no" so a genuinely-down display still recovers. A
/// confirmed-foreground probe IS a liveness success: it refreshes
/// `last_success` so a healthy display that skips relaunch every cycle does
/// not show an ever-aging "last success" on the operator dashboard (#419
/// review).
async fn stage_already_foreground(
    runner: &dyn AdbRunner,
    serial: &str,
    launch_pkg: &str,
    config: &AndroidStageDisplay,
    status: &Arc<RwLock<AndroidStageDisplayStatusSnapshot>>,
) -> bool {
    let foreground = adb_foreground_component(runner, serial).await;
    if should_launch_stage(foreground.as_deref(), launch_pkg) {
        info!(
            display = %config.label,
            foreground = foreground.as_deref().unwrap_or("<unknown>"),
            "android stage keep-alive: browser not foreground — relaunching (#419)"
        );
        return false;
    }
    debug!(
        display = %config.label,
        package = launch_pkg,
        foreground = foreground.as_deref().unwrap_or("<unknown>"),
        "android stage keep-alive: stage page already foreground — skipping relaunch (#419)"
    );
    let mut guard = status.write().await;
    guard.state = AndroidStageDisplayState::Running;
    guard.last_success = Some(Utc::now());
    guard.last_error = None;
    true
}

/// Ensure our own Presenter Stage app is installed before we launch it, so the
/// stage runs on ANY Android TV — including ones with no usable browser (e.g.
/// Sharp/MediaTek, where com.tcl.browser is absent) — without a kiosk browser.
/// Only our app ([`DEFAULT_LAUNCH_PACKAGE`]) is auto-installed; a legacy or
/// operator-set browser package is assumed already present on the device. An
/// error (install failure, or an unreadable package state — #793) aborts the
/// cycle. No-op for any other launch package.
async fn prepare_our_stage_app(
    runner: &dyn AdbRunner,
    serial: &str,
    launch_pkg: &str,
    apk_path: &Arc<Option<PathBuf>>,
    link: &mut AdbLinkState,
) -> anyhow::Result<()> {
    if launch_pkg != DEFAULT_LAUNCH_PACKAGE {
        return Ok(());
    }
    if let Some(apk) = apk_path.as_deref() {
        ensure_app_installed(runner, serial, launch_pkg, apk, link).await?;
    }
    // #477: suppress the old per-brand kiosk browsers. The TV's own system
    // keeps resurfacing them (e.g. TCL relaunches com.tcl.browser to the
    // foreground), so the watchdog otherwise fights a losing battle —
    // relaunching our app every tick while the browser flickers back. Disable
    // them so OUR app holds. Only done when our app is the launch target, so
    // an operator who deliberately set a browser as the launch_component is
    // never affected.
    suppress_kiosk_browsers(runner, serial).await;
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

/// Resumed-activity suffixes that mean the STAGE PAGE is genuinely showing:
/// - `StageActivity` — our own Presenter Stage app (its single activity);
/// - `BrowsePageActivity` — legacy `com.tcl.browser`'s content/browse activity.
///
/// When the foreground component ends with one of these AND its package is the
/// configured launch package, the stage is loaded and the keep-alive must NOT
/// relaunch (re-firing the VIEW intent would reload the page — the #419 black
/// blink). Any OTHER activity of the same package (notably com.tcl.browser's
/// home portal `.portal.home.activity.StartActivity` the TV opens to at
/// power-on — #447) is NOT the stage and MUST be (re)launched.
const STAGE_CONTENT_ACTIVITY_SUFFIXES: &[&str] = &["StageActivity", "BrowsePageActivity"];

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
            let stage_is_showing = package == launch_package
                && STAGE_CONTENT_ACTIVITY_SUFFIXES
                    .iter()
                    .any(|suffix| activity.ends_with(suffix));
            !stage_is_showing
        }
        // Foreground could not be determined → don't suppress a possibly-needed
        // launch; (re)launch and let the device sort it out.
        None => true,
    }
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
        // Accumulate whole chars until the next one would exceed the byte budget,
        // so multi-byte codepoints in adb output are never split (no panic) and
        // there is no decrement loop to mutate into an infinite loop.
        let mut truncated = String::new();
        for c in message.chars() {
            if truncated.len() + c.len_utf8() > MAX_LEN - 1 {
                break;
            }
            truncated.push(c);
        }
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::AndroidStageDisplayId;
    use uuid::Uuid;

    /// The watchdog's expected code and the built APK's stay in lockstep (#879):
    /// a drift never upgrades the TVs, or reinstalls the app on every pass.
    #[test]
    fn expected_version_code_matches_the_built_apk() {
        let gradle = include_str!("../../../android/presenter-stage/app/build.gradle.kts");
        let line = format!("versionCode = {EXPECTED_STAGE_APK_VERSION_CODE}\n");
        assert!(
            gradle.contains(&line),
            "expected `{line}` in build.gradle.kts"
        );
    }

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

    use crate::stage_connections::StageConnections;
    use async_trait::async_trait;
    use std::ffi::OsString;
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
        /// Whether `pm path <pkg>` reports the app installed. `false` models a TV
        /// that needs the Presenter Stage APK installed first.
        installed: bool,
        /// The versionCode `dumpsys package <pkg>` reports for the installed app.
        /// Defaults to [`EXPECTED_STAGE_APK_VERSION_CODE`] (up to date → no
        /// reinstall); lower models a stale install the watchdog must upgrade.
        installed_version: i64,
        /// When true, `dumpsys package` returns output with NO parseable
        /// `versionCode=` line (models a transient adb blip / truncated dumpsys),
        /// so `adb_installed_version_code` reads `None` even though the app is
        /// present (#734).
        version_unreadable: bool,
        /// #793: the stdout `adb devices` returns (empty by default → the
        /// target is not listed → a plain `adb connect`).
        devices_output: String,
        /// #793: when `Some`, `pm path <pkg>` fails with this adb error on
        /// stderr (non-zero exit) — the transient "device still authorizing" /
        /// "device offline" read that used to be mistaken for "not installed".
        pm_path_error: Option<String>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeAdbRunner {
        fn new(foreground: Foreground) -> Self {
            Self {
                foreground,
                connect_fails: false,
                installed: true,
                installed_version: EXPECTED_STAGE_APK_VERSION_CODE,
                version_unreadable: false,
                devices_output: String::new(),
                pm_path_error: None,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn with_connect_failure(foreground: Foreground) -> Self {
            Self {
                foreground,
                connect_fails: true,
                installed: true,
                installed_version: EXPECTED_STAGE_APK_VERSION_CODE,
                version_unreadable: false,
                devices_output: String::new(),
                pm_path_error: None,
                calls: Mutex::new(Vec::new()),
            }
        }

        /// Builder: the app IS installed but its versionCode is UNREADABLE
        /// (`dumpsys package` returns no parseable `versionCode=`), modelling the
        /// transient-read false-negative that drove the #734 tear-down.
        fn version_unreadable(mut self) -> Self {
            self.installed = true;
            self.version_unreadable = true;
            self
        }

        /// Builder (#793): `adb devices` lists the test target in `state`.
        fn devices_listing(mut self, state: &str) -> Self {
            self.devices_output = format!("List of devices attached\n10.0.0.42:5555\t{state}\n\n");
            self
        }

        /// Builder (#793): `pm path` fails with the given adb error.
        fn pm_path_fails(mut self, stderr: &str) -> Self {
            self.pm_path_error = Some(stderr.to_string());
            self
        }

        fn disconnect_calls(&self) -> usize {
            self.invocations()
                .iter()
                .filter(|c| c.starts_with("disconnect "))
                .count()
        }

        fn devices_calls(&self) -> usize {
            self.invocations()
                .iter()
                .filter(|c| *c == "devices")
                .count()
        }

        fn uninstall_calls(&self) -> usize {
            self.invocations()
                .iter()
                .filter(|c| c.contains("uninstall"))
                .count()
        }

        /// Builder: the app is NOT yet installed (so `pm path` reports nothing and
        /// the watchdog must `adb install` the APK).
        fn app_missing(mut self) -> Self {
            self.installed = false;
            self
        }

        /// Builder: the app IS installed but at an older versionCode, so the
        /// watchdog must upgrade it.
        fn installed_version(mut self, version: i64) -> Self {
            self.installed = true;
            self.installed_version = version;
            self
        }

        fn install_calls(&self) -> usize {
            self.invocations()
                .iter()
                .filter(|c| c.contains("install") && !c.contains("uninstall"))
                .count()
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

        fn dumpsys_package_calls(&self) -> usize {
            self.invocations()
                .iter()
                .filter(|c| c.contains("dumpsys package"))
                .count()
        }

        fn disable_user_calls(&self) -> usize {
            self.invocations()
                .iter()
                .filter(|c| c.contains("pm disable-user"))
                .count()
        }

        fn screen_awake_calls(&self) -> usize {
            self.invocations()
                .iter()
                .filter(|c| c.contains("settings put system screen_off_timeout"))
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

            // `adb devices` lists the connected targets and their states (#793).
            if joined == "devices" {
                return Ok(ok_output(&self.devices_output));
            }

            // `pm path <pkg>` reports install state (`package:` line = installed).
            if joined.contains("shell pm path") {
                if let Some(stderr) = &self.pm_path_error {
                    return Ok(err_output(stderr));
                }
                return Ok(ok_output(if self.installed {
                    "package:/data/app/test/base.apk"
                } else {
                    ""
                }));
            }

            // `dumpsys package <pkg>` reports the installed versionCode (empty
            // when the app is absent; no `versionCode=` line when the read is
            // modelled as unreadable — #734).
            if joined.contains("dumpsys package") {
                return Ok(ok_output(&if !self.installed {
                    String::new()
                } else if self.version_unreadable {
                    "    minSdk=22 targetSdk=34".to_string()
                } else {
                    format!(
                        "    versionCode={} minSdk=22 targetSdk=34",
                        self.installed_version
                    )
                }));
            }

            // `adb install [-r] <apk>` succeeds (prints `Success`); `uninstall`
            // is handled by the generic clean-success path below.
            if joined.contains("install") && !joined.contains("uninstall") {
                return Ok(ok_output("Success"));
            }

            // `adb connect <serial>` fails when the device is unreachable.
            if self.connect_fails && joined.starts_with("connect ") {
                return Ok(err_output("error: failed to connect to 'host:port'"));
            }

            // connect / disconnect / uninstall / `am start` all succeed cleanly.
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

    /// No APK configured → the watchdog never attempts an install (the default
    /// for the keep-alive/foreground tests, which use a browser package).
    fn no_apk() -> Arc<Option<PathBuf>> {
        Arc::new(None)
    }

    /// An APK is available on disk → install-if-missing can fire.
    fn some_apk() -> Arc<Option<PathBuf>> {
        Arc::new(Some(PathBuf::from("/opt/presenter/presenter-stage.apk")))
    }

    /// A display configured to launch our OWN app ([`DEFAULT_LAUNCH_PACKAGE`]),
    /// which is the only package the watchdog auto-installs.
    fn our_app_display() -> AndroidStageDisplay {
        let now = Utc::now();
        AndroidStageDisplay::new(
            AndroidStageDisplayId::from_uuid(Uuid::new_v4()),
            "Stage TV".to_string(),
            "10.0.0.42".to_string(),
            5555,
            DEFAULT_LAUNCH_PACKAGE.to_string(),
            true,
            now,
            now,
        )
    }

    // Our own app's single activity IS the stage page → keep-alive must NOT
    // relaunch when it is foreground; a different app must (re)launch.
    #[test]
    fn should_launch_stage_skips_for_our_stage_activity() {
        assert!(
            !should_launch_stage(
                Some("sk.newlevel.presenterstage/sk.newlevel.presenterstage.StageActivity"),
                "sk.newlevel.presenterstage",
            ),
            "our StageActivity foreground IS the stage — must not relaunch",
        );
        assert!(
            should_launch_stage(
                Some("com.android.tv.settings/.Main"),
                "sk.newlevel.presenterstage"
            ),
            "another app foreground must relaunch our stage",
        );
    }

    // A TV missing our app MUST get it installed via adb before the stage launch.
    #[tokio::test]
    async fn installs_our_app_when_missing_before_launch() {
        let runner = FakeAdbRunner::new(Foreground::StagePage).app_missing();
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = our_app_display();
        let status = test_status();

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &some_apk(),
            &config,
            &status,
            true,
            &mut AdbLinkState::default(),
        )
        .await;
        assert!(result.is_ok(), "install + launch must succeed: {result:?}");
        assert_eq!(
            runner.install_calls(),
            1,
            "a missing app MUST be installed via adb before launch",
        );
        assert_eq!(
            runner.am_start_calls(),
            1,
            "the stage must still be launched after install",
        );
        // The version check (dumpsys package) is ONLY reached when the package is
        // already present. An absent app must install directly without probing the
        // version — asserts adb_package_state actually gates that branch.
        assert_eq!(
            runner.dumpsys_package_calls(),
            0,
            "an absent app must NOT be version-probed before install",
        );
    }

    // An already-installed app MUST NOT be reinstalled on every launch.
    #[tokio::test]
    async fn skips_install_when_our_app_present() {
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = our_app_display();
        let status = test_status();

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &some_apk(),
            &config,
            &status,
            true,
            &mut AdbLinkState::default(),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(
            runner.install_calls(),
            0,
            "an already-installed app MUST NOT be reinstalled",
        );
    }

    // #734 REGRESSION: a present app whose versionCode read fails (transient adb
    // blip / truncated dumpsys) must NOT be reinstalled — the old "reinstall to
    // be safe" path tore down a healthy running app mid-event (the grey-play-
    // arrow surface feeding #732, which remains open). Present + unreadable
    // version = leave the running app in place, ZERO install/uninstall calls.
    #[tokio::test]
    async fn does_not_reinstall_when_present_but_version_unreadable() {
        let runner = FakeAdbRunner::new(Foreground::StagePage).version_unreadable();
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = our_app_display();
        let status = test_status();

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &some_apk(),
            &config,
            &status,
            true,
            &mut AdbLinkState::default(),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(
            runner.install_calls(),
            0,
            "#734: a healthy app with an unreadable versionCode must NOT be reinstalled",
        );
        assert_eq!(
            runner.uninstall_calls(),
            0,
            "#734: must not tear down (uninstall) a healthy running app",
        );
    }

    // An app present at a versionCode OLDER than the bundled APK MUST be upgraded
    // (the versionCode comparison the gradle comment promises). Without it, a
    // bumped APK never reaches a TV that already has an older copy.
    #[tokio::test]
    async fn upgrades_our_app_when_installed_version_is_stale() {
        let runner = FakeAdbRunner::new(Foreground::StagePage)
            .installed_version(EXPECTED_STAGE_APK_VERSION_CODE - 1);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = our_app_display();
        let status = test_status();

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &some_apk(),
            &config,
            &status,
            true,
            &mut AdbLinkState::default(),
        )
        .await;
        assert!(result.is_ok(), "upgrade + launch must succeed: {result:?}");
        assert_eq!(
            runner.install_calls(),
            1,
            "a stale (older versionCode) app MUST be upgraded via adb install",
        );
    }

    // A present app at the EXACT expected versionCode must NOT be reinstalled
    // (boundary: installed == expected → up to date).
    #[tokio::test]
    async fn skips_install_when_installed_version_matches_expected() {
        let runner = FakeAdbRunner::new(Foreground::StagePage)
            .installed_version(EXPECTED_STAGE_APK_VERSION_CODE);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = our_app_display();
        let status = test_status();

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &some_apk(),
            &config,
            &status,
            true,
            &mut AdbLinkState::default(),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(
            runner.install_calls(),
            0,
            "an app already at the expected versionCode MUST NOT be reinstalled",
        );
        assert!(
            runner.dumpsys_package_calls() >= 1,
            "a present app MUST be version-probed (dumpsys package) to decide upgrade",
        );
    }

    #[test]
    fn parse_version_code_extracts_first_integer_or_none() {
        assert_eq!(
            parse_version_code("    versionCode=7 minSdk=22 targetSdk=34"),
            Some(7),
            "must parse the integer right after versionCode=",
        );
        assert_eq!(
            parse_version_code("flags=[ DEBUGGABLE ]\n    versionCode=42 ..."),
            Some(42),
        );
        assert_eq!(
            parse_version_code("no version field here"),
            None,
            "absent versionCode must yield None (→ reinstall to be safe)",
        );
        assert_eq!(
            parse_version_code("versionCode= notanumber"),
            None,
            "non-numeric versionCode must yield None",
        );
    }

    #[test]
    fn truncate_error_is_utf8_safe_at_boundary() {
        // 'č' (2 bytes) occupies bytes 278-279; the cut index MAX_LEN-1 = 279
        // lands MID-codepoint. Correct code walks DOWN to byte 278 (the char
        // boundary at the start of 'č'), so the result is exactly the 278 'x's
        // plus the ellipsis — 'č' is dropped, never split. Asserting the EXACT
        // string kills the boundary-walk mutants (end += 1 would instead keep
        // 'č'; && → || would walk all the way to 0).
        let message = "x".repeat(278) + "č" + &"y".repeat(20);
        let out = truncate_error(&message);
        assert_eq!(
            out,
            format!("{}…", "x".repeat(278)),
            "must cut on the char boundary below the limit (drop the split 'č')",
        );
        // Short messages pass through unchanged.
        assert_eq!(truncate_error("short"), "short");
        // A message exactly at the cap passes through unchanged (boundary of the
        // len() <= MAX_LEN branch).
        assert_eq!(truncate_error(&"a".repeat(280)), "a".repeat(280));
        // All single-byte chars: the budget cut lands EXACTLY at MAX_LEN-1 = 279,
        // so the result is exactly 279 'a's + ellipsis. This pins the budget
        // comparison precisely — `> → >=` would keep 278, `+ → *` would keep 280.
        assert_eq!(
            truncate_error(&"a".repeat(400)),
            format!("{}…", "a".repeat(279)),
            "byte budget must keep exactly MAX_LEN-1 single-byte chars",
        );
    }

    // A browser package (e.g. com.tcl.browser) is assumed pre-installed — the
    // watchdog must NOT try to install it even when an APK is available.
    #[tokio::test]
    async fn skips_install_for_non_default_package() {
        let runner = FakeAdbRunner::new(Foreground::StagePage).app_missing();
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &some_apk(),
            &config,
            &status,
            true,
            &mut AdbLinkState::default(),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(
            runner.install_calls(),
            0,
            "only our own app is auto-installed; a browser package is left alone",
        );
        // #477: a non-our-app (operator-set browser) display must NOT have kiosk
        // browsers disabled — only our-app displays do.
        assert_eq!(
            runner.disable_user_calls(),
            0,
            "kiosk browsers must NOT be disabled when the launch target is a browser",
        );
    }

    // #481: every connect disables the TV's display-sleep timeout so a stage TV
    // never drops to standby — for ANY display (our app or an operator browser).
    #[tokio::test]
    async fn keeps_stage_tv_awake_on_connect() {
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = our_app_display();
        let status = test_status();

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &some_apk(),
            &config,
            &status,
            true,
            &mut AdbLinkState::default(),
        )
        .await;
        assert!(result.is_ok());
        assert!(
            runner.screen_awake_calls() >= 1,
            "connect must disable the display-sleep timeout (settings put screen_off_timeout)",
        );
        let calls = runner.invocations().join("\n");
        assert!(
            calls.contains("settings put system screen_off_timeout 2147483647"),
            "screen_off_timeout must be set to the never-sleep max",
        );
    }

    // #477: when our app is the launch target, the watchdog disables the known
    // kiosk browsers (pm disable-user) so the TV cannot resurface them over the
    // stage — one disable call per package in KIOSK_PACKAGES_TO_SUPPRESS.
    #[tokio::test]
    async fn suppresses_kiosk_browsers_for_our_app_display() {
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = our_app_display();
        let status = test_status();

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &some_apk(),
            &config,
            &status,
            true,
            &mut AdbLinkState::default(),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(
            runner.disable_user_calls(),
            KIOSK_PACKAGES_TO_SUPPRESS.len(),
            "every kiosk browser must be disabled when our app is the launch target",
        );
        // Each configured kiosk package is the one disabled.
        let calls = runner.invocations().join("\n");
        for pkg in KIOSK_PACKAGES_TO_SUPPRESS {
            assert!(
                calls.contains(&format!("pm disable-user --user 0 {pkg}")),
                "must disable-user {pkg}",
            );
        }
    }

    // APK path resolution: an existing configured file resolves to itself; a
    // missing path (or empty → default, absent in tests) resolves to None so the
    // watchdog never tries to install a non-existent file.
    #[test]
    fn resolve_apk_path_uses_existing_file_and_rejects_missing() {
        let f =
            std::env::temp_dir().join(format!("presenter-stage-test-{}.apk", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(
            resolve_apk_path_from(Some(f.to_string_lossy().into_owned())),
            Some(f.clone()),
            "an existing configured APK path must resolve to itself",
        );
        std::fs::remove_file(&f).ok();
        assert_eq!(
            resolve_apk_path_from(Some("/no/such/dir/presenter-stage.apk".to_string())),
            None,
            "a missing APK path must resolve to None (install skipped)",
        );
        assert_eq!(
            resolve_apk_path_from(Some(String::new())),
            None,
            "empty falls back to the default path, which is absent under test → None",
        );
    }

    // `adb install` success requires BOTH a success exit AND a `Success` line —
    // a `Failure [..]` printed on a zero exit must NOT count as installed.
    #[test]
    fn adb_install_succeeded_requires_success_exit_and_marker() {
        assert!(
            adb_install_succeeded(&ok_output("Success")),
            "exit 0 + Success ⇒ installed",
        );
        assert!(
            !adb_install_succeeded(&ok_output("Failure [INSTALL_FAILED_VERSION_DOWNGRADE]")),
            "exit 0 but no Success marker ⇒ NOT installed",
        );
        assert!(
            !adb_install_succeeded(&err_output("Success")),
            "non-zero exit ⇒ NOT installed",
        );
    }

    // (a) Periodic keep-alive, stage page already foreground → NO `am start`.
    #[tokio::test]
    async fn tick_skips_am_start_when_stage_page_foreground() {
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();

        // force_launch=false models the periodic tick.
        let result = connect_and_launch(
            &runner,
            &stage_url,
            &no_apk(),
            &config,
            &status,
            false,
            &mut AdbLinkState::default(),
        )
        .await;
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

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &no_apk(),
            &config,
            &status,
            false,
            &mut AdbLinkState::default(),
        )
        .await;
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

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &no_apk(),
            &config,
            &status,
            false,
            &mut AdbLinkState::default(),
        )
        .await;
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

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &no_apk(),
            &config,
            &status,
            false,
            &mut AdbLinkState::default(),
        )
        .await;
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

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &no_apk(),
            &config,
            &status,
            true,
            &mut AdbLinkState::default(),
        )
        .await;
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

        let result = connect_and_launch(
            &runner,
            &stage_url,
            &no_apk(),
            &config,
            &status,
            true,
            &mut AdbLinkState::default(),
        )
        .await;
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

        run_device_worker(
            runner,
            stage_url,
            no_apk(),
            Arc::new(OnceLock::new()),
            config,
            status,
            rx,
        )
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

    // ── #793: tri-state package check + non-churning adb connect ────────────
    //
    // SNV event 2026-09-27: SD3's `pm path` failed with "device still
    // authorizing"; the old bool check read that as "not installed" and ran
    // `adb install -r` → uninstall + install, killing the running stage app
    // mid-event (logcat `due to installPackageLI`). And `adb_connect` did
    // `adb disconnect` + `adb connect` every 20 s per TV, itself producing
    // those transient offline/authorizing states.

    /// An `Output` with an arbitrary exit code (wait-status encoding).
    fn exit_output(code: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    /// The exact adb client errors seen on the SNV TVs (#793 findings).
    const REAL_ADB_ERRORS: [&str; 3] = [
        "error: device still authorizing",
        "error: device offline",
        "error: device '10.77.9.33:5555' not found",
    ];

    #[test]
    fn package_state_installed_from_package_line() {
        assert_eq!(
            parse_package_state(&ok_output(
                "package:/data/app/~~Qx1==/sk.example.presenterstage-2/base.apk\n"
            )),
            PackageState::Installed,
        );
    }

    #[test]
    fn package_state_not_installed_on_clean_empty_exit() {
        assert_eq!(
            parse_package_state(&ok_output("")),
            PackageState::NotInstalled,
            "empty stdout + clean exit = genuinely absent",
        );
        // Android 11+ `pm path <absent>` exits 1 with no output at all.
        assert_eq!(
            parse_package_state(&exit_output(1, "", "")),
            PackageState::NotInstalled,
            "`pm path` of an absent package exits 1 silently — still NotInstalled",
        );
    }

    #[test]
    fn package_state_unknown_on_real_adb_errors() {
        for stderr in REAL_ADB_ERRORS {
            match parse_package_state(&err_output(stderr)) {
                PackageState::Unknown(detail) => assert!(
                    detail.contains(stderr),
                    "Unknown must carry the adb error text for the log: {detail}",
                ),
                other => panic!("`{stderr}` must be Unknown, got {other:?}"),
            }
        }
    }

    #[test]
    fn package_state_unknown_on_unexpected_exit_or_output() {
        assert!(
            matches!(
                parse_package_state(&exit_output(255, "", "")),
                PackageState::Unknown(_)
            ),
            "an adb-level failure exit (255) is not evidence of absence",
        );
        assert!(
            matches!(
                parse_package_state(&ok_output("Exception occurred while executing 'path'")),
                PackageState::Unknown(_)
            ),
            "unexpected stdout without a `package:` line is not evidence of absence",
        );
    }

    // #793 REGRESSION: an unreadable package state (any real adb error) must
    // NEVER install or uninstall — it skips the cycle (error recorded, no
    // `am start` on a flaky link).
    #[tokio::test]
    async fn never_installs_or_uninstalls_when_package_state_unknown() {
        for stderr in REAL_ADB_ERRORS {
            let runner = FakeAdbRunner::new(Foreground::OtherApp).pm_path_fails(stderr);
            let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
            let config = our_app_display();
            let status = test_status();

            let result = connect_and_launch(
                &runner,
                &stage_url,
                &some_apk(),
                &config,
                &status,
                false,
                &mut AdbLinkState::default(),
            )
            .await;
            assert!(
                result.is_err(),
                "`{stderr}`: an unknown package state must skip this cycle",
            );
            assert_eq!(
                runner.install_calls(),
                0,
                "`{stderr}`: must NOT install on an unknown package state (#793)",
            );
            assert_eq!(
                runner.uninstall_calls(),
                0,
                "`{stderr}`: must NOT uninstall (kill) the running app (#793)",
            );
            assert_eq!(
                runner.am_start_calls(),
                0,
                "`{stderr}`: a skipped cycle fires no `am start`",
            );
            assert_eq!(
                status.read().await.state,
                AndroidStageDisplayState::Error,
                "`{stderr}`: the skipped cycle surfaces as Error on the dashboard",
            );
        }
    }

    #[test]
    fn parse_adb_devices_reads_serial_states() {
        let out = "* daemon not running; starting now at tcp:5037\n\
                   * daemon started successfully\n\
                   List of devices attached\n\
                   10.77.9.32:5555\tdevice\n\
                   10.77.9.33:5555\toffline\n\
                   10.77.9.34:5555\tunauthorized\n\
                   emulator-5554          device product:sdk model:x\n\
                   \n";
        let map = parse_adb_devices(out);
        assert_eq!(
            map.get("10.77.9.32:5555").map(String::as_str),
            Some("device")
        );
        assert_eq!(
            map.get("10.77.9.33:5555").map(String::as_str),
            Some("offline")
        );
        assert_eq!(
            map.get("10.77.9.34:5555").map(String::as_str),
            Some("unauthorized")
        );
        assert_eq!(map.get("emulator-5554").map(String::as_str), Some("device"));
        assert_eq!(map.len(), 4, "header/daemon lines are not devices: {map:?}");
    }

    #[test]
    fn connect_decision_never_touches_a_connected_target() {
        let mut link = AdbLinkState::default();
        for _ in 0..5 {
            assert_eq!(
                decide_connect(Some("device"), &mut link),
                ConnectAction::AlreadyConnected,
            );
        }
        assert_eq!(decide_connect(None, &mut link), ConnectAction::Connect);
    }

    #[test]
    fn connect_decision_disconnects_only_after_three_stuck_cycles() {
        let mut link = AdbLinkState::default();
        assert_eq!(
            decide_connect(Some("offline"), &mut link),
            ConnectAction::Connect
        );
        assert_eq!(
            decide_connect(Some("offline"), &mut link),
            ConnectAction::Connect
        );
        assert_eq!(
            decide_connect(Some("offline"), &mut link),
            ConnectAction::Reconnect,
            "the 3rd consecutive stuck cycle drops the stale entry",
        );
        // The streak restarts after the reconnect …
        assert_eq!(
            decide_connect(Some("unauthorized"), &mut link),
            ConnectAction::Connect
        );
        assert_eq!(
            decide_connect(Some("unauthorized"), &mut link),
            ConnectAction::Connect
        );
        // … and a healthy `device` cycle in between resets it.
        assert_eq!(
            decide_connect(Some("device"), &mut link),
            ConnectAction::AlreadyConnected
        );
        assert_eq!(
            decide_connect(Some("offline"), &mut link),
            ConnectAction::Connect
        );
        assert_eq!(
            decide_connect(Some("offline"), &mut link),
            ConnectAction::Connect
        );
    }

    // #793: a target `adb devices` already lists as `device` gets NO
    // disconnect and NO connect on the keep-alive tick.
    #[tokio::test]
    async fn tick_leaves_a_connected_target_alone() {
        let runner = FakeAdbRunner::new(Foreground::StagePage).devices_listing("device");
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();
        let mut link = AdbLinkState::default();

        for _ in 0..3 {
            let result = connect_and_launch(
                &runner,
                &stage_url,
                &no_apk(),
                &config,
                &status,
                false,
                &mut link,
            )
            .await;
            assert!(
                result.is_ok(),
                "healthy keep-alive must succeed: {result:?}"
            );
        }
        assert_eq!(
            runner.devices_calls(),
            3,
            "`adb devices` read once per cycle"
        );
        assert_eq!(
            runner.connect_calls(),
            0,
            "a `device` target is never re-connected"
        );
        assert_eq!(
            runner.disconnect_calls(),
            0,
            "a `device` target is never disconnected (#793 churn)",
        );
    }

    // #793: a target stuck `offline` is plainly re-connected, and only on the
    // 3rd consecutive stuck cycle is its stale entry disconnected first.
    #[tokio::test]
    async fn stuck_offline_target_disconnected_only_on_third_cycle() {
        let runner = FakeAdbRunner::new(Foreground::StagePage).devices_listing("offline");
        let stage_url = Arc::new(Some(TEST_STAGE_URL.to_string()));
        let config = test_display();
        let status = test_status();
        let mut link = AdbLinkState::default();

        for _ in 0..2 {
            let _ = connect_and_launch(
                &runner,
                &stage_url,
                &no_apk(),
                &config,
                &status,
                false,
                &mut link,
            )
            .await;
        }
        assert_eq!(
            runner.connect_calls(),
            2,
            "stuck target: plain connect each cycle"
        );
        assert_eq!(
            runner.disconnect_calls(),
            0,
            "no disconnect before 3 stuck cycles"
        );

        let _ = connect_and_launch(
            &runner,
            &stage_url,
            &no_apk(),
            &config,
            &status,
            false,
            &mut link,
        )
        .await;
        assert_eq!(
            runner.disconnect_calls(),
            1,
            "3rd stuck cycle drops the stale entry"
        );
        assert_eq!(runner.connect_calls(), 3, "and reconnects it");
    }

    // ── #793 review: rate-limited adb logs, never re-prompt an RSA dialog ──

    // Disconnecting an `unauthorized`/`authorizing` target re-shows the RSA
    // prompt on the TV while someone may be accepting it — only a stale
    // `offline` entry is ever dropped.
    #[test]
    fn unauthorized_target_is_never_disconnected() {
        for state in ["unauthorized", "authorizing"] {
            let mut link = AdbLinkState::default();
            for cycle in 1..=6 {
                assert_eq!(
                    decide_connect(Some(state), &mut link),
                    ConnectAction::Connect,
                    "`{state}` cycle {cycle}: plain connect, never a disconnect",
                );
            }
        }
    }

    // .claude/rules/log-flood-backoff.md: log the 1st + power-of-two milestones.
    #[test]
    fn adb_log_gate_is_first_plus_powers_of_two() {
        let logged: Vec<bool> = (0..=9).map(should_log_adb_streak).collect();
        assert_eq!(
            logged,
            [false, true, true, false, true, false, false, false, true, false]
        );
    }

    // The per-cycle "package state unknown" WARN must not flood journald for a
    // TV stuck in an adb error (~180 lines/h/TV before).
    #[test]
    fn unknown_package_warn_is_rate_limited() {
        let mut link = AdbLinkState::default();
        let logged: Vec<bool> = (0..8).map(|_| note_unknown_package(&mut link)).collect();
        assert_eq!(logged, [true, true, false, true, false, false, false, true]);
    }

    // #793 re-review: an `unauthorized`/`authorizing` cycle between `offline`
    // cycles breaks the streak — the stale-entry disconnect needs 3
    // CONSECUTIVE `offline` cycles, never counts across an RSA prompt.
    #[test]
    fn rsa_prompt_cycle_resets_the_stale_streak() {
        let mut link = AdbLinkState::default();
        for state in ["offline", "offline", "unauthorized", "offline", "offline"] {
            assert_eq!(
                decide_connect(Some(state), &mut link),
                ConnectAction::Connect,
                "`{state}`: no disconnect across an RSA-prompt cycle",
            );
        }
        assert_eq!(
            decide_connect(Some("offline"), &mut link),
            ConnectAction::Reconnect,
            "3 consecutive offline cycles after the prompt → drop the stale entry",
        );
    }

    // ── #990: page-level keep-alive ─────────────────────────────────────────
    // "Our app is foreground" is not proof of life: SD2 showed a dark page for
    // 11 h with StageActivity in front. A display with no live stage
    // connection from its address for 3 min gets the VIEW intent re-fired, at
    // most once per 5 min.

    const DISPLAY_IP: &str = "10.0.0.42"; // test_display()'s host

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    /// One keep-alive tick against `connections` at `now`, our stage page
    /// foreground (the #419 gate alone would skip the relaunch).
    async fn page_tick(
        runner: &FakeAdbRunner,
        page: &mut PageLiveness,
        connections: Option<&StageConnections>,
        now: std::time::Instant,
    ) -> anyhow::Result<bool> {
        keep_alive_tick(
            runner,
            &Arc::new(Some(TEST_STAGE_URL.to_string())),
            &no_apk(),
            &test_display(),
            &test_status(),
            &mut AdbLinkState::default(),
            page,
            connections,
            now,
        )
        .await
    }

    async fn live_connection_from(connections: &StageConnections, ip: &str) -> Uuid {
        let id = Uuid::new_v4();
        connections.register(id, "worship-snv", Utc::now()).await;
        connections
            .set_client_ip(id, Some(ip.parse().expect("test ip")))
            .await;
        id
    }

    #[test]
    fn page_liveness_schedule_waits_3_min_then_5_min_between_relaunches() {
        let t0 = std::time::Instant::now();
        let mut page = PageLiveness::new(t0);
        page.observe(Some(false), t0 + secs(179));
        assert!(!page.relaunch_due(t0 + secs(179)), "3 min not over yet");
        page.observe(Some(false), t0 + secs(180));
        assert!(
            page.relaunch_due(t0 + secs(180)),
            "3 min without a connection"
        );

        page.note_relaunch_attempt(t0 + secs(180));
        page.note_launch(t0 + secs(180));
        page.observe(Some(false), t0 + secs(360));
        assert!(
            !page.relaunch_due(t0 + secs(360)),
            "silent 3 min again, but the last relaunch was < 5 min ago"
        );
        assert!(
            page.relaunch_due(t0 + secs(480)),
            "5 min after the relaunch"
        );

        page.observe(Some(true), t0 + secs(500));
        assert!(
            !page.relaunch_due(t0 + secs(600)),
            "a live page resets the silence"
        );
        page.observe(None, t0 + secs(900));
        assert!(
            page.relaunch_due(t0 + secs(900)),
            "an unknown reading neither proves nor disproves life"
        );
    }

    #[tokio::test]
    async fn foreground_page_without_a_connection_for_3_min_is_relaunched() {
        let capture = crate::tracing_capture::Capture::install();
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let connections = StageConnections::new();
        let t0 = std::time::Instant::now();
        let mut page = PageLiveness::new(t0);

        let early = page_tick(&runner, &mut page, Some(&connections), t0 + secs(60)).await;
        assert!(matches!(early, Ok(false)), "{early:?}");
        assert_eq!(
            runner.am_start_calls(),
            0,
            "1 min of silence is not dead yet"
        );

        let late = page_tick(&runner, &mut page, Some(&connections), t0 + secs(200)).await;
        assert!(matches!(late, Ok(true)), "{late:?}");
        assert_eq!(
            runner.am_start_calls(),
            1,
            "app foreground but no live stage connection for 3 min → re-fire the VIEW intent (#990)"
        );
        let lines = capture.events_with_message("re-fired the stage page (#990)");
        assert_eq!(lines.len(), 1, "one INFO per relaunch");
        assert_eq!(lines[0].level, tracing::Level::INFO);
        assert_eq!(lines[0].field("display"), Some("Stage TV"));
    }

    #[tokio::test]
    async fn foreground_page_with_a_live_connection_is_left_alone() {
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let connections = StageConnections::new();
        live_connection_from(&connections, DISPLAY_IP).await;
        let t0 = std::time::Instant::now();
        let mut page = PageLiveness::new(t0);

        for minute in 1..=20 {
            let result = page_tick(
                &runner,
                &mut page,
                Some(&connections),
                t0 + secs(60 * minute),
            )
            .await;
            assert!(matches!(result, Ok(false)), "{result:?}");
        }
        assert_eq!(
            runner.am_start_calls(),
            0,
            "a page with a live stage connection is never reloaded (#419 no-blink)"
        );
    }

    #[tokio::test]
    async fn another_displays_connection_does_not_keep_this_page_alive() {
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let connections = StageConnections::new();
        live_connection_from(&connections, "10.0.0.99").await;
        let t0 = std::time::Instant::now();
        let mut page = PageLiveness::new(t0);

        let result = page_tick(&runner, &mut page, Some(&connections), t0 + secs(181)).await;
        assert!(matches!(result, Ok(true)), "{result:?}");
        assert_eq!(runner.am_start_calls(), 1);
    }

    #[tokio::test]
    async fn a_disconnected_page_is_relaunched_after_3_min() {
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let connections = StageConnections::new();
        let id = live_connection_from(&connections, DISPLAY_IP).await;
        let t0 = std::time::Instant::now();
        let mut page = PageLiveness::new(t0);

        let alive = page_tick(&runner, &mut page, Some(&connections), t0 + secs(600)).await;
        assert!(matches!(alive, Ok(false)), "{alive:?}");
        connections.mark_disconnected(id).await;
        let soon = page_tick(&runner, &mut page, Some(&connections), t0 + secs(700)).await;
        assert!(matches!(soon, Ok(false)), "{soon:?}");
        assert_eq!(runner.am_start_calls(), 0, "dead for 100 s only");
        let dead = page_tick(&runner, &mut page, Some(&connections), t0 + secs(781)).await;
        assert!(matches!(dead, Ok(true)), "{dead:?}");
        assert_eq!(
            runner.am_start_calls(),
            1,
            "3 min after the last live reading"
        );
    }

    #[tokio::test]
    async fn fruitless_page_relaunches_back_off_and_log_on_powers_of_two() {
        // Review of 0ef6783c: a flat 5 min gap reloaded a display whose page
        // connects from an address its host does not resolve to every 5 min
        // forever (a blink + an INFO line each, ~288/day). Fruitless
        // relaunches now back off 5, 10, 20, 40, 60 min (the #484 pattern).
        let capture = crate::tracing_capture::Capture::install();
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let connections = StageConnections::new();
        let t0 = std::time::Instant::now();
        let mut page = PageLiveness::new(t0);

        // A page that never connects: every tick for 3 h is silent.
        let mut relaunched_at = Vec::new();
        let mut tick = 0;
        while tick <= 3 * 3600 {
            let result = page_tick(&runner, &mut page, Some(&connections), t0 + secs(tick)).await;
            assert!(result.is_ok(), "{result:?}");
            if matches!(result, Ok(true)) {
                relaunched_at.push(tick);
            }
            tick += 20;
        }
        assert_eq!(
            relaunched_at,
            vec![180, 480, 1080, 2280, 4680, 8280],
            "3 min of silence, then gaps of 5, 10, 20, 40, 60 min — never one per 20 s tick"
        );
        let info: Vec<String> = capture
            .events_with_message("re-fired the stage page (#990)")
            .iter()
            .filter(|line| line.level == tracing::Level::INFO)
            .filter_map(|line| line.field("relaunches").map(str::to_string))
            .collect();
        assert_eq!(
            info,
            ["1", "2", "4"],
            "INFO on the 1st, 2nd, 4th relaunch only"
        );

        // The page finally connects: one recovery line, the streak ends.
        live_connection_from(&connections, DISPLAY_IP).await;
        let healed = page_tick(&runner, &mut page, Some(&connections), t0 + secs(tick)).await;
        assert!(matches!(healed, Ok(false)), "{healed:?}");
        let recovered = capture.events_with_message("the stage page is connected again (#990)");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].field("relaunches"), Some("6"));
    }

    #[test]
    fn relaunch_gap_doubles_up_to_an_hour() {
        let gaps: Vec<u64> = (0..=8).map(|n| relaunch_gap(n).as_secs() / 60).collect();
        assert_eq!(gaps, [5, 5, 10, 20, 40, 60, 60, 60, 60]);
        assert_eq!(relaunch_gap(u32::MAX), PAGE_RELAUNCH_MAX_GAP, "no overflow");
    }

    #[tokio::test]
    async fn an_explicit_launch_gives_the_fresh_page_the_full_3_min() {
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let connections = StageConnections::new();
        let t0 = std::time::Instant::now();
        let mut page = PageLiveness::new(t0);

        let launched = forced_launch(
            &runner,
            &Arc::new(Some(TEST_STAGE_URL.to_string())),
            &no_apk(),
            &test_display(),
            &test_status(),
            &mut AdbLinkState::default(),
            &mut page,
            t0 + secs(170),
        )
        .await;
        assert!(matches!(launched, Ok(true)), "{launched:?}");
        assert_eq!(runner.am_start_calls(), 1);

        let soon = page_tick(&runner, &mut page, Some(&connections), t0 + secs(200)).await;
        assert!(matches!(soon, Ok(false)), "{soon:?}");
        assert_eq!(
            runner.am_start_calls(),
            1,
            "30 s after a launch is not dead yet"
        );
        let dead = page_tick(&runner, &mut page, Some(&connections), t0 + secs(351)).await;
        assert!(matches!(dead, Ok(true)), "{dead:?}");
        assert_eq!(
            runner.am_start_calls(),
            2,
            "3 min after the explicit launch"
        );
    }

    #[tokio::test]
    async fn no_registry_means_no_page_relaunch() {
        let runner = FakeAdbRunner::new(Foreground::StagePage);
        let t0 = std::time::Instant::now();
        let mut page = PageLiveness::new(t0);
        let result = page_tick(&runner, &mut page, None, t0 + secs(3600)).await;
        assert!(matches!(result, Ok(false)), "{result:?}");
        assert_eq!(
            runner.am_start_calls(),
            0,
            "unknown liveness keeps the #419 behaviour"
        );
    }

    #[tokio::test]
    async fn a_failed_relaunch_logs_no_info_and_still_counts_for_the_rate_limit() {
        let capture = crate::tracing_capture::Capture::install();
        let runner = FakeAdbRunner::with_connect_failure(Foreground::StagePage);
        let connections = StageConnections::new();
        let t0 = std::time::Instant::now();
        let mut page = PageLiveness::new(t0);

        let first = page_tick(&runner, &mut page, Some(&connections), t0 + secs(200)).await;
        assert!(first.is_err(), "an unreachable TV fails the cycle");
        let again = page_tick(&runner, &mut page, Some(&connections), t0 + secs(220)).await;
        assert!(again.is_err());
        assert_eq!(runner.am_start_calls(), 0);
        assert!(
            capture
                .events_with_message("re-fired the stage page (#990)")
                .is_empty(),
            "a TV that is off logs no relaunch line"
        );
        assert!(!page.relaunch_due(t0 + secs(220)), "the attempt counts");
        assert!(page.relaunch_due(t0 + secs(500)), "5 min after the attempt");
    }
}
