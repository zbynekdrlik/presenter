//! Live WebSocket hook for the stream-graphics output page (#709, epic #718).
//!
//! Modeled on `ws/stage.rs` (heartbeat-driven zombie detection + exponential
//! backoff reconnect) but simpler: the output page is a passive CONSUMER — it
//! sends no presence and no heartbeat ACK (like the plain `ws/mod.rs` hook),
//! it only reads. Connects `/live/ws?surface=stream` (`&preview=1` in preview
//! mode so the server can exclude the editor's preview iframe from real-output
//! counts, mirroring the `/stage?preview=1` precedent, #460).
//!
//! ## Why raw-JSON parsing for the two NEW events
//!
//! The `stream_state` / `stream_config_changed` LiveEvents are added to
//! `presenter-core::live::LiveEvent` by #706, which lands on a DIFFERENT branch
//! (parallel lane). This lane cannot import those variants and still compile, so
//! it parses those two frames from raw JSON into LOCAL mirror structs whose
//! fields exactly match the #706 wire shape (serde `tag = "type"`, snake_case).
//! Content events that ALREADY exist (`Timers`, `Heartbeat`) are parsed through
//! the real `LiveEvent` type. The wire shape IS the contract — after #706 merges
//! the same bytes still parse, so nothing here needs to change at integration.

use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::{BibleSlideOutput, LiveEvent, StageDisplaySnapshot, TimersOverview};
use serde::Deserialize;
use std::cell::RefCell;
use std::rc::Rc;

const INITIAL_RECONNECT_MS: u32 = 1_000;
const MAX_RECONNECT_MS: u32 = 30_000;
const HEARTBEAT_CHECK_INTERVAL_MS: u32 = 500;
const DEFAULT_DISCONNECT_MS: f64 = 12_000.0;

/// Connection state of the stream output socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamWsState {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

/// Client mirror of #706's `LiveEvent::StreamState` payload (wire tag
/// `"stream_state"`). The extra `type` field on the wire is ignored (no
/// `deny_unknown_fields`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StreamStateMsg {
    pub output: String,
    #[serde(default)]
    pub active_scene_id: Option<i64>,
    #[serde(default)]
    pub active_overlay_ids: Vec<i64>,
    pub config_revision: u64,
}

/// Client mirror of #706's `LiveEvent::StreamConfigChanged` payload (wire tag
/// `"stream_config_changed"`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ConfigChangedMsg {
    pub output: String,
    pub config_revision: u64,
}

/// A `Timers` snapshot stamped with the local receipt time, so the countdown
/// element can tick smoothly BETWEEN server pushes:
/// `remaining = seconds_remaining - floor((now_ms - received_at_ms)/1000)`.
/// Anchoring to the server value at each push avoids any dependence on the
/// client's absolute clock.
#[derive(Debug, Clone, PartialEq)]
pub struct TimersReceipt {
    pub overview: TimersOverview,
    pub received_at_ms: f64,
}

/// Signals produced by the stream WS hook. Separate per-kind signals (not one
/// coalescing `last_event`) so a burst that mixes kinds never drops one kind for
/// another — each kind is latest-wins, which is correct (every payload is a full
/// snapshot / a monotonic revision).
///
/// `stage` and `bible` reuse the EXISTING worship-stage / Bible live events
/// (#710): `stage` is the latest `Stage` snapshot (its `current` drives lyrics),
/// and `bible` is `Some` after a `BibleSlide` event / `None` after `BibleCleared`
/// (drives the verse element). Content pipelines are reused, not duplicated.
#[derive(Clone, Copy)]
pub struct StreamWsHandle {
    pub state: ReadSignal<StreamWsState>,
    pub stream_state: ReadSignal<Option<StreamStateMsg>>,
    pub config_changed: ReadSignal<Option<ConfigChangedMsg>>,
    pub timers: ReadSignal<Option<TimersReceipt>>,
    pub stage: ReadSignal<Option<StageDisplaySnapshot>>,
    pub bible: ReadSignal<Option<BibleSlideOutput>>,
}

