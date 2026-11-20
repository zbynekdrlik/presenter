//! The `GET /listen/ws` reader (#852 C3-9, spec §4, plan Decision 8): the
//! guest's live transcript for the interpreter page. Every (re)connect starts
//! with the server's `snapshot`, so the page needs no separate GET and a
//! reconnect simply replaces what it showed; then `sentence` / `tail` /
//! `session` messages, each handed to `on_message` synchronously in arrival
//! order. Liveness: the server pings every 30 s (the browser answers by
//! itself) and closes a silent client, so a socket that ends or errors is
//! reopened with the `ws::passive` back-off (1 s doubling to 10 s, reset by a
//! delivered frame); a page visible again after more than 30 s hidden
//! reconnects at once (A+B §6.7, iOS suspends sockets). Attempts are numbered
//! ([`Generation`]): an older socket, read loop or timer stands down quietly.

use super::passive::{
    next_reconnect_delay, watch_long_hide, Generation, PassiveWsState, INITIAL_RECONNECT_MS,
};
use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::listen::ws::ListenWsMessage;
use std::cell::Cell;
use std::rc::Rc;

/// Called synchronously for every message, in arrival order.
pub type ListenMessageHandler = Rc<dyn Fn(ListenWsMessage)>;

struct Shared {
    on_message: ListenMessageHandler,
    set_state: WriteSignal<PassiveWsState>,
    reconnect_delay: Cell<u32>,
    generation: Generation,
}

/// Open `/listen/ws` and keep it open while the page lives; the state is the
/// passive socket's (`connecting` / `connected` / `reconnecting`).
pub fn use_listen_websocket(on_message: ListenMessageHandler) -> ReadSignal<PassiveWsState> {
    let (state, set_state) = signal(PassiveWsState::Connecting);
    let shared = Rc::new(Shared {
        on_message,
        set_state,
        reconnect_delay: Cell::new(INITIAL_RECONNECT_MS),
        generation: Generation::default(),
    });
    let back = Rc::clone(&shared);
    watch_long_hide(move || {
        back.reconnect_delay.set(INITIAL_RECONNECT_MS);
        back.set_state.set(PassiveWsState::Reconnecting);
        connect(Rc::clone(&back));
    });
    connect(shared);
    state
}

fn listen_ws_url() -> String {
    format!("{}/listen/ws", super::ws_origin())
}

/// Open a socket as a NEW generation; when it ends, the next attempt after the
/// back-off, unless a newer attempt took over.
fn connect(shared: Rc<Shared>) {
    let generation = shared.generation.begin();
    leptos::task::spawn_local(async move {
        match WebSocket::open(&listen_ws_url()) {
            Ok(socket) => {
                use futures_util::StreamExt;
                // `_write` stays bound: dropping the write half could close the socket.
                let (_write, read) = socket.split();
                read_until_closed(read, &shared, generation).await;
            }
            Err(err) => leptos::logging::log!("listen ws: open failed: {err:?}"),
        }
        if !shared.generation.is_current(generation) {
            return;
        }
        shared.set_state.set(PassiveWsState::Reconnecting);
        let delay = shared.reconnect_delay.get();
        shared.reconnect_delay.set(next_reconnect_delay(delay));
        Timeout::new(delay, move || {
            if shared.generation.is_current(generation) {
                connect(shared);
            }
        })
        .forget();
    });
}

/// Read until the socket closes or errors, or a newer attempt supersedes it.
async fn read_until_closed(
    mut read: futures_util::stream::SplitStream<WebSocket>,
    shared: &Rc<Shared>,
    generation: u32,
) {
    use futures_util::StreamExt;

    let mut opened = false;
    while let Some(frame) = read.next().await {
        if !shared.generation.is_current(generation) {
            return;
        }
        match frame {
            Ok(Message::Text(text)) => {
                if !opened {
                    opened = true;
                    shared.reconnect_delay.set(INITIAL_RECONNECT_MS);
                    shared.set_state.set(PassiveWsState::Connected);
                }
                match serde_json::from_str::<ListenWsMessage>(&text) {
                    Ok(message) => (shared.on_message)(message),
                    Err(err) => leptos::logging::log!("listen ws: unreadable message: {err}"),
                }
            }
            Ok(Message::Bytes(_)) => {}
            Err(err) => {
                leptos::logging::log!("listen ws: read failed: {err:?}");
                return;
            }
        }
    }
}
