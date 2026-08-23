use crate::stage_connections::{DiagRecord, StageConnections};
use axum::extract::ws::{Message, WebSocket};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use presenter_core::NdiVideoDiag;
pub use presenter_core::{InboundMessage, LiveEvent};
use tokio::{sync::broadcast, task::JoinHandle};
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct LiveHub {
    tx: broadcast::Sender<LiveEvent>,
}

impl LiveHub {
    pub fn new() -> Self {
        // Buffer sized for high-activity live events (timers, stage updates, integrations)
        // Prevents event drops during peak broadcast periods
        let (tx, _rx) = broadcast::channel(256);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
        self.tx.subscribe()
    }

    pub fn publish(&self, event: LiveEvent) {
        if let Err(err) = self.tx.send(event) {
            // All subscribers dropped; that's acceptable.
            debug!(?err, "no live subscribers to consume event");
        }
    }
}

/// Forward live events from a broadcast stream to a WebSocket sink until the
/// stream ends (hub dropped) or the sink errors (socket closed).
///
/// A `Lagged` broadcast error (this subscriber fell more than the channel
/// capacity behind — e.g. a TV whose TCP send buffer stalled) SKIPS the
/// missed events and KEEPS forwarding: `BroadcastStream` resumes at the
/// oldest retained event. Breaking on lag — the previous behavior — left a
/// ZOMBIE socket: the read side stayed open so the client believed it was
/// connected, but no event (including `ndi_source_activated`) ever arrived
/// again until a manual page reload. Matches the Lagged handling in
/// `companion/mod.rs` and `state/mod.rs`.
async fn forward_live_events<S>(stream: &mut BroadcastStream<LiveEvent>, sender: &mut S)
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Debug,
{
    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => match serde_json::to_string(&event) {
                Ok(payload) => {
                    if sender.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
                Err(err) => warn!(?err, "failed to serialise live event"),
            },
            Err(err) => {
                warn!(?err, "live subscriber lagged; skipping missed events");
            }
        }
    }
}

/// #732: emit ONE `presenter::stage::diag` INFO line for a recorded NDI
/// `<video>` diagnostics snapshot — but only when the tracker's rate-limiter
/// (change-triggered, else ≤1/30 s per stage) allowed it. This is the
/// evidence lane the owner reads live at the next event
/// (`journalctl -u presenter | grep stage::diag`).
fn log_stage_diag(client_ip: &str, id: Uuid, record: &DiagRecord) {
    if !record.should_log {
        return;
    }
    let d = record.snapshot.ndi_video.as_ref();
    info!(
        target: "presenter::stage::diag",
        client_ip = %client_ip,
        id = %id,
        layout = %record.snapshot.layout_code,
        paused = ?d.and_then(|v| v.paused),
        ready_state = ?d.and_then(|v| v.ready_state),
        video_width = ?d.and_then(|v| v.video_width),
        video_height = ?d.and_then(|v| v.video_height),
        current_time = ?d.and_then(|v| v.current_time),
        error_code = ?d.and_then(|v| v.error_code),
        has_src_object = ?d.and_then(|v| v.has_src_object),
        muted = ?d.and_then(|v| v.muted),
        controls = ?d.and_then(|v| v.controls),
        frames_decoded = ?d.and_then(|v| v.frames_decoded),
        frames_dropped = ?d.and_then(|v| v.frames_dropped),
        last_frame_age_ms = ?d.and_then(|v| v.last_frame_age_ms),
        playback_guard_replays = ?d.and_then(|v| v.playback_guard_replays),
        cover_visible = ?d.and_then(|v| v.cover_visible),
        "stage ndi video diagnostics (#732)"
    );
}