/// Signal writers threaded into the reconnecting task.
#[derive(Clone, Copy)]
struct StreamWsSetters {
    state: WriteSignal<StreamWsState>,
    stream_state: WriteSignal<Option<StreamStateMsg>>,
    config_changed: WriteSignal<Option<ConfigChangedMsg>>,
    timers: WriteSignal<Option<TimersReceipt>>,
    stage: WriteSignal<Option<StageDisplaySnapshot>>,
    bible: WriteSignal<Option<BibleSlideOutput>>,
}

/// Open the live socket for the stream output page and keep it reconnected.
pub fn use_stream_websocket() -> StreamWsHandle {
    let (state, set_state) = signal(StreamWsState::Connecting);
    let (stream_state, set_stream_state) = signal::<Option<StreamStateMsg>>(None);
    let (config_changed, set_config_changed) = signal::<Option<ConfigChangedMsg>>(None);
    let (timers, set_timers) = signal::<Option<TimersReceipt>>(None);
    let (stage, set_stage) = signal::<Option<StageDisplaySnapshot>>(None);
    let (bible, set_bible) = signal::<Option<BibleSlideOutput>>(None);

    let setters = StreamWsSetters {
        state: set_state,
        stream_state: set_stream_state,
        config_changed: set_config_changed,
        timers: set_timers,
        stage: set_stage,
        bible: set_bible,
    };

    let reconnect_delay = Rc::new(RefCell::new(INITIAL_RECONNECT_MS));
    let last_heartbeat_at = Rc::new(RefCell::new(js_sys::Date::now()));

    // Heartbeat/liveness checker: flip the surfaced state to Disconnected once no
    // frame (heartbeat OR event) has arrived for the disconnect window. The read
    // loop below independently drops the zombie socket and triggers reconnect.
    let last_hb = last_heartbeat_at.clone();
    let _checker = gloo_timers::callback::Interval::new(HEARTBEAT_CHECK_INTERVAL_MS, move || {
        let elapsed = js_sys::Date::now() - *last_hb.borrow();
        if elapsed >= DEFAULT_DISCONNECT_MS {
            set_state.set(StreamWsState::Disconnected);
        }
    });
    _checker.forget();

    spawn_stream_ws(setters, reconnect_delay, last_heartbeat_at);

    StreamWsHandle {
        state,
        stream_state,
        config_changed,
        timers,
        stage,
        bible,
    }
}

/// Spawn the connect → read → backoff-reconnect task.
fn spawn_stream_ws(
    setters: StreamWsSetters,
    reconnect_delay: Rc<RefCell<u32>>,
    last_heartbeat_at: Rc<RefCell<f64>>,
) {
    use futures_util::StreamExt;

    let reconnect_delay_clone = reconnect_delay.clone();
    let last_hb = last_heartbeat_at.clone();

    leptos::task::spawn_local(async move {
        let url = ws_url();

        match WebSocket::open(&url) {
            Ok(ws) => {
                let (_write, read) = ws.split();
                setters.state.set(StreamWsState::Connected);
                *reconnect_delay_clone.borrow_mut() = INITIAL_RECONNECT_MS;
                *last_hb.borrow_mut() = js_sys::Date::now();

                run_read_loop(read, setters, &last_hb).await;

                setters.state.set(StreamWsState::Reconnecting);
            }
            Err(_) => {
                setters.state.set(StreamWsState::Disconnected);
            }
        }

        // Exponential backoff reconnect.
        let delay = {
            let mut d = reconnect_delay.borrow_mut();
            let current = *d;
            *d = (*d * 2).min(MAX_RECONNECT_MS);
            current
        };

        Timeout::new(delay, move || {
            setters.state.set(StreamWsState::Connecting);
            spawn_stream_ws(setters, reconnect_delay, last_heartbeat_at);
        })
        .forget();
    });
}

