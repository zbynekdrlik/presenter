use crate::stage_connections::{StageClientSnapshot, StageConnections};
use axum::extract::ws::{Message, WebSocket};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use presenter_core::{BibleBroadcast, StageDisplaySnapshot, TimersOverview};
use serde::{Deserialize, Serialize};
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
        let (tx, _rx) = broadcast::channel(64);
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InboundMessage {
    StagePresence {
        client_id: String,
        layout_code: String,
    },
    StageHeartbeatAck {
        client_id: String,
        #[serde(default)]
        heartbeat_id: Option<String>,
    },
    StageDisconnect {
        client_id: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum LiveEvent {
    Timers { overview: TimersOverview },
    Stage { snapshot: StageDisplaySnapshot },
    Heartbeat { id: Uuid, timestamp: DateTime<Utc> },
    StageConnection { snapshot: StageClientSnapshot },
    Bible { broadcast: BibleBroadcast },
    BibleCleared,
    StageLayout { code: String },
}

pub async fn serve_websocket(hub: LiveHub, connections: StageConnections, socket: WebSocket) {
    let rx = hub.subscribe();
    let mut stream = BroadcastStream::new(rx);
    let (mut sender, mut receiver) = socket.split();

    let forward_handle: JoinHandle<()> = tokio::spawn(async move {
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
