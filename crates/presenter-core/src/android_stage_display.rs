use crate::id::AndroidStageDisplayId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default launcher target: our own Presenter Stage app PACKAGE.
///
/// The launcher fires a `VIEW` intent at this package with the configured
/// `PRESENTER_ANDROID_STAGE_URL` (see `android_stage.rs`). When the package is
/// our own app, the watchdog also auto-installs it via ADB if missing — so the
/// stage runs on ANY Android TV without a kiosk browser and without depending on
/// a per-brand browser (e.g. `com.tcl.browser`, absent on Sharp/MediaTek TVs).
/// It is a bare package (no `/activity`). Legacy `package/activity` values are
/// still accepted by `validate()` and the launcher extracts the package from
/// them for backward compatibility.
pub const DEFAULT_LAUNCH_PACKAGE: &str = "sk.newlevel.presenterstage";
pub const DEFAULT_ADB_PORT: u16 = 5555;

/// What the Android stage watchdog should do about the Presenter Stage app on a
/// TV, derived PURELY from whether the package is installed and its readable
/// versionCode (#734). Kept in core (no I/O) so the decision is unit-tested
/// without an adb runner, and so the watchdog file stays under its size cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageAppInstallAction {
    /// App absent → install it (a fresh install; nothing running to disturb).
    Install,
    /// App present at a versionCode below `expected` → upgrade it in place.
    Upgrade,
    /// App present and current (versionCode >= `expected`) → nothing to do.
    UpToDate,
    /// App present but its versionCode could not be read (a transient adb blip /
    /// truncated `dumpsys` — NOT evidence of staleness). Leave the running app in
    /// place; NEVER reinstall on a false-negative read — reinstalling tears the
    /// running app down mid-event (the #734 harm feeding the grey-play-arrow
    /// surface; #732 remains open).
    PresentVersionUnknown,
}

