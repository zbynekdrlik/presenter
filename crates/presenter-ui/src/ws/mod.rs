use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::LiveEvent;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::Window;

/// Reconnection delay in milliseconds.
const RECONNECT_DELAY_MS: u32 = 2_000;

/// WebSocket connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsState {
    Connecting,
    Connected,
    Disconnected,
    Reconnecting,
}

/// Build the WebSocket URL from the current page location.
fn ws_url() -> String {
    let window: Window = web_sys::window().expect("no global window");
    let location = window.location();
    let protocol = location.protocol().unwrap_or_else(|_| "http:".to_string());
    let host = location.host().unwrap_or_else(|_| "localhost".to_string());
    let ws_protocol = if protocol == "https:" { "wss:" } else { "ws:" };
    format!("{ws_protocol}//{host}/live/ws")
}

/// Create a reactive WebSocket connection that automatically reconnects.
///
/// Returns:
/// - `ReadSignal<WsState>` - current connection state
/// - `ReadSignal<Option<LiveEvent>>` - latest received event
pub fn use_live_websocket() -> (ReadSignal<WsState>, ReadSignal<Option<LiveEvent>>) {
    let (ws_state, set_ws_state) = signal(WsState::Connecting);
    let (last_event, set_last_event) = signal::<Option<LiveEvent>>(None);

    // Initial connection
    spawn_ws_connection(set_ws_state, set_last_event);

    (ws_state, last_event)
}

/// Spawn a WebSocket connection task.
fn spawn_ws_connection(
    set_ws_state: WriteSignal<WsState>,
    set_last_event: WriteSignal<Option<LiveEvent>>,
) {
    use futures_util::StreamExt;

    leptos::task::spawn_local(async move {
        let url = ws_url();

        match WebSocket::open(&url) {
            Ok(ws) => {
                set_ws_state.set(WsState::Connected);
                let (_write, mut read) = ws.split();

                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            match serde_json::from_str::<LiveEvent>(&text) {
                                Ok(event) => set_last_event.set(Some(event)),
                                Err(_) => { /* ignore malformed messages */ }
                            }
                        }
                        Ok(Message::Bytes(_)) => { /* ignore binary messages */ }
                        Err(_) => break,
                    }
                }

                // Connection closed — schedule reconnect
                set_ws_state.set(WsState::Reconnecting);
                schedule_reconnect(set_ws_state, set_last_event);
            }
            Err(_) => {
                set_ws_state.set(WsState::Disconnected);
                schedule_reconnect(set_ws_state, set_last_event);
            }
        }
    });
}

/// Schedule a reconnection attempt after a delay.
fn schedule_reconnect(
    set_ws_state: WriteSignal<WsState>,
    set_last_event: WriteSignal<Option<LiveEvent>>,
) {
    Timeout::new(RECONNECT_DELAY_MS, move || {
        set_ws_state.set(WsState::Connecting);
        spawn_ws_connection(set_ws_state, set_last_event);
    })
    .forget();
}
