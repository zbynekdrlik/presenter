use crate::state::AppState;
mod protocol;
mod variables;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use presenter_core::bible::{BibleBroadcast, BibleReference};
use presenter_core::{
    PresentationId, SlideId, StageDisplayLayout, StageDisplaySnapshot, TimerCommand, TimerState,
    TimersOverview,
};
use protocol::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{broadcast::error::RecvError, Mutex};
use tracing::{debug, info, warn};
use uuid::Uuid;

const COMPANION_SERVER_NAME: &str = "presenter";
const COMPANION_PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

pub async fn serve_companion_socket(state: AppState, socket: WebSocket) {
    let (mut raw_sender, mut raw_receiver) = socket.split();

    let hello =
        match receive_hello(state.companion_token(), &mut raw_sender, &mut raw_receiver).await {
            Ok(hello) => hello,
            Err(_) => {
                return;
            }
        };

    info!(
        client = hello.client.as_deref().unwrap_or("unknown"),
        instance = hello.instance_name.as_deref().unwrap_or("unspecified"),
        "companion client connected"
    );

    let sender = Arc::new(Mutex::new(raw_sender));
    let mut receiver = raw_receiver;
    let mut variables = initialise_variable_state(&state).await;

    if let Err(err) = send_message(
        &sender,
        OutgoingMessage::Welcome {
            server: COMPANION_SERVER_NAME,
            version: COMPANION_PROTOCOL_VERSION,
        },
    )
    .await
    {
        warn!(?err, "failed to send companion welcome message");
        return;
    }

    if let Err(err) = send_variables(&sender, &variables).await {
        warn!(?err, "failed to send initial companion variables");
        return;
    }

    let mut live_rx = state.live_hub().subscribe();

    loop {
        tokio::select! {
            maybe_msg = receiver.next() => {
                match maybe_msg {
                    Some(Ok(message)) => {
                        if let Err(err) = handle_incoming_message(&state, &sender, &mut variables, message).await {
                            warn!(?err, "terminating companion session due to incoming error");
                            break;
                        }
                    }
                    Some(Err(err)) => {
                        warn!(?err, "companion socket error");
                        break;
                    }
                    None => break,
                }
            }
            event = live_rx.recv() => {
                match event {
                    Ok(live_event) => {
                        if variables.apply_live_event(live_event) {
                            if let Err(err) = send_variables(&sender, &variables).await {
                                warn!(?err, "failed to send companion live variables");
                                break;
                            }
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        warn!(skipped, "companion client lagged; resetting variable state");
                        variables = initialise_variable_state(&state).await;
                        if let Err(err) = send_variables(&sender, &variables).await {
                            warn!(?err, "failed to send variables after lag recovery");
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }

    info!(
        client = hello.client.as_deref().unwrap_or("unknown"),
        instance = hello.instance_name.as_deref().unwrap_or("unspecified"),
        "companion client disconnected"
    );
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/companion/ws", get(websocket_entry))
        .with_state(state)
}

async fn websocket_entry(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    if !state.companion_enabled() {
        return StatusCode::NOT_FOUND.into_response();
    }

    ws.on_upgrade(move |socket| async move {
        serve_companion_socket(state, socket).await;
    })
}
#[cfg(test)]
mod tests;
