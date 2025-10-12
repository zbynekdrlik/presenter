mod ableset;
mod android_stage;
mod companion;
mod config;
mod live;
mod osc;
mod resolume;
mod router;
mod stage_connections;
mod stage_ui;
mod state;
mod ui;

use anyhow::Context;
use config::ServerConfig;
use router::build_router;
use state::AppState;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    setup_tracing();
    let config = ServerConfig::load()?;
    let addr: SocketAddr = SocketAddr::from(([0, 0, 0, 0], config.http.port));
    let state = AppState::from_config(config).await?;
    let app = build_router(state);

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind to {addr}"))?;
    tracing::info!(%addr, "presenter server listening");
    axum::serve(listener, app).await.context("server failure")
}

fn setup_tracing() {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info,tower_http=debug");
    }

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();
}

#[cfg(test)]
mod tests {
    use crate::config::DEFAULT_SERVER_PORT;

    #[test]
    fn default_port_is_number() {
        assert_eq!(DEFAULT_SERVER_PORT, 80);
    }
}
