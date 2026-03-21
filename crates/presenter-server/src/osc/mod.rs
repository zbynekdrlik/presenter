mod handler;
mod parser;

use std::{future::Future, pin::Pin, sync::Arc};

use crate::{config::OscConfig, state::AppState};
use chrono::{DateTime, Utc};
use presenter_core::{OscSettings, VelocityMode};
use tokio::sync::oneshot;
use tokio::{
    net::UdpSocket,
    sync::{Mutex, RwLock},
    task::JoinHandle,
};
use tracing::debug;

use handler::{run_listener, ListenerConfig};

#[derive(Clone)]
pub struct OscBridge {
    inner: Arc<OscBridgeInner>,
}

#[allow(dead_code)] // Trait abstraction for test mocking
type OscFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[allow(dead_code)] // Trait abstraction for test mocking
pub trait OscClient: Send + Sync {
    fn apply_settings(
        &self,
        settings: OscSettings,
        app_state: AppState,
    ) -> OscFuture<'_, anyhow::Result<()>>;
    fn status(&self) -> OscFuture<'_, OscStatusSnapshot>;
}

#[allow(dead_code)] // Trait abstraction for test mocking
pub type DynOscClient = Arc<dyn OscClient>;

struct OscBridgeInner {
    status: RwLock<OscStatusInner>,
    listener: Mutex<Option<ListenerGuard>>,
    host_port_override: Option<u16>,
}

struct ListenerGuard {
    shutdown: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

#[derive(Debug, Clone)]
struct OscStatusInner {
    enabled: bool,
    listening: bool,
    listen_port: u16,
    host_port: Option<u16>,
    address_pattern: String,
    velocity_mode: VelocityMode,
    last_message_at: Option<DateTime<Utc>>,
    last_note: Option<u8>,
    last_velocity: Option<u8>,
    last_error: Option<String>,
}

impl Default for OscStatusInner {
    fn default() -> Self {
        Self {
            enabled: false,
            listening: false,
            listen_port: 39051,
            host_port: None,
            address_pattern: "/note".to_string(),
            velocity_mode: VelocityMode::OneBased,
            last_message_at: None,
            last_note: None,
            last_velocity: None,
            last_error: None,
        }
    }
}

impl OscStatusInner {
    fn with_host_override(host_port: Option<u16>) -> Self {
        let mut status = Self::default();
        status.host_port = host_port;
        status
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OscStatusSnapshot {
    pub enabled: bool,
    pub listening: bool,
    pub listen_port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_port: Option<u16>,
    pub address_pattern: String,
    pub velocity_mode: VelocityMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_note: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_velocity: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl From<&OscStatusInner> for OscStatusSnapshot {
    fn from(inner: &OscStatusInner) -> Self {
        Self {
            enabled: inner.enabled,
            listening: inner.listening,
            listen_port: inner.listen_port,
            host_port: inner.host_port,
            address_pattern: inner.address_pattern.clone(),
            velocity_mode: inner.velocity_mode,
            last_message_at: inner.last_message_at,
            last_note: inner.last_note,
            last_velocity: inner.last_velocity,
            last_error: inner.last_error.clone(),
        }
    }
}

impl OscBridge {
    pub fn new(config: &OscConfig) -> Self {
        Self {
            inner: Arc::new(OscBridgeInner {
                status: RwLock::new(OscStatusInner::with_host_override(config.host_port)),
                listener: Mutex::new(None),
                host_port_override: config.host_port,
            }),
        }
    }

    pub async fn apply_settings(
        &self,
        settings: OscSettings,
        app_state: AppState,
    ) -> anyhow::Result<()> {
        {
            let mut status = self.inner.status.write().await;
            status.enabled = settings.enabled;
            status.listen_port = settings.listen_port;
            status.host_port = self.inner.host_port_override;
            status.address_pattern = settings.address_pattern.clone();
            status.velocity_mode = settings.velocity_mode;
            status.last_error = None;
        }

        self.stop_listener().await;

        if !settings.enabled {
            return Ok(());
        }

        match self.start_listener(settings, app_state).await {
            Ok(()) => Ok(()),
            Err(err) => {
                let mut status = self.inner.status.write().await;
                status.listening = false;
                status.last_error = Some(err.to_string());
                Err(err)
            }
        }
    }

    pub async fn status(&self) -> OscStatusSnapshot {
        let status = self.inner.status.read().await;
        OscStatusSnapshot::from(&*status)
    }

    async fn start_listener(
        &self,
        settings: OscSettings,
        app_state: AppState,
    ) -> anyhow::Result<()> {
        use anyhow::Context;

        let socket = UdpSocket::bind(("0.0.0.0", settings.listen_port))
            .await
            .with_context(|| {
                format!("failed to bind OSC listener port {}", settings.listen_port)
            })?;
        socket
            .set_broadcast(true)
            .with_context(|| "failed to enable broadcast for OSC listener")?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let inner = self.inner.clone();
        let config = ListenerConfig::from_settings(&settings);
        let handle = tokio::spawn(async move {
            run_listener(socket, shutdown_rx, app_state, inner.clone(), config).await;
        });

        {
            let mut guard = self.inner.listener.lock().await;
            *guard = Some(ListenerGuard {
                shutdown: shutdown_tx,
                handle,
            });
        }

        let mut status = self.inner.status.write().await;
        status.listening = true;
        status.last_error = None;
        Ok(())
    }

    async fn stop_listener(&self) {
        let mut guard = self.inner.listener.lock().await;
        if let Some(listener) = guard.take() {
            if listener.shutdown.send(()).is_err() {
                debug!("osc listener already stopped");
            }
            if let Err(err) = listener.handle.await {
                debug!(?err, "osc listener join failed");
            }
        }
        let mut status = self.inner.status.write().await;
        status.listening = false;
    }
}

impl OscClient for OscBridge {
    fn apply_settings(
        &self,
        settings: OscSettings,
        app_state: AppState,
    ) -> OscFuture<'_, anyhow::Result<()>> {
        let bridge = self.clone();
        Box::pin(async move { OscBridge::apply_settings(&bridge, settings, app_state).await })
    }

    fn status(&self) -> OscFuture<'_, OscStatusSnapshot> {
        let bridge = self.clone();
        Box::pin(async move { OscBridge::status(&bridge).await })
    }
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Clone, Default)]
pub struct MockOscClient {
    state: Arc<Mutex<MockOscState>>,
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug)]
struct MockOscState {
    settings: Option<OscSettings>,
    status: OscStatusSnapshot,
}