/// Route one parsed inbound message to its handler. Extracted from
/// `serve_websocket` so that function stays well under the fn-length cap
/// (#732 added the diagnostics arms).
async fn dispatch_inbound(
    inbound: InboundMessage,
    hub: &LiveHub,
    connections: &StageConnections,
    client_ip: &str,
    preview: bool,
    registered_client: &mut Option<Uuid>,
) {
    match inbound {
        InboundMessage::StagePresence {
            client_id,
            layout_code,
            user_agent,
        } => {
            handle_stage_presence(
                client_id,
                layout_code,
                user_agent,
                hub,
                connections,
                client_ip,
                preview,
                registered_client,
            )
            .await;
        }
        InboundMessage::StageHeartbeatAck {
            client_id,
            heartbeat_id,
            ndi_video,
        } => {
            handle_heartbeat_ack(
                client_id,
                heartbeat_id,
                ndi_video,
                hub,
                connections,
                client_ip,
            )
            .await;
        }
        InboundMessage::StageDiag {
            client_id,
            ndi_video,
        } => {
            handle_stage_diag(client_id, ndi_video, hub, connections, client_ip).await;
        }
        InboundMessage::StageDisconnect { client_id } => {
            handle_stage_disconnect(client_id, hub, connections, registered_client).await;
        }
        InboundMessage::Unknown => {}
    }
}

/// A stage TV (or the operator preview mirror) announcing itself. A preview
/// client (`/stage?preview=1`, #460) renders live but is excluded from the
/// stage-monitor count, so it never registers. A real client registers,
/// records its `user_agent` (#732), and logs it once under
/// `presenter::stage::diag`.
#[allow(clippy::too_many_arguments)]
async fn handle_stage_presence(
    client_id: String,
    layout_code: String,
    user_agent: Option<String>,
    hub: &LiveHub,
    connections: &StageConnections,
    client_ip: &str,
    preview: bool,
    registered_client: &mut Option<Uuid>,
) {
    if preview {
        debug!(
            %client_id,
            %layout_code,
            "preview stage client — excluded from stage-monitor count"
        );
        return;
    }
    match Uuid::parse_str(&client_id) {
        Ok(id) => {
            let now = Utc::now();
            let snapshot = connections.register(id, &layout_code, now).await;
            connections.set_user_agent(id, user_agent.clone()).await;
            hub.publish(LiveEvent::StageConnection { snapshot });
            *registered_client = Some(id);
            info!(
                target: "presenter::stage::diag",
                client_ip = %client_ip,
                %id,
                user_agent = ?user_agent,
                "stage display connected — user agent (#732)"
            );
        }
        Err(err) => warn!(?client_id, ?err, "invalid stage client id"),
    }
}

/// Heartbeat ACK — records the round-trip and, when the client attached a
/// diagnostics snapshot (#732 heartbeat cadence), stores + rate-limit-logs it.
/// A single `StageConnection` broadcast carries the freshest snapshot (the
/// diag one when present, else the ack one).
async fn handle_heartbeat_ack(
    client_id: String,
    heartbeat_id: Option<String>,
    ndi_video: Option<NdiVideoDiag>,
    hub: &LiveHub,
    connections: &StageConnections,
    client_ip: &str,
) {
    let id = match Uuid::parse_str(&client_id) {
        Ok(id) => id,
        Err(err) => {
            warn!(?client_id, ?err, "invalid stage heartbeat id");
            return;
        }
    };
    let now = Utc::now();
    let heartbeat_uuid = heartbeat_id.as_ref().and_then(|v| Uuid::parse_str(v).ok());
    let ack_snapshot = connections
        .record_heartbeat_ack(id, heartbeat_uuid, now)
        .await;
    if let Some(diag) = ndi_video {
        if let Some(record) = connections.record_diag(id, diag, now).await {
            log_stage_diag(client_ip, id, &record);
            hub.publish(LiveEvent::StageConnection {
                snapshot: record.snapshot,
            });
            return;
        }
    }
    if let Some(snapshot) = ack_snapshot {
        hub.publish(LiveEvent::StageConnection { snapshot });
    }
}

/// An out-of-band diagnostics push (#732) — the client saw paused/error/cover
/// change between heartbeats. Store + rate-limit-log + rebroadcast the snapshot.
async fn handle_stage_diag(
    client_id: String,
    ndi_video: Option<NdiVideoDiag>,
    hub: &LiveHub,
    connections: &StageConnections,
    client_ip: &str,
) {
    let Some(diag) = ndi_video else { return };
    match Uuid::parse_str(&client_id) {
        Ok(id) => {
            if let Some(record) = connections.record_diag(id, diag, Utc::now()).await {
                log_stage_diag(client_ip, id, &record);
                hub.publish(LiveEvent::StageConnection {
                    snapshot: record.snapshot,
                });
            }
        }
        Err(err) => warn!(?client_id, ?err, "invalid stage diag id"),
    }
}

