//! `GET /listen/ws` (#852 C3-6, spec §4, plan Decision 8): the guest's live
//! transcript. On connect ONE `snapshot`, then every `sentence` / `tail` /
//! `session` message of the [`TranscriptHub`]. A reader that fell
//! [`HUB_CAPACITY`](crate::listen::transcript_hub::HUB_CAPACITY) messages
//! behind gets a fresh `snapshot` (a resync, INFO). What the client sends is
//! read only to notice the close. Connect / disconnect / resync log on
//! `presenter::listen::ws` with the client IP of the #793 rule.

use std::net::SocketAddr;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Extension;
use presenter_core::listen::ws::{ListenWsMessage, TranscriptSnapshot};
use tokio::sync::broadcast::error::RecvError;

use super::integrations::extract_client_ip;
use crate::listen::transcript_hub::TranscriptHub;
use crate::state::AppState;

/// The target of the reader lines (plan Decision 16).
pub(crate) const LISTEN_WS_TARGET: &str = "presenter::listen::ws";

/// Why a reader's socket ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndReason {
    ClientClosed,
    ReadFailed,
    WriteFailed,
    HubClosed,
}

impl EndReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::ClientClosed => "client closed",
            Self::ReadFailed => "read failed",
            Self::WriteFailed => "write failed",
            Self::HubClosed => "hub closed",
        }
    }
}

pub(super) async fn listen_websocket(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let peer = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let client_ip = extract_client_ip(&headers, peer);
    let hub = state.listen_transcripts();
    ws.on_upgrade(move |socket| serve_reader(hub, socket, client_ip, peer))
}

async fn serve_reader(
    hub: TranscriptHub,
    mut socket: WebSocket,
    client_ip: String,
    peer: Option<SocketAddr>,
) {
    let peer = peer.map_or_else(|| "-".to_string(), |addr| addr.to_string());
    tracing::info!(
        target: LISTEN_WS_TARGET,
        client_ip = client_ip.as_str(),
        peer = peer.as_str(),
        "listen ws reader connected"
    );
    let connected_at = Instant::now();
    let mut resyncs: u64 = 0;
    let reason = forward(&hub, &mut socket, &client_ip, &mut resyncs).await;
    tracing::info!(
        target: LISTEN_WS_TARGET,
        client_ip = client_ip.as_str(),
        peer = peer.as_str(),
        reason = reason.as_str(),
        resyncs,
        connected_ms = u64::try_from(connected_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        "listen ws reader disconnected"
    );
}

/// Send the snapshot, then every hub message, until either side ends.
async fn forward(
    hub: &TranscriptHub,
    socket: &mut WebSocket,
    client_ip: &str,
    resyncs: &mut u64,
) -> EndReason {
    let (snapshot, mut messages) = hub.subscribe();
    if send_snapshot(socket, snapshot).await.is_err() {
        return EndReason::WriteFailed;
    }
    loop {
        tokio::select! {
            message = messages.recv() => match message {
                Ok(message) => {
                    if send(socket, &message).await.is_err() {
                        return EndReason::WriteFailed;
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    let (snapshot, fresh) = hub.subscribe();
                    messages = fresh;
                    *resyncs += 1;
                    tracing::info!(
                        target: LISTEN_WS_TARGET,
                        client_ip,
                        missed,
                        resyncs = *resyncs,
                        "listen ws reader fell behind — sending a fresh snapshot"
                    );
                    if send_snapshot(socket, snapshot).await.is_err() {
                        return EndReason::WriteFailed;
                    }
                }
                Err(RecvError::Closed) => return EndReason::HubClosed,
            },
            incoming = socket.recv() => match incoming {
                None | Some(Ok(Message::Close(_))) => return EndReason::ClientClosed,
                Some(Err(_)) => return EndReason::ReadFailed,
                Some(Ok(_)) => {}
            },
        }
    }
}

async fn send_snapshot(socket: &mut WebSocket, snapshot: TranscriptSnapshot) -> Result<(), ()> {
    send(socket, &ListenWsMessage::Snapshot { snapshot }).await
}

async fn send(socket: &mut WebSocket, message: &ListenWsMessage) -> Result<(), ()> {
    let json = serde_json::to_string(message).map_err(|_| ())?;
    socket
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| ())
}

#[cfg(test)]
mod tests;
