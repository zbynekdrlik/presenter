//! Presence-free live socket for read-only pages (#852: the interpreter
//! tablet and the šepkár page). Unlike `use_live_websocket` it hands EVERY
//! frame to `on_event` synchronously, in arrival order (#793: a latest-value
//! slot coalesces a burst and loses events); unlike the stage socket it sends
//! no presence, so these pages never count as stage displays.
//!
//! Liveness (A+B spec §6.7): the hub heartbeats every 1.5 s. A socket is
//! "connected" from its first frame; one silent for 5 s is dropped and
//! reopened (backoff 1 s doubling to 10 s); a page visible again after more
//! than 30 s hidden reconnects at once (iOS suspends sockets). `connects`
//! counts sockets that delivered a frame, so a page resyncs on every
//! (re)connect — the hub never replays.

use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::LiveEvent;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

const INITIAL_RECONNECT_MS: u32 = 1_000;
const MAX_RECONNECT_MS: u32 = 10_000;
const CHECK_INTERVAL_MS: u32 = 500;
const SILENT_SOCKET_MS: f64 = 5_000.0;
const HIDDEN_RECONNECT_MS: f64 = 30_000.0;

/// Called synchronously for every live event, in arrival order.
pub type LiveEventHandler = Rc<dyn Fn(LiveEvent)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassiveWsState {
    Connecting,
    Connected,
    Reconnecting,
}

impl PassiveWsState {
    /// The `data-ws` value the pages expose (and the E2E reads).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
        }
    }
}

#[derive(Clone, Copy)]
pub struct PassiveWsHandle {
    pub state: ReadSignal<PassiveWsState>,
    /// Sockets that delivered a frame so far; a change = resync now.
    pub connects: ReadSignal<u32>,
}

/// Shared by the read loop, the reconnect timer and the visibility listener.
struct Shared {
    surface: &'static str,
    on_event: LiveEventHandler,
    set_state: WriteSignal<PassiveWsState>,
    set_connects: WriteSignal<u32>,
    last_frame_at: Cell<f64>,
    reconnect_delay: Cell<u32>,
    hidden_since: Cell<Option<f64>>,
    force_reconnect: Cell<bool>,
}

pub fn use_passive_websocket(surface: &'static str, on_event: LiveEventHandler) -> PassiveWsHandle {
    let (state, set_state) = signal(PassiveWsState::Connecting);
    let (connects, set_connects) = signal(0_u32);
    let shared = Rc::new(Shared {
        surface,
        on_event,
        set_state,
        set_connects,
        last_frame_at: Cell::new(js_sys::Date::now()),
        reconnect_delay: Cell::new(INITIAL_RECONNECT_MS),
        hidden_since: Cell::new(None),
        force_reconnect: Cell::new(false),
    });
    watch_visibility(Rc::clone(&shared));
    spawn_connection(shared);
    PassiveWsHandle { state, connects }
}

fn spawn_connection(shared: Rc<Shared>) {
    leptos::task::spawn_local(async move {
        match WebSocket::open(&super::ws_url(shared.surface)) {
            Ok(socket) => {
                use futures_util::StreamExt;
                // `_write` stays bound: dropping the write half could close the socket.
                let (_write, read) = socket.split();
                shared.last_frame_at.set(js_sys::Date::now());
                shared.force_reconnect.set(false);
                read_until_dead(read, &shared).await;
            }
            Err(err) => leptos::logging::log!("{} ws: open failed: {err:?}", shared.surface),
        }
        shared.set_state.set(PassiveWsState::Reconnecting);
        let delay = shared.reconnect_delay.get();
        shared
            .reconnect_delay
            .set(delay.saturating_mul(2).min(MAX_RECONNECT_MS));
        Timeout::new(delay, move || spawn_connection(shared)).forget();
    });
}

/// Read until the socket closes, errors, goes silent for 5 s or the page
/// asks for a fresh socket (back from a long hide).
async fn read_until_dead(
    mut read: futures_util::stream::SplitStream<WebSocket>,
    shared: &Rc<Shared>,
) {
    use futures_util::future::{select, Either};
    use futures_util::StreamExt;

    let mut opened = false;
    loop {
        let next = read.next();
        let tick = gloo_timers::future::TimeoutFuture::new(CHECK_INTERVAL_MS);
        futures_util::pin_mut!(next, tick);
        match select(next, tick).await {
            Either::Left((Some(Ok(Message::Text(text))), _)) => {
                shared.last_frame_at.set(js_sys::Date::now());
                if !opened {
                    opened = true;
                    shared.reconnect_delay.set(INITIAL_RECONNECT_MS);
                    shared.set_state.set(PassiveWsState::Connected);
                    shared.set_connects.update(|n| *n += 1);
                }
                if let Ok(event) = serde_json::from_str::<LiveEvent>(&text) {
                    (shared.on_event)(event);
                }
            }
            Either::Left((Some(Ok(Message::Bytes(_))), _)) => {}
            Either::Left((Some(Err(_)) | None, _)) => return,
            Either::Right(((), _)) => {
                let silent_for = js_sys::Date::now() - shared.last_frame_at.get();
                if shared.force_reconnect.replace(false) || silent_for >= SILENT_SOCKET_MS {
                    leptos::logging::log!(
                        "{} ws: silent for {silent_for:.0} ms — reconnecting",
                        shared.surface
                    );
                    return;
                }
            }
        }
    }
}

/// Remember when the page was hidden; back after more than 30 s → fresh
/// socket at the next check (≤ 0.5 s). The closure lives as long as the page.
fn watch_visibility(shared: Rc<Shared>) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let doc = document.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        let now = js_sys::Date::now();
        if doc.visibility_state() == web_sys::VisibilityState::Hidden {
            shared.hidden_since.set(Some(now));
            return;
        }
        if let Some(since) = shared.hidden_since.take() {
            if now - since > HIDDEN_RECONNECT_MS {
                shared.force_reconnect.set(true);
            }
        }
    });
    if document
        .add_event_listener_with_callback("visibilitychange", callback.as_ref().unchecked_ref())
        .is_err()
    {
        leptos::logging::log!("passive ws: visibilitychange listener not registered");
    }
    callback.forget();
}
