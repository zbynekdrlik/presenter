use crate::id::AndroidStageDisplayId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default launcher target: the TCL built-in browser PACKAGE.
///
/// The launcher fires a `VIEW` intent at this package with the configured
/// `PRESENTER_ANDROID_STAGE_URL` (see `android_stage.rs`). It is a bare package
/// (no `/activity`), which is the proven-working shape on the prod stage TVs.
/// Legacy `package/activity` values are still accepted by `validate()` and the
/// launcher extracts the package from them for backward compatibility.
pub const DEFAULT_LAUNCH_PACKAGE: &str = "com.tcl.browser";
pub const DEFAULT_ADB_PORT: u16 = 5555;

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
    fn default_launch_component_is_bare_browser_package() {
        // The launcher fires a VIEW intent at a browser PACKAGE (not a
        // package/activity component), so the default must be a bare package.
        assert_eq!(DEFAULT_LAUNCH_PACKAGE, "com.tcl.browser");
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
