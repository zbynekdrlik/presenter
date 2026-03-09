use crate::stage_connections::StageHeartbeatConfig;
use anyhow::{Context, Result};
use std::{env, ffi::OsString, time::Duration};

pub const DEFAULT_SERVER_PORT: u16 = 80;
const DEFAULT_DATABASE_URL: &str = "sqlite://presenter_dev.db";

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub http: HttpConfig,
    pub database: DatabaseConfig,
    pub companion: CompanionConfig,
    pub osc: OscConfig,
    pub stage: StageConfig,
    #[allow(dead_code)]
    pub android: AndroidConfig,
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, Default)]
pub struct CompanionConfig {
    pub token: Option<String>,
    pub enabled_override: Option<bool>,
    pub port_override: Option<u16>,
}

#[derive(Debug, Clone, Default)]
pub struct OscConfig {
    #[allow(dead_code)]
    pub listen_port_override: Option<u16>,
    #[allow(dead_code)]
    pub listen_port_invalid: Option<String>,
    pub host_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct StageConfig {
    pub heartbeat: StageHeartbeatConfig,
}

#[derive(Debug, Clone, Default)]
pub struct AndroidConfig {
    #[allow(dead_code)]
    pub adb_path: Option<OsString>,
}

impl ServerConfig {
    pub fn load() -> Result<Self> {
        Ok(Self {
            http: HttpConfig::load()?,
            database: DatabaseConfig::load(),
            companion: CompanionConfig::load(),
            osc: OscConfig::load(),
            stage: StageConfig::load(),
            android: AndroidConfig::load(),
        })
    }
}

impl HttpConfig {
    fn load() -> Result<Self> {
        let raw = env::var("PRESENTER_PORT").unwrap_or_else(|_| DEFAULT_SERVER_PORT.to_string());
        let port = raw
            .parse::<u16>()
            .with_context(|| format!("invalid PRESENTER_PORT value: {raw}"))?;
        Ok(Self { port })
    }
}

impl DatabaseConfig {
    fn load() -> Self {
        let url = env::var("PRESENTER_DB_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
        Self { url }
    }
}

impl CompanionConfig {
    fn load() -> Self {
        let token = env::var("PRESENTER_COMPANION_TOKEN").ok();
        let enabled_override = env::var("PRESENTER_COMPANION_ENABLED")
            .ok()
            .as_deref()
            .and_then(parse_bool_flag);
        let port_override = env::var("PRESENTER_COMPANION_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port > 0);
        Self {
            token,
            enabled_override,
            port_override,
        }
    }
}

impl OscConfig {
    fn load() -> Self {
        let (listen_port_override, listen_port_invalid) =
            parse_listen_port_override(env::var("PRESENTER_OSC_LISTEN_PORT").ok());
        if let Some(ref invalid) = listen_port_invalid {
            tracing::warn!(
                value = %invalid,
                "ignoring invalid PRESENTER_OSC_LISTEN_PORT value; expected a positive u16"
            );
        }
        let host_port = env::var("PRESENTER_OSC_HOST_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok());
        Self {
            listen_port_override,
            listen_port_invalid,
            host_port,
        }
    }
}

impl StageConfig {
    fn load() -> Self {
        let defaults = StageHeartbeatConfig::default_values();
        let interval =
            duration_override("PRESENTER_HEARTBEAT_INTERVAL_MS").unwrap_or(defaults.interval);
        let grace = duration_override("PRESENTER_HEARTBEAT_GRACE_MS").unwrap_or(defaults.grace);
        let disconnect_after = duration_override("PRESENTER_HEARTBEAT_DISCONNECT_MS")
            .unwrap_or(defaults.disconnect_after);
        let heartbeat = StageHeartbeatConfig::new(interval, grace, disconnect_after);
        Self { heartbeat }
    }
}

impl AndroidConfig {
    fn load() -> Self {
        let adb_path = env::var_os("PRESENTER_ANDROID_ADB_BIN");
        Self { adb_path }
    }
}

fn duration_override(var: &str) -> Option<Duration> {
    env::var(var)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
}

fn parse_listen_port_override(raw: Option<String>) -> (Option<u16>, Option<String>) {
    match raw {
        Some(value) => match value.parse::<u16>() {
            Ok(port) if port > 0 => (Some(port), None),
            _ => (None, Some(value)),
        },
        None => (None, None),
    }
}

pub fn parse_bool_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bool_flag_handles_common_truthy_and_falsy_values() {
        assert_eq!(parse_bool_flag("true"), Some(true));
        assert_eq!(parse_bool_flag("1"), Some(true));
        assert_eq!(parse_bool_flag(" yes "), Some(true));
        assert_eq!(parse_bool_flag("false"), Some(false));
        assert_eq!(parse_bool_flag("0"), Some(false));
        assert_eq!(parse_bool_flag("off"), Some(false));
        assert_eq!(parse_bool_flag("maybe"), None);
    }

    #[test]
    fn parse_listen_port_override_accepts_positive_ports() {
        assert_eq!(
            parse_listen_port_override(Some("1200".to_string())),
            (Some(1200), None)
        );
        assert_eq!(
            parse_listen_port_override(Some("0".to_string())),
            (None, Some("0".to_string()))
        );
        assert_eq!(
            parse_listen_port_override(Some("bad".to_string())),
            (None, Some("bad".to_string()))
        );
        assert_eq!(parse_listen_port_override(None), (None, None));
    }

    #[test]
    fn duration_override_rejects_zero_and_invalid_values() {
        let original = env::var("PRESENTER_HEARTBEAT_INTERVAL_MS").ok();
        env::set_var("PRESENTER_HEARTBEAT_INTERVAL_MS", "1500");
        assert_eq!(
            duration_override("PRESENTER_HEARTBEAT_INTERVAL_MS"),
            Some(Duration::from_millis(1500))
        );
        env::set_var("PRESENTER_HEARTBEAT_INTERVAL_MS", "0");
        assert_eq!(
            duration_override("PRESENTER_HEARTBEAT_INTERVAL_MS"),
            Some(Duration::from_millis(0))
        );
        env::set_var("PRESENTER_HEARTBEAT_INTERVAL_MS", "not-a-number");
        assert_eq!(duration_override("PRESENTER_HEARTBEAT_INTERVAL_MS"), None);
        match original {
            Some(value) => env::set_var("PRESENTER_HEARTBEAT_INTERVAL_MS", value),
            None => env::remove_var("PRESENTER_HEARTBEAT_INTERVAL_MS"),
        }
    }
}
