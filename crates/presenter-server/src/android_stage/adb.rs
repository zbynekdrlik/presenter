//! ADB I/O layer for the Android stage watchdog: the [`AdbRunner`] injection
//! seam + its process implementation, every `adb` command this crate runs, the
//! pure `adb`-output parsers, and the install/upgrade orchestration. Split out
//! of `android_stage.rs` to keep that file under the 1000-line size cap (#742).
//!
//! `android_stage.rs` flattens this module's namespace with `use adb::*;`, so the
//! watchdog (`connect_and_launch`, `run_device_worker`) and the existing test
//! module reference these items at their original unqualified paths — the split
//! is a pure relocation, zero behavior change.

use anyhow::anyhow;
use async_trait::async_trait;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use presenter_core::{stage_app_install_action, StageAppInstallAction};

use super::{EXPECTED_STAGE_APK_VERSION_CODE, KIOSK_PACKAGES_TO_SUPPRESS};

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
pub(super) trait AdbRunner: Send + Sync {
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
        match tokio::time::timeout(
            ADB_COMMAND_TIMEOUT,
            tokio::process::Command::new(self.adb_bin.as_os_str())
                .args(args)
                .output(),
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

/// Build the production [`AdbRunner`] from the `PRESENTER_ANDROID_ADB_BIN` env
/// (default `adb`). Kept with the adb layer so `android_stage.rs` never needs to
/// name `ProcessAdbRunner` / `OsString` directly.
pub(super) fn make_process_runner() -> Arc<dyn AdbRunner> {
    let adb_bin = std::env::var_os("PRESENTER_ANDROID_ADB_BIN")
        .map(Arc::from)
        .unwrap_or_else(|| Arc::new(OsString::from("adb")));
    Arc::new(ProcessAdbRunner { adb_bin })
}

/// Convenience for building an adb argument vector from string-ish parts.
pub(super) fn adb_args<I, S>(parts: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    parts
        .into_iter()
        .map(|p| p.as_ref().to_os_string())
        .collect()
}

/// Disconnect any stale entry then `adb connect <serial>`, returning an error
/// (without recording status) on timeout, exec failure, or a connect error.
///
/// The disconnect clears stale offline entries ADB leaves after a TV power
/// cycle, which otherwise make subsequent `-s serial` commands fail until the
/// daemon restarts. Its result is intentionally ignored — the typical case is
/// "not connected", a non-zero exit we don't care about.
pub(super) async fn adb_connect(runner: &dyn AdbRunner, serial: &str) -> anyhow::Result<()> {
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

/// Run `adb -s <serial> shell <launch_args>` (the `am start` VIEW intent),
/// returning an error (without recording status) on timeout, exec failure, or
/// a non-success `am start` result.
pub(super) async fn adb_launch(
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

/// Disable the TV's display-sleep timeout so a stage TV never drops to standby
/// (#481). `screen_off_timeout` is set to the i32 max (~24 days), effectively
/// "never". Best-effort: errors are ignored. Idempotent — safe every connect.
pub(super) async fn keep_screen_awake(runner: &dyn AdbRunner, serial: &str) {
    let _ = runner
        .run(&adb_args([
            "-s",
            serial,
            "shell",
            "settings",
            "put",
            "system",
            "screen_off_timeout",
            "2147483647",
        ]))
        .await;
}

/// Disable the known per-brand kiosk browsers ([`KIOSK_PACKAGES_TO_SUPPRESS`])
/// via `pm disable-user`, so the TV cannot keep resurfacing them over our stage
/// app (#477). Best-effort: a package that is absent, already disabled, or not
/// disable-able just no-ops (its error is ignored). Idempotent — safe to call on
/// every connect.
pub(super) async fn suppress_kiosk_browsers(runner: &dyn AdbRunner, serial: &str) {
    for pkg in KIOSK_PACKAGES_TO_SUPPRESS {
        let _ = runner
            .run(&adb_args([
                "-s",
                serial,
                "shell",
                "pm",
                "disable-user",
                "--user",
                "0",
                pkg,
            ]))
            .await;
    }
}

/// True when `package` is installed on the device — `pm path <package>` prints a
/// `package:` line. A missing package prints nothing (or errors) → false.
pub(super) async fn adb_package_installed(
    runner: &dyn AdbRunner,
    serial: &str,
    package: &str,
) -> bool {
    let args = adb_args(["-s", serial, "shell", "pm", "path", package]);
    match runner.run(&args).await {
        Ok(output) => String::from_utf8_lossy(&output.stdout).contains("package:"),
        Err(_) => false,
    }
}

/// `adb install` reports failure on stdout (`Failure [INSTALL_FAILED_…]`) and,
/// depending on adb version, may still exit 0 — so require BOTH a success exit
/// AND a `Success` line.
pub(super) fn adb_install_succeeded(output: &Output) -> bool {
    ensure_success(output).is_ok() && String::from_utf8_lossy(&output.stdout).contains("Success")
}

/// Read the `versionCode` of `package` installed on the device via
/// `dumpsys package <pkg>`. Returns `None` when the command fails or no
/// `versionCode=` line is present (e.g. package absent).
pub(super) async fn adb_installed_version_code(
    runner: &dyn AdbRunner,
    serial: &str,
    package: &str,
) -> Option<i64> {
    let args = adb_args(["-s", serial, "shell", "dumpsys", "package", package]);
    let output = runner.run(&args).await.ok()?;
    parse_version_code(&String::from_utf8_lossy(&output.stdout))
}

/// Parse the first `versionCode=<n>` integer out of `dumpsys package` output.
/// `dumpsys` prints e.g. `    versionCode=7 minSdk=22 targetSdk=34` — we take the
/// digits immediately after the first `versionCode=`. Returns `None` when no such
/// field is present. Pure (no I/O) so the parsing is unit-testable.
pub(super) fn parse_version_code(dumpsys: &str) -> Option<i64> {
    let after = dumpsys.split("versionCode=").nth(1)?;
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Query the device's currently-resumed COMPONENT (`<pkg>/<activity>`) via
/// `adb -s <serial> shell dumpsys activity activities`. Returns the resumed
/// component, or None on any adb error/timeout/non-success or when no resumed
/// activity is reported — the caller treats None as "foreground unknown →
/// (re)launch". Read-only: the dumpsys probe never disturbs the running browser.
pub(super) async fn adb_foreground_component(
    runner: &dyn AdbRunner,
    serial: &str,
) -> Option<String> {
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

/// Parse the resumed-activity COMPONENT (`<pkg>/<activity>`) from
/// `dumpsys activity activities` output. Finds the
/// `[m]ResumedActivity: ActivityRecord{<hash> u0 <pkg>/<activity> …}` line and
/// returns `<pkg>/<activity>`. Returns None when no resumed activity is reported
/// (`mResumedActivity: null`) or the line is absent — the caller treats None as
/// "foreground unknown → (re)launch".
///
/// The component (package AND activity) is required by [`super::should_launch_stage`]
/// to tell the loaded stage page (`…BrowsePageActivity`) from the home portal
/// (`…StartActivity`), which share the `com.tcl.browser` package (#447).
pub(super) fn parse_foreground_component(dumpsys_output: &str) -> Option<String> {
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

pub(super) fn ensure_success(output: &Output) -> Result<(), String> {
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

pub(super) fn format_command_failure(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!(
        "status: {} stdout: {} stderr: {}",
        output.status,
        stdout.trim(),
        stderr.trim()
    )
}

/// Ensure our Presenter Stage app is installed + up to date, per
/// [`stage_app_install_action`] (#734): (re)install ONLY when genuinely absent
/// or at a readable LOWER versionCode — an UNREADABLE code leaves it in place
/// (never tear a healthy app down mid-event, the harm this fixes; #732 open).
pub(super) async fn ensure_app_installed(
    runner: &dyn AdbRunner,
    serial: &str,
    package: &str,
    apk: &Path,
) -> anyhow::Result<()> {
    let installed = adb_package_installed(runner, serial, package).await;
    let version_code = if installed {
        adb_installed_version_code(runner, serial, package).await
    } else {
        None
    };

    match stage_app_install_action(installed, version_code, EXPECTED_STAGE_APK_VERSION_CODE) {
        StageAppInstallAction::UpToDate => Ok(()),
        StageAppInstallAction::PresentVersionUnknown => {
            // #734: a failed read is NOT evidence of staleness — never tear down
            // a healthy running app; a real upgrade needs a readable LOWER code.
            warn!(
                serial,
                package,
                "Presenter Stage installed but versionCode unreadable — leaving the \
                 running app in place (#734); not reinstalling"
            );
            Ok(())
        }
        StageAppInstallAction::Upgrade => {
            info!(
                serial,
                package,
                expected = EXPECTED_STAGE_APK_VERSION_CODE,
                "Presenter Stage app is stale — upgrading"
            );
            install_stage_apk(runner, serial, package, apk).await
        }
        StageAppInstallAction::Install => {
            info!(serial, package, apk = %apk.display(), "installing Presenter Stage app on TV");
            install_stage_apk(runner, serial, package, apk).await
        }
    }
}

/// `adb install -r <apk>`; on failure fall back to uninstall + clean install
/// (stateless WebView shell → loses nothing). Called ONLY for a genuine
/// absent/lower-version install (#734). The `install -r` failure OUTPUT is now
/// logged — it fails ~100% on these debug-signed builds (fresh per-run CI debug
/// key → `INSTALL_FAILED_UPDATE_INCOMPATIBLE`) and was previously swallowed.
pub(super) async fn install_stage_apk(
    runner: &dyn AdbRunner,
    serial: &str,
    package: &str,
    apk: &Path,
) -> anyhow::Result<()> {
    let mut install_args = adb_args(["-s", serial, "install", "-r"]);
    install_args.push(apk.as_os_str().to_os_string());
    match runner.run(&install_args).await {
        Ok(output) if adb_install_succeeded(&output) => return Ok(()),
        Ok(output) => {
            warn!(
                serial,
                package,
                adb_output = %format_command_failure(&output),
                "adb install -r failed — retrying with uninstall + install (#734)"
            );
        }
        Err(err) => {
            warn!(
                serial,
                package,
                %err,
                "adb install -r could not run — retrying with uninstall + install (#734)"
            );
        }
    }

    // Reinstall path: drop any conflicting/old copy, then install clean.
    let _ = runner
        .run(&adb_args(["-s", serial, "uninstall", package]))
        .await;
    let mut clean_args = adb_args(["-s", serial, "install"]);
    clean_args.push(apk.as_os_str().to_os_string());
    let output = runner
        .run(&clean_args)
        .await
        .map_err(|e| anyhow!("failed to execute adb install for {serial}: {e}"))?;
    if adb_install_succeeded(&output) {
        Ok(())
    } else {
        Err(anyhow!(
            "adb install failed for {serial}: {}",
            format_command_failure(&output)
        ))
    }
}