/// Read frames until the socket closes, errors, or goes silent past the
/// disconnect window (zombie: TCP-open but the server's forward task died).
/// Returning lets the caller drop the socket and schedule a reconnect — the
/// exact `ws/stage.rs` zombie-recovery pattern.
async fn run_read_loop(
    mut read: futures_util::stream::SplitStream<WebSocket>,
    setters: StreamWsSetters,
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
                        leptos::logging::log!(
                            "stream ws: no frame for {elapsed:.0}ms — dropping zombie socket and reconnecting"
                        );
                        return;
                    }
                    continue;
                }
            }
        };
        let Some(msg) = msg else { return };
        match msg {
            Ok(Message::Text(text)) => handle_text(&text, setters, last_hb),
            Ok(Message::Bytes(_)) => {}
            Err(_) => return,
        }
    }
}

/// Dispatch one inbound text frame. Any frame (heartbeat or event) counts as
/// liveness and resets the zombie deadline.
fn handle_text(text: &str, setters: StreamWsSetters, last_hb: &Rc<RefCell<f64>>) {
    let now = js_sys::Date::now();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    // Extract the tag as an OWNED string so the `value.get(...)` borrow is
    // released before an arm moves `value` into `serde_json::from_value`.
    let tag = value
        .get("type")
        .and_then(|t| t.as_str())
        .map(str::to_string);
    match tag.as_deref() {
        Some("stream_state") => {
            if let Ok(msg) = serde_json::from_value::<StreamStateMsg>(value) {
                *last_hb.borrow_mut() = now;
                setters.stream_state.set(Some(msg));
            }
        }
        Some("stream_config_changed") => {
            if let Ok(msg) = serde_json::from_value::<ConfigChangedMsg>(value) {
                *last_hb.borrow_mut() = now;
                setters.config_changed.set(Some(msg));
            }
        }
        Some("timers") => {
            // Parse through the real LiveEvent type (exists today), reusing the
            // already-parsed `value` rather than re-parsing `text`.
            if let Ok(LiveEvent::Timers { overview }) = serde_json::from_value::<LiveEvent>(value) {
                *last_hb.borrow_mut() = now;
                setters.timers.set(Some(TimersReceipt {
                    overview,
                    received_at_ms: now,
                }));
            }
        }
        // Worship-stage snapshot drives the lyrics element (#710). These events
        // exist in `presenter-core::live` on this tree, so — unlike the
        // stream_state/config frames above — they are parsed through the REAL
        // `LiveEvent` type, not a mirror struct.
        Some("stage") => {
            if let Ok(LiveEvent::Stage { snapshot }) = serde_json::from_value::<LiveEvent>(value) {
                *last_hb.borrow_mut() = now;
                setters.stage.set(Some(snapshot));
            }
        }
        // Bible slide drives the verse element (#710): a `bible_slide` sets the
        // current output, `bible_cleared` clears it (verse gone).
        Some("bible_slide") => {
            if let Ok(LiveEvent::BibleSlide { output }) = serde_json::from_value::<LiveEvent>(value)
            {
                *last_hb.borrow_mut() = now;
                setters.bible.set(Some(output));
            }
        }
        Some("bible_cleared") => {
            *last_hb.borrow_mut() = now;
            setters.bible.set(None);
        }
        Some("heartbeat") => {
            *last_hb.borrow_mut() = now;
            setters.state.set(StreamWsState::Connected);
        }
        // Any other event still proves the link is alive.
        _ => {
            *last_hb.borrow_mut() = now;
        }
    }
}

/// Build the `/live/ws` URL, tagging `surface=stream` and `preview=1` when the
/// page is in editor-preview mode.
fn ws_url() -> String {
    let window = web_sys::window().expect("no global window");
    let location = window.location();
    let protocol = location.protocol().unwrap_or_else(|_| "http:".to_string());
    let host = location.host().unwrap_or_else(|_| "localhost".to_string());
    let ws_protocol = if protocol == "https:" { "wss:" } else { "ws:" };
    let mut url = format!("{ws_protocol}//{host}/live/ws?surface=stream");
    if crate::utils::window::url_flag_enabled("preview") {
        url.push_str("&preview=1");
    }
    url
}
