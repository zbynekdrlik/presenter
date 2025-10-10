use std::sync::{atomic::Ordering, Arc};

use tokio::{
    net::TcpListener,
    sync::{oneshot, Mutex},
    task::JoinHandle,
};
use tracing::{debug, error, info};

use super::{AppState, COMPANION_FEATURE_KEY, COMPANION_PORT_KEY};

#[derive(Clone, Default)]
pub(crate) struct CompanionServerManager {
    handle: Arc<Mutex<Option<CompanionServerHandle>>>,
}

struct CompanionServerHandle {
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
}

impl CompanionServerManager {
    pub(super) async fn reconfigure(
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

    async fn stop(&self) {
        let mut guard = self.handle.lock().await;
        if let Some(handle) = guard.take() {
            handle.shutdown().await;
        }
    }
}

impl CompanionServerHandle {
    async fn shutdown(mut self) {
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

impl AppState {
    pub async fn configure_companion_service(
        &self,
        enabled: bool,
        port: u16,
    ) -> anyhow::Result<()> {
        self.companion_server
            .reconfigure(self.clone(), enabled, port)
            .await
    }

    pub async fn set_companion_settings(&self, enabled: bool, port: u16) -> anyhow::Result<()> {
        if port == 0 {
            return Err(anyhow::anyhow!(
                "companion port must be between 1 and 65535"
            ));
        }
        let previous_enabled = self.companion_enabled();
        let previous_port = self.companion_port();

        self.configure_companion_service(enabled, port).await?;

        if let Err(err) = self
            .repository
            .set_app_setting(COMPANION_PORT_KEY, &port.to_string())
            .await
        {
            let _ = self
                .configure_companion_service(previous_enabled, previous_port)
                .await;
            return Err(err);
        }

        if let Err(err) = self
            .repository
            .set_app_setting(COMPANION_FEATURE_KEY, if enabled { "1" } else { "0" })
            .await
        {
            let _ = self
                .repository
                .set_app_setting(COMPANION_PORT_KEY, &previous_port.to_string())
                .await;
            let _ = self
                .configure_companion_service(previous_enabled, previous_port)
                .await;
            return Err(err);
        }

        self.companion_enabled.store(enabled, Ordering::SeqCst);
        self.companion_port.store(port, Ordering::SeqCst);
        Ok(())
    }
}