#[cfg(test)]
impl Default for MockOscState {
    fn default() -> Self {
        Self {
            settings: None,
            status: OscStatusSnapshot {
                enabled: false,
                listening: false,
                listen_port: 39051,
                host_port: None,
                address_pattern: "/note".to_string(),
                velocity_mode: VelocityMode::OneBased,
                last_message_at: None,
                last_note: None,
                last_velocity: None,
                last_error: None,
            },
        }
    }
}

#[cfg(test)]
#[allow(dead_code)]
impl MockOscClient {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
impl OscClient for MockOscClient {
    fn apply_settings(
        &self,
        settings: OscSettings,
        _app_state: AppState,
    ) -> OscFuture<'_, anyhow::Result<()>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut guard = state.lock().await;
            guard.status.enabled = settings.enabled;
            guard.status.listen_port = settings.listen_port;
            guard.status.address_pattern = settings.address_pattern.clone();
            guard.status.velocity_mode = settings.velocity_mode;
            guard.status.listening = settings.enabled;
            guard.status.last_error = None;
            guard.settings = Some(settings);
            Ok(())
        })
    }

    fn status(&self) -> OscFuture<'_, OscStatusSnapshot> {
        let state = self.state.clone();
        Box::pin(async move { state.lock().await.status.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::handler::{compute_index, store_partial_event, ListenerConfig};
    use super::parser::{extract_note_velocity, Extracted};
    use presenter_core::VelocityMode;
    use rosc::{OscMessage, OscType};
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::Mutex;

    #[test]
    fn extract_two_arguments_returns_note_and_velocity() {
        let config = ListenerConfig {
            address_pattern: "/note".to_string(),
            velocity_mode: VelocityMode::OneBased,
        };
        let message = OscMessage {
            addr: "/note".to_string(),
            args: vec![OscType::Int(60), OscType::Int(90)],
        };
        match extract_note_velocity(&message, &config) {
            Extracted::Complete(60, 90) => {}
            other => panic!("unexpected extraction result: {:?}", other),
        }
    }

    #[test]
    fn extract_three_arguments_uses_last_two_values() {
        let config = ListenerConfig {
            address_pattern: "/note".to_string(),
            velocity_mode: VelocityMode::OneBased,
        };
        let message = OscMessage {
            addr: "/note".to_string(),
            args: vec![OscType::Int(1), OscType::Int(62), OscType::Int(45)],
        };
        match extract_note_velocity(&message, &config) {
            Extracted::Complete(62, 45) => {}
            other => panic!("unexpected extraction result: {:?}", other),
        }
    }

    #[test]
    fn compute_index_zero_based_mapping() {
        assert_eq!(compute_index(0, VelocityMode::ZeroBased), Some(0));
        assert_eq!(compute_index(5, VelocityMode::ZeroBased), Some(5));
    }

    #[test]
    fn compute_index_one_based_mapping() {
        assert_eq!(compute_index(0, VelocityMode::OneBased), None);
        assert_eq!(compute_index(1, VelocityMode::OneBased), Some(0));
        assert_eq!(compute_index(7, VelocityMode::OneBased), Some(6));
    }

    #[tokio::test]
    async fn split_messages_merge_into_single_event() {
        let config = ListenerConfig {
            address_pattern: "/note".to_string(),
            velocity_mode: VelocityMode::OneBased,
        };
        let accumulator = Arc::new(Mutex::new(HashMap::new()));

        let note_message = OscMessage {
            addr: "/Note1".to_string(),
            args: vec![OscType::Int(19)],
        };
        if let Extracted::Pending {
            channel,
            note,
            velocity,
        } = extract_note_velocity(&note_message, &config)
        {
            assert_eq!(note, Some(19));
            assert!(velocity.is_none());
            assert!(store_partial_event(&accumulator, channel, note, velocity)
                .await
                .is_none());
        } else {
            panic!("expected pending note extraction");
        }

        let velocity_message = OscMessage {
            addr: "/Velocity1".to_string(),
            args: vec![OscType::Int(7)],
        };
        if let Extracted::Pending {
            channel,
            note,
            velocity,
        } = extract_note_velocity(&velocity_message, &config)
        {
            assert!(note.is_none());
            assert_eq!(velocity, Some(7));
            let combined = store_partial_event(&accumulator, channel, note, velocity).await;
            assert_eq!(combined, Some((19, 7)));
        } else {
            panic!("expected pending velocity extraction");
        }
    }
}
