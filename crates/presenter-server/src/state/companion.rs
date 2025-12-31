use std::sync::Arc;
use tokio::{
    net::TcpListener,
    sync::{oneshot, Mutex},
    task::JoinHandle,
};
use tracing::{debug, error, info};

use super::AppState;

pub(crate) const COMPANION_FEATURE_KEY: &str = "feature.companion.enabled";
pub(crate) const COMPANION_PORT_KEY: &str = "feature.companion.port";
pub(crate) const DEFAULT_COMPANION_PORT: u16 = 18_175;

pub(crate) fn parse_bool_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[derive(Clone, Default)]
pub(crate) struct CompanionServerManager {
    handle: Arc<Mutex<Option<CompanionServerHandle>>>,
}

pub(crate) struct CompanionServerHandle {
    pub(crate) port: u16,
    pub(crate) shutdown: Option<oneshot::Sender<()>>,
    pub(crate) join: JoinHandle<()>,
}

impl CompanionServerManager {
    pub(crate) async fn reconfigure(
        &self,
        state: AppState,
        enabled: bool,
        port: u16,
    ) -> anyhow::Result<()> {
        if !enabled {
            self.stop().await;
            return Ok(());
        }

        {
            let guard = self.handle.lock().await;
            if let Some(existing) = guard.as_ref() {
                if existing.port == port && !existing.join.is_finished() {
                    return Ok(());
                }
            }
        }

        // Attempt to bind before tearing down the current server so we can surface errors without
        // losing the previous listener when switching ports.
        let listener = TcpListener::bind(("0.0.0.0", port)).await.map_err(|err| {
            anyhow::anyhow!("failed to bind Companion websocket on port {port}: {err}")
        })?;

        let mut guard = self.handle.lock().await;
        if let Some(existing) = guard.take() {
            existing.shutdown().await;
        }

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let router = crate::companion::build_router(state.clone());
        let join = tokio::spawn(async move {
            let shutdown = async {
                let _ = shutdown_rx.await;
            };
            let server = axum::serve(listener, router);
            info!(port, "Companion websocket server listening");
            let result = server.with_graceful_shutdown(shutdown).await;
            if let Err(err) = result {
                error!(?err, port, "Companion websocket server exited with error");
            } else {
                debug!(port, "Companion websocket server stopped");
            }
        });

        *guard = Some(CompanionServerHandle {
            port,
            shutdown: Some(shutdown_tx),
            join,
        });

        Ok(())
    }

    pub(crate) async fn stop(&self) {
        let mut guard = self.handle.lock().await;
        if let Some(handle) = guard.take() {
            handle.shutdown().await;
        }
    }
}

impl CompanionServerHandle {
    pub(crate) async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Err(err) = self.join.await {
            error!(
                ?err,
                port = self.port,
                "Companion websocket task join error"
            );
        }
    }
}
