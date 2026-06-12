use crate::stage_connections::StageConnections;
use axum::extract::ws::{Message, WebSocket};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
pub use presenter_core::{InboundMessage, LiveEvent};
use tokio::{sync::broadcast, task::JoinHandle};
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, warn};
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
                warn!(?err, "live broadcast stream closed unexpectedly");
                break;
            }
        }
    }
}

pub async fn serve_websocket(hub: LiveHub, connections: StageConnections, socket: WebSocket) {
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
                Ok(inbound) => match inbound {
                    InboundMessage::StagePresence {
                        client_id,
                        layout_code,
                    } => match Uuid::parse_str(&client_id) {
                        Ok(id) => {
                            let now = Utc::now();
                            let snapshot = connections.register(id, &layout_code, now).await;
                            hub.publish(LiveEvent::StageConnection { snapshot });
                            registered_client = Some(id);
                        }
                        Err(err) => warn!(?client_id, ?err, "invalid stage client id"),
                    },
                    InboundMessage::StageHeartbeatAck {
                        client_id,
                        heartbeat_id,
                    } => match Uuid::parse_str(&client_id) {
                        Ok(id) => {
                            let now = Utc::now();
                            let heartbeat_uuid = heartbeat_id
                                .as_ref()
                                .and_then(|value| Uuid::parse_str(value).ok());
                            if let Some(snapshot) = connections
                                .record_heartbeat_ack(id, heartbeat_uuid, now)
                                .await
                            {
                                hub.publish(LiveEvent::StageConnection { snapshot });
                            }
                        }
                        Err(err) => warn!(?client_id, ?err, "invalid stage heartbeat id"),
                    },
                    InboundMessage::StageDisconnect { client_id } => {
                        match Uuid::parse_str(&client_id) {
                            Ok(id) => {
                                if let Some(snapshot) = connections.mark_disconnected(id).await {
                                    hub.publish(LiveEvent::StageConnection { snapshot });
                                }
                                if registered_client == Some(id) {
                                    registered_client = None;
                                }
                            }
                            Err(err) => warn!(?client_id, ?err, "invalid stage disconnect id"),
                        }
                    }
                    InboundMessage::Unknown => {}
                },
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