/// Decide the watchdog's install action from the observed device state (#734).
/// `installed` = the package is present (`pm path` returned a `package:` line);
/// `version_code` = the readable installed versionCode, or `None` when the
/// `dumpsys` read failed or had no parseable `versionCode=`. The watchdog only
/// (re)installs when the app is genuinely absent or at a readable LOWER
/// versionCode — never as a routine keep-alive step and never on an unreadable
/// read.
pub fn stage_app_install_action(
    installed: bool,
    version_code: Option<i64>,
    expected: i64,
) -> StageAppInstallAction {
    if !installed {
        return StageAppInstallAction::Install;
    }
    match version_code {
        Some(v) if v >= expected => StageAppInstallAction::UpToDate,
        Some(_) => StageAppInstallAction::Upgrade,
        None => StageAppInstallAction::PresentVersionUnknown,
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AndroidStageDisplayValidationError {
    #[error("label cannot be empty")]
    EmptyLabel,
    #[error("host cannot be empty")]
    EmptyHost,
    #[error("host contains invalid characters (only alphanumeric, dots, and hyphens allowed)")]
    InvalidHostCharacters,
    #[error("port must be between 1 and 65535")]
    InvalidPort,
    #[error("launch component cannot be empty")]
    EmptyLaunchComponent,
    #[error(
        "launch component must be a package name (or 'package/activity') with valid characters"
    )]
    InvalidLaunchComponent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidStageDisplay {
    pub id: AndroidStageDisplayId,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub launch_component: String,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AndroidStageDisplay {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AndroidStageDisplayId,
        label: String,
        host: String,
        port: u16,
        launch_component: String,
        is_enabled: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            label,
            host,
            port,
            launch_component,
            is_enabled,
            created_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidStageDisplayDraft {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub launch_component: String,
    pub is_enabled: bool,
}

impl AndroidStageDisplayDraft {
    pub fn new(label: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            host: host.into(),
            port: DEFAULT_ADB_PORT,
            launch_component: DEFAULT_LAUNCH_PACKAGE.to_string(),
            is_enabled: true,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn with_launch_component(mut self, launch_component: impl Into<String>) -> Self {
        self.launch_component = launch_component.into();
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.is_enabled = enabled;
        self
    }

    pub fn validate(&self) -> Result<(), AndroidStageDisplayValidationError> {
        if self.label.trim().is_empty() {
            return Err(AndroidStageDisplayValidationError::EmptyLabel);
        }
        let host = self.host.trim();
        if host.is_empty() {
            return Err(AndroidStageDisplayValidationError::EmptyHost);
        }
        // Host must only contain alphanumeric, dots, and hyphens (no shell metacharacters)
        if !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            return Err(AndroidStageDisplayValidationError::InvalidHostCharacters);
        }
        if self.port == 0 {
            return Err(AndroidStageDisplayValidationError::InvalidPort);
        }
        let component = self.launch_component.trim();
        if component.is_empty() {
            return Err(AndroidStageDisplayValidationError::EmptyLaunchComponent);
        }
        // Launch component is a package name (e.g. "com.tcl.browser"), or a
        // legacy "package/activity" component. Only Android identifier chars
        // are allowed so the value is safe to splice into an `adb shell am`
        // command: alphanumeric, dots, underscores, slashes (legacy
        // package/activity separator), dollar signs (inner classes).
        if !component
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '/' || c == '$')
        {
            return Err(AndroidStageDisplayValidationError::InvalidLaunchComponent);
        }
        Ok(())
    }
}

impl Default for AndroidStageDisplayDraft {
    fn default() -> Self {
        Self::new("Stage Display", "sd1l.lan")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_action_absent_installs() {
        // Absent → install (fresh; nothing running to tear down). versionCode is
        // irrelevant when the package is not present.
        assert_eq!(
            stage_app_install_action(false, None, 1),
            StageAppInstallAction::Install
        );
        assert_eq!(
            stage_app_install_action(false, Some(1), 1),
            StageAppInstallAction::Install
        );
    }

    #[test]
    fn install_action_current_is_up_to_date() {
        assert_eq!(
            stage_app_install_action(true, Some(1), 1),
            StageAppInstallAction::UpToDate
        );
        assert_eq!(
            stage_app_install_action(true, Some(2), 1),
            StageAppInstallAction::UpToDate
        );
    }

    #[test]
    fn install_action_stale_upgrades() {
        assert_eq!(
            stage_app_install_action(true, Some(0), 1),
            StageAppInstallAction::Upgrade
        );
    }

    #[test]
    fn install_action_present_but_unreadable_version_is_left_in_place() {
        // #734 regression: a present app whose versionCode read failed (a
        // transient adb blip / truncated dumpsys — NOT staleness) must NOT be
        // reinstalled. The old "reinstall to be safe" default tore down a
        // healthy running app mid-event (the grey-play-arrow surface; #732
        // remains open). Present + unknown version = leave it running.
        assert_eq!(
            stage_app_install_action(true, None, 1),
            StageAppInstallAction::PresentVersionUnknown
        );
    }

    #[test]
    fn default_launch_component_is_bare_app_package() {
        // The launcher fires a VIEW intent at a bare PACKAGE (not a
        // package/activity component); the default is our own Presenter Stage app
        // so the stage runs on ANY Android TV without a kiosk/per-brand browser.
        assert_eq!(DEFAULT_LAUNCH_PACKAGE, "sk.newlevel.presenterstage");
    }

    #[test]
    fn validate_accepts_bare_package_name() {
        // A bare package (no "/") is the new valid shape for the VIEW-intent
        // launcher and MUST pass validation.
        let draft = AndroidStageDisplayDraft::new("Stage", "sd1l.lan")
            .with_launch_component("com.tcl.browser");
        assert_eq!(draft.validate(), Ok(()));
    }

    #[test]
    fn validate_still_accepts_legacy_package_activity() {
        // Backward compat: existing "package/activity" values stay valid.
        let draft = AndroidStageDisplayDraft::new("Stage", "sd1l.lan")
            .with_launch_component("com.example/.Main");
        assert_eq!(draft.validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_empty_launch_component() {
        let draft = AndroidStageDisplayDraft::new("Stage", "sd1l.lan").with_launch_component("   ");
        assert_eq!(
            draft.validate(),
            Err(AndroidStageDisplayValidationError::EmptyLaunchComponent)
        );
    }

    #[test]
    fn validate_rejects_shell_metacharacters_in_package() {
        let draft = AndroidStageDisplayDraft::new("Stage", "sd1l.lan")
            .with_launch_component("com.tcl.browser; rm -rf /");
        assert_eq!(
            draft.validate(),
            Err(AndroidStageDisplayValidationError::InvalidLaunchComponent)
        );
    }
}
