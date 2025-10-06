mod ableset;
mod android_stage;
mod companion;
mod live;
mod osc;
mod resolume;
mod router;
mod stage_connections;
mod stage_ui;
mod state;
mod ui;

use anyhow::Context;
use router::build_router;
use state::AppState;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_PORT: &str = "80";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    setup_tracing();
    let port = std::env::var("PRESENTER_PORT").unwrap_or_else(|_| DEFAULT_PORT.to_string());
    let port: u16 = port
        .parse()
        .with_context(|| format!("invalid PRESENTER_PORT value: {port}"))?;

    let addr: SocketAddr = SocketAddr::from(([0, 0, 0, 0], port));
    let state = AppState::from_env().await?;
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
    use super::*;

    #[test]
    fn default_port_is_number() {
        let port: u16 = DEFAULT_PORT.parse().unwrap();
        assert_eq!(port, 80);
    }
}
