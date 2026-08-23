use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::callback::{Interval, Timeout};
use leptos::prelude::*;
use presenter_core::{InboundMessage, LiveEvent};
use std::cell::RefCell;
use std::rc::Rc;

use crate::ws::stage_diag::{collect_ndi_video_diag, diag_change_key, DiagChangeKey};

const INITIAL_RECONNECT_MS: u32 = 1_000;
const MAX_RECONNECT_MS: u32 = 30_000;
const HEARTBEAT_CHECK_INTERVAL_MS: u32 = 500;
const DEFAULT_GRACE_MS: f64 = 4_500.0;
const DEFAULT_DISCONNECT_MS: f64 = 12_000.0;
/// #732: how often the on-change diagnostics poll checks the NDI `<video>`
/// for a paused/error/cover change to push immediately (between heartbeats).
const DIAG_POLL_INTERVAL_MS: u32 = 500;

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

/// #732: latest-wins slot for an on-change diagnostics frame the poll produced
/// and the read loop (the sole writer) drains. A serialized `StageDiag` JSON so
/// the poll's `Interval` callback (sync) never touches the socket sink.
type PendingDiag = Rc<RefCell<Option<String>>>;

/// Send one text frame over the shared write half: take the sink, send, restore.
/// Safe against no borrow-across-await conflict because EVERY caller runs in the
/// single read-loop task (heartbeat ACK + the pending-diag drain) — there is
/// never a second task holding the sink (#732 review WARNING).
async fn send_via_writer(write: &SharedWrite, json: String) {
    use futures_util::SinkExt;

    let mut writer = write.borrow_mut().take();
    if let Some(ref mut w) = writer {
        let _ = w.send(Message::Text(json)).await;
    }
    *write.borrow_mut() = writer;
}

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

                // Send StagePresence — carries the TV WebView's userAgent for
                // the #732 diagnostics (its Chromium version lives in the UA).
                let presence = InboundMessage::StagePresence {
                    client_id: client_id_for_task.clone(),
                    layout_code: layout_code.get_untracked(),
                    user_agent: navigator_user_agent(),
                };
                if let Ok(json) = serde_json::to_string(&presence) {
                    let _ = write.send(Message::Text(json)).await;
                }

                set_state.set(StageWsState::Connected);
                *reconnect_delay_clone.borrow_mut() = INITIAL_RECONNECT_MS;
                *last_hb.borrow_mut() = js_sys::Date::now();

                let write: SharedWrite = Rc::new(RefCell::new(Some(write)));
                // #732: poll the NDI <video> for a paused/error/cover change and
                // stash a StageDiag frame the instant it changes (between
                // heartbeats). The poll NEVER sends — the read loop is the sole
                // writer of the socket sink (heartbeat ACK + this drain), so
                // there is exactly one writer and no frame can be lost to a
                // two-task take/restore race. Scoped to THIS connection —
                // dropped after the read loop returns so a reconnect doesn't
                // leak overlapping pollers.
                let pending_diag: PendingDiag = Rc::new(RefCell::new(None));
                let diag_poll = start_diag_poll(client_id_for_task.clone(), pending_diag.clone());
                run_stage_read_loop(
                    read,
                    &write,
                    &pending_diag,
                    &client_id_for_task,
                    set_state,
                    set_last_event,
                    set_latency_ms,
                    &last_hb,
                )
                .await;
                drop(diag_poll);

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
    pending_diag: &PendingDiag,
    client_id: &str,
    set_state: WriteSignal<StageWsState>,
    set_last_event: WriteSignal<Option<LiveEvent>>,
    set_latency_ms: WriteSignal<Option<f64>>,
    last_hb: &Rc<RefCell<f64>>,
) {
    use futures_util::StreamExt;

    loop {
        // #732: drain the on-change diagnostics frame the poll stashed. Done
        // HERE (the sole writer task) so ack + diag never race the sink; runs
        // at least every HEARTBEAT_CHECK_INTERVAL_MS (the select timeout below)
        // and after each inbound message, so a change is pushed within ~500ms.
        // Take-then-drop the RefMut on its OWN line so the RefCell borrow is
        // released BEFORE the await — otherwise the poll's Interval firing
        // mid-send would hit an already-borrowed panic.
        let pending = pending_diag.borrow_mut().take();
        if let Some(json) = pending {
            send_via_writer(write, json).await;
        }
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
    match serde_json::from_str::<LiveEvent>(text) {
        Ok(LiveEvent::Heartbeat { id, timestamp: _ }) => {
            *last_hb.borrow_mut() = js_sys::Date::now();
            set_state.set(StageWsState::Connected);

            // Send heartbeat ACK (latency is measured server-side). #732: the
            // heartbeat is the steady-cadence carrier for the NDI <video>
            // diagnostics snapshot (on-change pushes go via StageDiag).
            let ack = InboundMessage::StageHeartbeatAck {
                client_id: client_id.to_string(),
                heartbeat_id: Some(id.to_string()),
                ndi_video: collect_ndi_video_diag(),
            };
            if let Ok(json) = serde_json::to_string(&ack) {
                send_via_writer(write, json).await;
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

/// The browser's `navigator.userAgent` (#732 diagnostics) — `None` if
/// unavailable. On the TV WebViews this carries the Chromium version, the key
/// unknown behind the field play-arrow.
fn navigator_user_agent() -> Option<String> {
    web_sys::window().and_then(|w| w.navigator().user_agent().ok())
}

/// #732: start the per-connection on-change diagnostics poll. Every
/// `DIAG_POLL_INTERVAL_MS` it collects the NDI `<video>` snapshot and, when the
/// paused/error/cover key CHANGED since the last stash, serializes a `StageDiag`
/// frame into `pending_diag` (latest-wins) for the read loop to send — so a
/// stall/freeze/error is captured within ~500 ms rather than waiting for the
/// next heartbeat. The poll NEVER touches the socket sink itself (single-writer
/// invariant, #732 review WARNING). The returned `Interval` must be kept alive
/// for the connection and dropped when it ends.
fn start_diag_poll(client_id: String, pending_diag: PendingDiag) -> Interval {
    let last_key: Rc<RefCell<Option<DiagChangeKey>>> = Rc::new(RefCell::new(None));
    Interval::new(DIAG_POLL_INTERVAL_MS, move || {
        let diag = collect_ndi_video_diag();
        let key = diag.as_ref().map(diag_change_key);
        if key == *last_key.borrow() {
            return;
        }
        *last_key.borrow_mut() = key;
        // Only stash when there is a snapshot (video mounted); a video that
        // disappeared just updates the key so its reappearance re-triggers.
        if let Some(diag) = diag {
            let msg = InboundMessage::StageDiag {
                client_id: client_id.clone(),
                ndi_video: Some(diag),
            };
            if let Ok(json) = serde_json::to_string(&msg) {
                *pending_diag.borrow_mut() = Some(json);
            }
        }
    })
}

fn ws_url() -> String {
    let window = web_sys::window().expect("no global window");
    let location = window.location();
    let protocol = location.protocol().unwrap_or_else(|_| "http:".to_string());
    let host = location.host().unwrap_or_else(|_| "localhost".to_string());
    let ws_protocol = if protocol == "https:" { "wss:" } else { "ws:" };
    // Tag the surface so the server can log which UI opened the socket (#471).
    // The stage page and the camera page both render the stage display family.
    let mut url = format!("{ws_protocol}//{host}/live/ws?surface=stage");
    // The operator-header preview mirror loads `/stage?preview=1` (#460). Tag its
    // socket so the server excludes it from the stage-monitor connection count —
    // it still receives every live event and renders live, it just doesn't count
    // as a real stage display.
    if crate::utils::window::url_flag_enabled("preview") {
        url.push_str("&preview=1");
    }
    url
}
