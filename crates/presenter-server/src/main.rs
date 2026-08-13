//! Thin entry point. All application modules live in `lib.rs` (#680 split —
//! see that file's doc comment for why). This file wires startup only.

use anyhow::Context;
use presenter_server::config::ServerConfig;
use presenter_server::router::build_router;
use presenter_server::state::AppState;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    setup_tracing();

    // Initialize GStreamer + register Rust plugins (webrtcsink, webrtchttp, ndisrc).
    // Startup logs loudly on missing pieces but does NOT crash the server —
    // the hard fail-loudly gate lives at pipeline-build time
    // (presenter_ndi::pipeline::NdiPipeline::build returns Err when no HW
    // H264 encoder is registered). That way the server still serves non-NDI
    // features even if encoder drivers are broken on the host.
    if let Err(e) = presenter_ndi::init() {
        tracing::error!("GStreamer init failed: {e:#}. NDI WebRTC disabled.");
    } else {
        match presenter_ndi::hw_h264_encoder() {
            Some(name) => {
                tracing::info!("NDI WebRTC encoder: {name}");
            }
            None => {
                // #540: say WHY there is no encoder. The old message only ever
                // advised installing driver packages — on PP every package was
                // installed and the real cause was that the service could not open
                // /dev/dri/renderD128 (no render-group grant), so the `va` plugin
                // registered nothing. Report the host's actual render-node state.
                let access = presenter_ndi::gpu::render_node_access();
                tracing::warn!(
                    render_node = presenter_ndi::gpu::RENDER_NODE,
                    ?access,
                    "no hardware H264 encoder (vah264enc / nvh264enc) registered — \
                     NDI WebRTC pipeline build will fail at activation. {}",
                    presenter_ndi::gpu::missing_encoder_hint(access)
                );
            }
        }
    }

    let config = ServerConfig::load()?;
    let addr: SocketAddr = SocketAddr::from(([0, 0, 0, 0], config.http.port));
    let state = AppState::from_config(config).await?;
    let app = build_router(state.clone());

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind to {addr}"))?;
    tracing::info!(%addr, "presenter server listening");

    // #423: launch the Android stage displays NOW that the listener is bound, so
    // the on-device `am start` lands on a serving server. Doing this during
    // `from_config` (before bind) raced the listener — the TV hit
    // connection-refused, showed the browser error page, and the #419
    // foreground-aware keep-alive then skipped the relaunch forever. Non-fatal:
    // a launch failure must never stop the server from serving.
    if let Err(err) = state.start_android_stage_displays().await {
        tracing::warn!(?err, "failed to launch android stage displays on startup");
    }
    // Mock integrations (OSC/AbleSet/Resolume) bind FIXED localhost ports
    // (e.g. 127.0.0.1:8091). When a test server is spawned on a host that
    // already runs another mock-integrations build (e.g. the deployed
    // presenter-dev service on the self-hosted CI runner), those ports
    // collide and the second server fails to start. Tests that don't need the
    // mocks (the NDI WebRTC E2E lane) set PRESENTER_SKIP_MOCK_INTEGRATIONS=1
    // to skip them and avoid the conflict.
    #[cfg(feature = "mock-integrations")]
    if std::env::var_os("PRESENTER_SKIP_MOCK_INTEGRATIONS").is_none() {
        presenter_server::mock_integrations::start_all().await?;
    } else {
        tracing::info!("PRESENTER_SKIP_MOCK_INTEGRATIONS set — skipping mock integrations");
    }
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
