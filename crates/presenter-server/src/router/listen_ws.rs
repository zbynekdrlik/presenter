//! `GET /listen/ws` (#852 C3-6, spec §4, plan Decision 8): the guest's live
//! transcript. On connect ONE `snapshot`, then every `sentence` / `tail` /
//! `session` message of the [`TranscriptHub`]. A reader that fell
//! [`HUB_CAPACITY`](crate::listen::transcript_hub::HUB_CAPACITY) messages
//! behind gets a fresh `snapshot` (a resync, INFO). What the client sends is
//! read only to notice the close and that it is alive. Outside a sermon the hub
//! sends nothing, so a WS Ping goes out every [`KEEPALIVE`] (under Cloudflare's
//! 100 s idle cut); a client that sent no frame — not even the automatic
//! pong — for two keepalive intervals is a half-open socket and is dropped. Connect /
//! disconnect / resync log on `presenter::listen::ws` with the client IP of
//! the #793 rule and the end reason of `/live/ws` ([`WsEndReason`]).

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Extension;
use presenter_core::listen::ws::{ListenWsMessage, TranscriptSnapshot};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{Instant, MissedTickBehavior};

use super::integrations::extract_client_ip;
use crate::listen::transcript_hub::TranscriptHub;
use crate::live::{close_frame_reason, WsEndReason};
use crate::state::AppState;

/// The target of the reader lines (plan Decision 16).
pub(crate) const LISTEN_WS_TARGET: &str = "presenter::listen::ws";
/// The production ping interval: a ping and the two-interval silence cut
/// both stay under Cloudflare's ~100 s idle cut (checked at compile time).
const KEEPALIVE_PRODUCTION: Duration = Duration::from_secs(30);
const _: () = assert!(KEEPALIVE_PRODUCTION.as_secs() * 2 < 100);
/// A Ping this often; no frame from the client for two intervals ends the socket.
#[cfg(not(test))]
pub(crate) const KEEPALIVE: Duration = KEEPALIVE_PRODUCTION;
#[cfg(test)]
pub(crate) const KEEPALIVE: Duration = Duration::from_secs(1);

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
        reason = reason.to_string().as_str(),
        resyncs,
        connected_ms = u64::try_from(connected_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        "listen ws reader disconnected"
    );
}

/// Send the snapshot, then every hub message and a ping every [`KEEPALIVE`],
/// until either side ends.
async fn forward(
    hub: &TranscriptHub,
    socket: &mut WebSocket,
    client_ip: &str,
    resyncs: &mut u64,
) -> WsEndReason {
    let (snapshot, mut messages) = hub.subscribe();
    if let Err(reason) = send_snapshot(socket, snapshot).await {
        return reason;
    }
    // Set before the interval's start, so the 2nd tick is ≥ 2 intervals after it.
    let mut last_heard = Instant::now();
    let mut keepalive = tokio::time::interval_at(last_heard + KEEPALIVE, KEEPALIVE);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            message = messages.recv() => match message {
                Ok(message) => {
                    if let Err(reason) = send(socket, &message).await {
                        return reason;
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
                    if let Err(reason) = send_snapshot(socket, snapshot).await {
                        return reason;
                    }
                }
                Err(RecvError::Closed) => return WsEndReason::HubClosed,
            },
            incoming = socket.recv() => match incoming {
                None => return WsEndReason::StreamEnded,
                Some(Err(err)) => return WsEndReason::ReadError(err.to_string()),
                Some(Ok(Message::Close(frame))) => return close_frame_reason(frame),
                Some(Ok(_)) => last_heard = Instant::now(),
            },
            _ = keepalive.tick() => {
                if last_heard.elapsed() >= 2 * KEEPALIVE {
                    return WsEndReason::ClientSilent;
                }
                if let Err(err) = socket.send(Message::Ping(Default::default())).await {
                    return WsEndReason::SendFailed(err.to_string());
                }
            }
        }
    }
}

async fn send_snapshot(
    socket: &mut WebSocket,
    snapshot: TranscriptSnapshot,
) -> Result<(), WsEndReason> {
    send(socket, &ListenWsMessage::Snapshot { snapshot }).await
}

async fn send(socket: &mut WebSocket, message: &ListenWsMessage) -> Result<(), WsEndReason> {
    let json = serde_json::to_string(message).map_err(|err| {
        tracing::warn!(target: LISTEN_WS_TARGET, %err, "failed to serialise a listen ws message");
        WsEndReason::SendFailed(format!("encode: {err}"))
    })?;
    socket
        .send(Message::Text(json.into()))
        .await
        .map_err(|err| WsEndReason::SendFailed(err.to_string()))
}

#[cfg(test)]
mod tests;