/// A stage client announcing an intentional disconnect.
async fn handle_stage_disconnect(
    client_id: String,
    hub: &LiveHub,
    connections: &StageConnections,
    registered_client: &mut Option<Uuid>,
) {
    match Uuid::parse_str(&client_id) {
        Ok(id) => {
            if let Some(snapshot) = connections.mark_disconnected(id).await {
                hub.publish(LiveEvent::StageConnection { snapshot });
            }
            if *registered_client == Some(id) {
                *registered_client = None;
            }
        }
        Err(err) => warn!(?client_id, ?err, "invalid stage disconnect id"),
    }
}

pub async fn serve_websocket(
    hub: LiveHub,
    connections: StageConnections,
    socket: WebSocket,
    client_ip: String,
    surface: String,
    preview: bool,
) {
    // Mirrors the companion connect/disconnect INFO logs (companion/mod.rs) but
    // carries the client IP and surface so live (stage/operator/tablet) clients
    // are attributable in the logs (#471).
    info!(client_ip = %client_ip, surface = %surface, preview, "live ws client connected");

    let rx = hub.subscribe();
    let mut stream = BroadcastStream::new(rx);
    let (mut sender, mut receiver) = socket.split();

    let forward_handle: JoinHandle<()> = tokio::spawn(async move {
        forward_live_events(&mut stream, &mut sender).await;
    });

    let mut registered_client: Option<Uuid> = None;

    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(payload) => match serde_json::from_str::<InboundMessage>(&payload) {
                Ok(inbound) => {
                    dispatch_inbound(
                        inbound,
                        &hub,
                        &connections,
                        &client_ip,
                        preview,
                        &mut registered_client,
                    )
                    .await;
                }
                Err(err) => warn!(?err, "failed to parse inbound live message"),
            },
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {}
        }
    }

    if let Some(id) = registered_client {
        if let Some(snapshot) = connections.mark_disconnected(id).await {
            hub.publish(LiveEvent::StageConnection { snapshot });
        }
    }

    forward_handle.abort();

    info!(client_ip = %client_ip, surface = %surface, "live ws client disconnected");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// Test sink collecting every forwarded WS message.
    struct CollectSink {
        messages: Vec<Message>,
    }

    impl futures_util::Sink<Message> for CollectSink {
        type Error = std::convert::Infallible;

        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.messages.push(item);
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    /// Regression (stage white-screen incident): a subscriber that LAGS the
    /// broadcast channel (TV with a stalled TCP send buffer) must SKIP the
    /// missed events and KEEP forwarding — not silently stop forever. A
    /// forwarder that dies on lag leaves the socket open as a zombie: the
    /// stage client never receives `NdiSourceActivated` again until a manual
    /// page reload.
    #[tokio::test]
    async fn forwarder_survives_broadcast_lag() {
        let hub = LiveHub::new();
        let rx = hub.subscribe();
        let mut stream = BroadcastStream::new(rx);

        // Overflow the 256-slot broadcast buffer BEFORE the subscriber polls:
        // its first poll yields Err(Lagged(..)), then the retained events.
        for i in 0..300 {
            hub.publish(LiveEvent::NdiConnectionStatus {
                status: format!("event-{i}"),
            });
        }
        // Activation arrives AFTER the lag — the event the stage must see.
        hub.publish(LiveEvent::NdiSourceActivated {
            source_id: "src-1".into(),
            ndi_name: "TEST (PRESENTER-TEST)".into(),
            label: "tv".into(),
        });
        drop(hub); // close the channel so the stream terminates

        let mut sink = CollectSink {
            messages: Vec::new(),
        };
        forward_live_events(&mut stream, &mut sink).await;

        let activation_forwarded = sink.messages.iter().any(|m| {
            matches!(m, Message::Text(t) if t.contains("ndi_source_activated")
                || t.contains("NdiSourceActivated"))
        });
        assert!(
            activation_forwarded,
            "events published after a broadcast lag must still be forwarded \
             (got {} messages, none with the activation)",
            sink.messages.len()
        );
    }
}
