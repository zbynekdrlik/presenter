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
//!
//! Every connection attempt is numbered ([`Generation`]): a socket, read loop
//! or backoff timer of an older attempt stands down quietly the moment a newer
//! one starts, so a forced reconnect never leaves two sockets open.

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

/// Numbers the connection attempts: only the newest one is current.
#[derive(Debug, Default)]
struct Generation(Cell<u32>);

impl Generation {
    /// Start a new attempt; every older one is superseded from now on.
    fn begin(&self) -> u32 {
        let next = self.0.get().wrapping_add(1);
        self.0.set(next);
        next
    }

    fn is_current(&self, generation: u32) -> bool {
        self.0.get() == generation
    }
}

/// The backoff after a failed attempt: doubles up to the cap.
fn next_reconnect_delay(current_ms: u32) -> u32 {
    current_ms.saturating_mul(2).min(MAX_RECONNECT_MS)
}

/// A socket that delivered nothing for this long is dead.
fn socket_is_silent(silent_ms: f64) -> bool {
    silent_ms >= SILENT_SOCKET_MS
}

/// A page visible again after being hidden this long reconnects at once.
fn hidden_long_enough(hidden_ms: f64) -> bool {
    hidden_ms > HIDDEN_RECONNECT_MS
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
    generation: Generation,
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
        generation: Generation::default(),
    });
    watch_visibility(Rc::clone(&shared));
    connect(shared);
    PassiveWsHandle { state, connects }
}

/// Open a socket as a NEW generation and keep it alive: when it ends, schedule
/// the next attempt after the backoff, unless a newer attempt took over.
fn connect(shared: Rc<Shared>) {
    let generation = shared.generation.begin();
    leptos::task::spawn_local(async move {
        match WebSocket::open(&super::ws_url(shared.surface)) {
            Ok(socket) => {
                use futures_util::StreamExt;
                // `_write` stays bound: dropping the write half could close the socket.
                let (_write, read) = socket.split();
                shared.last_frame_at.set(js_sys::Date::now());
                read_until_dead(read, &shared, generation).await;
            }
            Err(err) => leptos::logging::log!("{} ws: open failed: {err:?}", shared.surface),
        }
        if !shared.generation.is_current(generation) {
            // A newer attempt (back from a long hide) owns the state and the
            // timer; this one only lets its socket go.
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

/// Read until the socket closes, errors, goes silent for 5 s or a newer
/// attempt supersedes this one (back from a long hide).
async fn read_until_dead(
    mut read: futures_util::stream::SplitStream<WebSocket>,
    shared: &Rc<Shared>,
    generation: u32,
) {
    use futures_util::future::{select, Either};
    use futures_util::StreamExt;

    let mut opened = false;
    loop {
        let next = read.next();
        let tick = gloo_timers::future::TimeoutFuture::new(CHECK_INTERVAL_MS);
        futures_util::pin_mut!(next, tick);
        let step = select(next, tick).await;
        if !shared.generation.is_current(generation) {
            return;
        }
        match step {
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
                if socket_is_silent(silent_for) {
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

/// Back from a long hide the socket is probably dead (iOS suspends them) even
/// if it still looks open, and a backoff timer may be pending with a long
/// delay. Start a fresh connection right now with the backoff reset: the new
/// generation silences the old socket, read loop and timer, so no duplicate
/// socket opens.
fn reconnect_now(shared: &Rc<Shared>) {
    shared.reconnect_delay.set(INITIAL_RECONNECT_MS);
    shared.set_state.set(PassiveWsState::Reconnecting);
    connect(Rc::clone(shared));
}

/// Remember when the page was hidden; back after more than 30 s → reconnect
/// at once ([`reconnect_now`]). The closure lives as long as the page.
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
            if hidden_long_enough(now - since) {
                reconnect_now(&shared);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reconnect_delay_doubles_up_to_the_cap() {
        let mut delay = INITIAL_RECONNECT_MS;
        let mut seen = vec![delay];
        for _ in 0..6 {
            delay = next_reconnect_delay(delay);
            seen.push(delay);
        }
        assert_eq!(
            seen,
            vec![1_000, 2_000, 4_000, 8_000, 10_000, 10_000, 10_000]
        );
        assert_eq!(next_reconnect_delay(u32::MAX), MAX_RECONNECT_MS);
    }

    #[test]
    fn a_socket_silent_for_5_s_is_dead() {
        assert!(!socket_is_silent(4_999.0));
        assert!(socket_is_silent(5_000.0));
    }

    #[test]
    fn only_a_page_hidden_for_more_than_30_s_reconnects() {
        assert!(!hidden_long_enough(30_000.0));
        assert!(hidden_long_enough(30_001.0));
        assert!(!hidden_long_enough(0.0));
    }

    #[test]
    fn a_newer_attempt_supersedes_every_older_one() {
        let generation = Generation::default();
        let first = generation.begin();
        assert!(generation.is_current(first));
        let second = generation.begin();
        assert!(
            !generation.is_current(first),
            "the old socket, read loop and backoff timer stand down"
        );
        assert!(generation.is_current(second));
    }
}
