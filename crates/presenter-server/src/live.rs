use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use presenter_core::{BibleBroadcast, StageDisplaySnapshot, TimersOverview};
use serde::Serialize;
use tokio::{sync::broadcast, task::JoinHandle};
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, warn};

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

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
    Timers { overview: TimersOverview },
    Stage { snapshot: StageDisplaySnapshot },
    Bible { broadcast: BibleBroadcast },
    BibleCleared,
}

pub async fn serve_websocket(hub: LiveHub, socket: WebSocket) {
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

    while let Some(Ok(msg)) = receiver.next().await {
        if matches!(msg, Message::Close(_)) {
            break;
        }
    }

    forward_handle.abort();
}
