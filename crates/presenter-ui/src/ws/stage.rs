use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::callback::{Interval, Timeout};
use leptos::prelude::*;
use presenter_core::{InboundMessage, LiveEvent};
use std::cell::RefCell;
use std::rc::Rc;

const INITIAL_RECONNECT_MS: u32 = 1_000;
const MAX_RECONNECT_MS: u32 = 30_000;
const HEARTBEAT_CHECK_INTERVAL_MS: u32 = 500;
const DEFAULT_GRACE_MS: f64 = 4_500.0;
const DEFAULT_DISCONNECT_MS: f64 = 12_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageWsState {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

#[derive(Clone)]
pub struct StageWsHandle {
    pub state: ReadSignal<StageWsState>,
    pub last_event: ReadSignal<Option<LiveEvent>>,
    pub latency_ms: ReadSignal<Option<f64>>,
}

pub fn use_stage_websocket(client_id: String, layout_code: RwSignal<String>) -> StageWsHandle {
    let (state, set_state) = signal(StageWsState::Connecting);
    let (last_event, set_last_event) = signal::<Option<LiveEvent>>(None);
    let (latency_ms, set_latency_ms) = signal::<Option<f64>>(None);

    let reconnect_delay = Rc::new(RefCell::new(INITIAL_RECONNECT_MS));
    let last_heartbeat_at = Rc::new(RefCell::new(js_sys::Date::now()));

    // Heartbeat timeout checker
    let last_hb = last_heartbeat_at.clone();
    let set_state_hb = set_state;
    let _checker = Interval::new(HEARTBEAT_CHECK_INTERVAL_MS, move || {
        let elapsed = js_sys::Date::now() - *last_hb.borrow();
        if elapsed >= DEFAULT_DISCONNECT_MS {
            set_state_hb.set(StageWsState::Disconnected);
        } else if elapsed >= DEFAULT_GRACE_MS {
            set_state_hb.set(StageWsState::Reconnecting);
        }
    });
    _checker.forget();

    spawn_stage_ws(
        client_id,
        layout_code,
        set_state,
        set_last_event,
        set_latency_ms,
        reconnect_delay,
        last_heartbeat_at,
    );

    StageWsHandle {
        state,
        last_event,
        latency_ms,
    }
}

/// Shared holder for the write half of the socket (taken/restored around
/// each async send).
type SharedWrite = Rc<RefCell<Option<futures_util::stream::SplitSink<WebSocket, Message>>>>;

#[allow(clippy::too_many_arguments)]
fn spawn_stage_ws(
    client_id: String,
    layout_code: RwSignal<String>,
    set_state: WriteSignal<StageWsState>,
    set_last_event: WriteSignal<Option<LiveEvent>>,
    set_latency_ms: WriteSignal<Option<f64>>,
    reconnect_delay: Rc<RefCell<u32>>,
    last_heartbeat_at: Rc<RefCell<f64>>,
) {
    use futures_util::{SinkExt, StreamExt};

    let client_id_for_task = client_id.clone();
    let reconnect_delay_clone = reconnect_delay.clone();
    let last_hb = last_heartbeat_at.clone();

    leptos::task::spawn_local(async move {
        let url = ws_url();

        match WebSocket::open(&url) {
            Ok(ws) => {
                let (mut write, read) = ws.split();

                // Send StagePresence
                let presence = InboundMessage::StagePresence {
                    client_id: client_id_for_task.clone(),
                    layout_code: layout_code.get_untracked(),
                };
                if let Ok(json) = serde_json::to_string(&presence) {
                    let _ = write.send(Message::Text(json)).await;
                }

                set_state.set(StageWsState::Connected);
                *reconnect_delay_clone.borrow_mut() = INITIAL_RECONNECT_MS;
                *last_hb.borrow_mut() = js_sys::Date::now();

                let write: SharedWrite = Rc::new(RefCell::new(Some(write)));
                run_stage_read_loop(
                    read,
                    &write,
                    &client_id_for_task,
                    set_state,
                    set_last_event,
                    set_latency_ms,
                    &last_hb,
                )
                .await;

                set_state.set(StageWsState::Reconnecting);
            }
            Err(_) => {
                set_state.set(StageWsState::Disconnected);
            }
        }

        // Exponential backoff reconnect
        let delay = {
            let mut d = reconnect_delay.borrow_mut();
            let current = *d;
            *d = (*d * 2).min(MAX_RECONNECT_MS);
            current
        };

        Timeout::new(delay, move || {
            set_state.set(StageWsState::Connecting);
            spawn_stage_ws(
                client_id,
                layout_code,
                set_state,
                set_last_event,
                set_latency_ms,
                reconnect_delay,
                last_heartbeat_at,
            );
        })
        .forget();
    });
}

/// Read messages until the socket closes, errors, or the ZOMBIE deadline
/// trips. Each `read.next()` is raced against a short timeout: a socket
/// that is TCP-open but silent (server's forward task dead, link silently
/// gone) would otherwise pend forever — the heartbeat checker flips the UI
/// to "Disconnected" but nothing ever reconnects, so every live event
/// (including ndi_source_activated) is lost until a manual page reload
/// (stage white-screen incident). Returning lets the caller drop BOTH
/// socket halves (closing it) and schedule the backoff reconnect.
#[allow(clippy::too_many_arguments)]
async fn run_stage_read_loop(
    mut read: futures_util::stream::SplitStream<WebSocket>,
    write: &SharedWrite,
    client_id: &str,
    set_state: WriteSignal<StageWsState>,
    set_last_event: WriteSignal<Option<LiveEvent>>,
    set_latency_ms: WriteSignal<Option<f64>>,
    last_hb: &Rc<RefCell<f64>>,
) {
    use futures_util::StreamExt;

    loop {
        let msg = {
            let next_msg = read.next();
            let timeout = gloo_timers::future::TimeoutFuture::new(HEARTBEAT_CHECK_INTERVAL_MS);
            futures_util::pin_mut!(next_msg, timeout);
            match futures_util::future::select(next_msg, timeout).await {
                futures_util::future::Either::Left((msg, _)) => msg,
                futures_util::future::Either::Right(((), _)) => {
                    let elapsed = js_sys::Date::now() - *last_hb.borrow();
                    if elapsed >= DEFAULT_DISCONNECT_MS {
                        leptos::logging::warn!(
                            "stage ws: no heartbeat for {elapsed:.0}ms — dropping zombie socket and reconnecting"
                        );
                        return;
                    }
                    continue;
                }
            }
        };
        let Some(msg) = msg else { return };
        match msg {
            Ok(Message::Text(text)) => {
                handle_stage_text(
                    &text,
                    write,
                    client_id,
                    set_state,
                    set_last_event,
                    set_latency_ms,
                    last_hb,
                )
                .await;
            }
            Ok(Message::Bytes(_)) => {}
            Err(_) => return,
        }
    }
}

/// Dispatch one inbound live-event text frame.
async fn handle_stage_text(
    text: &str,
    write: &SharedWrite,
    client_id: &str,
    set_state: WriteSignal<StageWsState>,
    set_last_event: WriteSignal<Option<LiveEvent>>,
    set_latency_ms: WriteSignal<Option<f64>>,
    last_hb: &Rc<RefCell<f64>>,
) {
    use futures_util::SinkExt;

    match serde_json::from_str::<LiveEvent>(text) {
        Ok(LiveEvent::Heartbeat { id, timestamp: _ }) => {
            *last_hb.borrow_mut() = js_sys::Date::now();
            set_state.set(StageWsState::Connected);

            // Send heartbeat ACK (latency is measured server-side)
            let ack = InboundMessage::StageHeartbeatAck {
                client_id: client_id.to_string(),
                heartbeat_id: Some(id.to_string()),
            };
            if let Ok(json) = serde_json::to_string(&ack) {
                let mut writer = write.borrow_mut().take();
                if let Some(ref mut w) = writer {
                    let _ = w.send(Message::Text(json)).await;
                }
                *write.borrow_mut() = writer;
            }
        }
        Ok(LiveEvent::StageConnection { snapshot }) => {
            // Use server-measured round-trip latency for our client
            if let Ok(our_id) = uuid::Uuid::parse_str(client_id) {
                if snapshot.id == our_id {
                    set_latency_ms.set(snapshot.latency_ms.map(|ms| ms as f64));
                }
            }
        }
        Ok(event) => {
            *last_hb.borrow_mut() = js_sys::Date::now();
            set_last_event.set(Some(event));
        }
        Err(_) => {}
    }
}

fn ws_url() -> String {
    let window = web_sys::window().expect("no global window");
    let location = window.location();
    let protocol = location.protocol().unwrap_or_else(|_| "http:".to_string());
    let host = location.host().unwrap_or_else(|_| "localhost".to_string());
    let ws_protocol = if protocol == "https:" { "wss:" } else { "ws:" };
    format!("{ws_protocol}//{host}/live/ws")
}
