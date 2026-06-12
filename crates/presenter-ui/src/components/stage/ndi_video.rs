//! NdiVideo — WHEP-subscribing `<video>` element for one NDI source.
//!
//! Each `<NdiVideo>` mounts an HTMLVideoElement and connects to
//! `/ndi/whep/<source_id>` via the WHEP protocol. The browser handles
//! ICE/DTLS/SRTP/jitter-buffer/AV-sync natively. WASM is signaling glue only.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use leptos::prelude::*;
use leptos::wasm_bindgen::{closure::Closure, JsCast, JsValue};
use leptos::web_sys::{
    HtmlVideoElement, MediaStream, RtcConfiguration, RtcIceConnectionState, RtcIceGatheringState,
    RtcPeerConnection, RtcRtpReceiver, RtcRtpTransceiver, RtcRtpTransceiverDirection,
    RtcRtpTransceiverInit, RtcSdpType, RtcSessionDescriptionInit, RtcTrackEvent,
};
use wasm_bindgen_futures::{spawn_local, JsFuture};

/// localStorage key for the codec fallback mode. Absent or `"h264"` = default
/// behavior (offer includes H264 → server serves H264). `"vp8"` = fallback
/// (the offer's video section is restricted to VP8+rtx via
/// `setCodecPreferences`, so the server's codec-selection rule picks the VP8
/// branch — spec addendum 2: broken Vestel H264 OMX decoders).
const CODEC_MODE_KEY: &str = "ndiCodecMode";

/// localStorage key for the persistent per-display identity used in stats
/// beacons (per-TV health attribution server-side).
const DISPLAY_ID_KEY: &str = "ndiDisplayId";

/// Access the window's localStorage (None when unavailable, e.g. sandboxed).
fn local_storage() -> Option<leptos::web_sys::Storage> {
    leptos::web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

thread_local! {
    /// In-memory codec mode for THIS page load, seeded from localStorage on
    /// first use. `None` = not yet seeded. Connect attempts read this, NOT
    /// localStorage directly: a fallback switch flips it in memory only —
    /// the sticky localStorage value is written exclusively by
    /// `persist_proven_codec_mode` once a mode actually decodes (so the
    /// persisted value is always a PROVEN one, never a guess mid-ping-pong).
    static CODEC_MODE_VP8: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    /// At most ONE codec switch per page load. One Vestel TV alternated
    /// H264↔VP8 repeatedly when its wall-clock-based decode check misfired;
    /// bounding the switch to once-per-pageload kills the ping-pong.
    static CODEC_SWITCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True when the codec fallback mode is "vp8". Any other value (including
/// absent) means the default H264-capable offer.
fn codec_mode_is_vp8() -> bool {
    CODEC_MODE_VP8.with(|cell| {
        if let Some(v) = cell.get() {
            return v;
        }
        let stored = local_storage()
            .and_then(|s| s.get_item(CODEC_MODE_KEY).ok().flatten())
            .as_deref()
            == Some("vp8");
        cell.set(Some(stored));
        stored
    })
}

/// Flip the in-memory codec mode (h264 → vp8 or vp8 → h264) and return the
/// new mode name — at most ONCE per page load. Returns `None` when the
/// one-shot switch was already spent (no further toggling until reload).
/// Deliberately does NOT touch localStorage: only a mode that goes on to
/// present `PROVEN_MODE_FRAMES` frames gets persisted.
fn switch_codec_mode_once() -> Option<&'static str> {
    if CODEC_SWITCHED.with(|c| c.replace(true)) {
        return None;
    }
    let new_vp8 = !codec_mode_is_vp8();
    CODEC_MODE_VP8.with(|c| c.set(Some(new_vp8)));
    Some(if new_vp8 { "vp8" } else { "h264" })
}

/// Persist the CURRENT codec mode to localStorage. Called once a session
/// presents `PROVEN_MODE_FRAMES` frames — the mode demonstrably decodes on
/// this display, so it is safe to make sticky across reloads.
fn persist_proven_codec_mode() {
    let mode = if codec_mode_is_vp8() { "vp8" } else { "h264" };
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(CODEC_MODE_KEY, mode);
    }
}

/// Persistent random display id (16 hex chars) for beacon attribution.
/// Generated once and stored in localStorage; None when storage is
/// unavailable (beacon then sends null — still better than dropping it).
fn display_id() -> Option<String> {
    let storage = local_storage()?;
    if let Ok(Some(id)) = storage.get_item(DISPLAY_ID_KEY) {
        if !id.is_empty() {
            return Some(id);
        }
    }
    let mut id = String::with_capacity(16);
    for _ in 0..16 {
        let digit = (js_sys::Math::random() * 16.0) as u32 % 16;
        id.push(char::from_digit(digit, 16)?);
    }
    let _ = storage.set_item(DISPLAY_ID_KEY, &id);
    Some(id)
}

/// Restrict the video transceiver's codec preferences to VP8 (+rtx) so the
/// offer carries NO H264. The server prefers H264 whenever the offer contains
/// it; an offer without H264 is exactly what selects the VP8 branch. Silent
/// no-op (with a warn) when the capabilities API is missing or lists no VP8 —
/// the offer is then left unchanged and the server serves H264 as before.
fn apply_vp8_codec_preferences(transceiver: &RtcRtpTransceiver) {
    let Some(caps) = RtcRtpReceiver::get_capabilities("video") else {
        leptos::logging::warn!(
            "codec fallback: RTCRtpReceiver.getCapabilities unavailable — offer left unchanged"
        );
        return;
    };
    let vp8_and_rtx = js_sys::Array::new();
    for codec in caps.get_codecs().iter() {
        let mime = js_sys::Reflect::get(&codec, &JsValue::from_str("mimeType"))
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if mime == "video/vp8" || mime == "video/rtx" {
            vp8_and_rtx.push(&codec);
        }
    }
    if vp8_and_rtx.length() == 0 {
        leptos::logging::warn!(
            "codec fallback: no VP8 in receiver capabilities — offer left unchanged"
        );
        return;
    }
    transceiver.set_codec_preferences(vp8_and_rtx.as_ref());
    leptos::logging::log!("codec fallback: offering VP8-only (ndiCodecMode=vp8)");
}

/// Holds an active WHEP session: the peer connection AND the WHEP resource URL
/// returned in the `Location` header on POST. The resource URL is used to
/// DELETE the session when the component unmounts — without this, sessions
/// accumulate inside webrtcsink (one per browser navigation), each one
/// occupying an encoder + ICE state, and after enough accumulation new
/// consumer connections start returning broken SDP answers (the consumer-
/// drift bug surfaced via 13 accumulated sessions producing transient
/// `rtph264pay: failed to set sps/pps` errors during discovery retries).
struct WhepSession {
    pc: RtcPeerConnection,
    resource_url: Option<String>,
}

/// Frame-presentation counters shared between the rVFC observer (writer)
/// and the health ticker (reader). All timestamps are `now_ms()` values.
struct FrameStats {
    /// Frames PRESENTED to the compositor this session (rVFC count, or the
    /// coarse currentTime proxy on rVFC-less browsers).
    frames_presented: Cell<u32>,
    /// Timestamp of the most recently presented frame.
    last_frame_at: Cell<f64>,
    /// When this session's watchdog was installed (≈ connect time).
    started_at: Cell<f64>,
}

/// Watchdog that fires `on_failure` when EITHER:
/// - the RTCPeerConnection's iceConnectionState becomes "failed",
///   "disconnected", or "closed" (genuine connection loss), OR
/// - a FRAME-BASED health rule trips (see `start_health_ticker`).
///
/// Frame observation is driven by `requestVideoFrameCallback` (fires once
/// per frame actually PRESENTED to the compositor) — NOT by wall-clock
/// currentTime sampling. The previous wall-clock heuristics misfired on
/// prod TVs whose JS timers throttle (Vestel WebViews): the 3s
/// currentTime-stall check fired during render hiccups although frames
/// decoded at 30fps, and the tick-12 codec check ping-ponged H264↔VP8 —
/// measured as 94 WHEP add/removes in 3 minutes across 4 TVs.
///
/// It deliberately does NOT reconnect on "connected but no first frame yet"
/// (except the bounded once-per-pageload codec fallback): the server
/// reliably delivers media to a stable consumer, so a frameless healthy
/// connection waits. Reconnecting in that window drove a multi-consumer
/// churn spiral (every reconnect's tee add/remove disrupted the other
/// displays, so they stalled and reconnected too — all black forever).
///
/// The closure handles are leaked via `forget()` because wasm-bindgen
/// `Closure` types are not `Send` and removing them on drop would require
/// keeping the original handles around in a `Send`-bounded `StoredValue` —
/// which doesn't fit. Instead we use an `active: Rc<Cell<bool>>` flag:
/// closures check it first and become no-ops once cleared (the rVFC chain
/// additionally stops rescheduling itself). `Watchdog::stop()` flips the
/// flag. The leaked closures consume only a few `Rc` clones each.
struct Watchdog {
    active: Rc<Cell<bool>>,
}

impl Watchdog {
    /// Real-freeze threshold: after playback has started, this long without
    /// a single PRESENTED frame triggers a reconnect. 10s tolerates render
    /// hiccups and heavy main-thread throttling — an actual freeze (zero
    /// frames at all) is unambiguous at this horizon.
    const STALL_NO_FRAME_MS: f64 = 10_000.0;
    /// True-no-decode horizon: ICE-connected with ZERO presented frames for
    /// this long after connect → the decoder is dead → codec fallback
    /// (bounded to once per page load).
    const NO_DECODE_FALLBACK_MS: f64 = 15_000.0;
    /// Beacon cadence driver tick (ms). Health decisions are frame-based;
    /// the tick only EVALUATES them and paces beacons. May fire late on
    /// throttled TVs — acceptable, the thresholds are 10-15s.
    const TICK_INTERVAL_MS: i32 = 1000;
    /// Presented-frame count at which the current codec mode is PROVEN to
    /// decode on this display and persisted to localStorage.
    const PROVEN_MODE_FRAMES: u32 = 100;
    /// rVFC-path beacon period (~15s at 30fps) — the reliable beacon channel
    /// on displays whose setInterval is throttled to near-silence (rVFC is
    /// compositor-driven and not throttled while video plays).
    const RVFC_BEACON_FRAME_PERIOD: u32 = 450;

    /// Install ICE-state listener + rVFC frame observer + health ticker.
    /// `on_failure` is called at most ONCE per Watchdog instance — after
    /// firing, all observers become no-ops (gated by the `active` flag).
    fn install<F: Fn() + 'static>(
        video: &HtmlVideoElement,
        pc: &RtcPeerConnection,
        source_id: &str,
        on_failure: F,
    ) -> Self {
        let active: Rc<Cell<bool>> = Rc::new(Cell::new(true));
        let on_failure = Rc::new(on_failure);

        install_ice_failure_listener(pc, Rc::clone(&active), Rc::clone(&on_failure));

        let now = now_ms();
        let stats = Rc::new(FrameStats {
            frames_presented: Cell::new(0),
            last_frame_at: Cell::new(now),
            started_at: Cell::new(now),
        });
        let rvfc_supported = start_rvfc_frame_observer(video, pc, source_id, &active, &stats);
        if !rvfc_supported {
            leptos::logging::warn!(
                "watchdog: requestVideoFrameCallback unsupported — using currentTime frame proxy"
            );
        }
        start_health_ticker(
            video,
            pc,
            source_id,
            &active,
            &stats,
            rvfc_supported,
            on_failure,
        );

        Self { active }
    }

    /// Disable all observers. Idempotent. Calling `stop` after `on_failure`
    /// has already fired is a safe no-op.
    fn stop(&self) {
        self.active.set(false);
    }
}

/// Monotonic now in milliseconds: `performance.now()`, with a `Date.now()`
/// fallback when the Performance API is unavailable.
fn now_ms() -> f64 {
    leptos::web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or_else(js_sys::Date::now)
}

/// Shared holder for the self-rescheduling rVFC closure (the closure needs a
/// handle to itself to re-register for the next presented frame).
type SharedRvfcClosure = Rc<RefCell<Option<Closure<dyn FnMut(JsValue, JsValue)>>>>;

/// Start a self-rescheduling `requestVideoFrameCallback` loop on `video`,
/// maintaining `stats.frames_presented` / `stats.last_frame_at`. rVFC fires
/// once per frame PRESENTED to the compositor and — unlike setInterval — is
/// NOT throttled by TV power-saving timer policies, so the counters stay
/// truthful exactly where the wall-clock heuristics lied.
///
/// Side effects driven from the frame path:
/// - at `Watchdog::PROVEN_MODE_FRAMES` presented frames, the current codec
///   mode is persisted to localStorage (proven-mode stickiness);
/// - every `Watchdog::RVFC_BEACON_FRAME_PERIOD` frames (~15s at 30fps) a
///   stats beacon posts — reliable on throttled displays where the 1s-tick
///   beacons can become sparse.
///
/// Returns false when the browser lacks rVFC (non-Chromium): the health
/// ticker then approximates frames from currentTime advance instead.
///
/// The closure is gated by `active`: once cleared it returns WITHOUT
/// rescheduling, ending the chain (the leaked holder cycle goes inert —
/// same bounded-leak idiom as the rest of this file).
fn start_rvfc_frame_observer(
    video: &HtmlVideoElement,
    pc: &RtcPeerConnection,
    source_id: &str,
    active: &Rc<Cell<bool>>,
    stats: &Rc<FrameStats>,
) -> bool {
    let supported = js_sys::Reflect::get(
        video.as_ref(),
        &JsValue::from_str("requestVideoFrameCallback"),
    )
    .map(|f| f.is_function())
    .unwrap_or(false);
    if !supported {
        return false;
    }

    let holder: SharedRvfcClosure = Rc::new(RefCell::new(None));
    let cb = {
        let active = Rc::clone(active);
        let stats = Rc::clone(stats);
        let video = video.clone();
        let pc = pc.clone();
        let source_id = source_id.to_string();
        let holder = Rc::clone(&holder);
        Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_now: JsValue, _meta: JsValue| {
            if !active.get() {
                return;
            }
            let n = stats.frames_presented.get().saturating_add(1);
            stats.frames_presented.set(n);
            stats.last_frame_at.set(now_ms());
            if n == Watchdog::PROVEN_MODE_FRAMES {
                persist_proven_codec_mode();
            }
            if n % Watchdog::RVFC_BEACON_FRAME_PERIOD == 0 {
                post_stats_beacon(&pc, &source_id);
            }
            schedule_video_frame_callback(&video, &holder);
        })
    };
    *holder.borrow_mut() = Some(cb);
    schedule_video_frame_callback(video, &holder);
    true
}

/// Invoke `video.requestVideoFrameCallback(cb)` via Reflect (web_sys has no
/// stable binding for rVFC). Silent no-op if the method is missing.
fn schedule_video_frame_callback(video: &HtmlVideoElement, holder: &SharedRvfcClosure) {
    let Ok(f) = js_sys::Reflect::get(
        video.as_ref(),
        &JsValue::from_str("requestVideoFrameCallback"),
    ) else {
        return;
    };
    let Some(f) = f.dyn_ref::<js_sys::Function>() else {
        return;
    };
    if let Some(cb) = holder.borrow().as_ref() {
        let _ = f.call1(video.as_ref(), cb.as_ref().unchecked_ref());
    }
}

/// 1s interval driving (a) the beacon cadence and (b) evaluation of the
/// FRAME-BASED health rules:
///
/// - STALL: playback started (`frames_presented > 0`) AND no frame presented
///   for `STALL_NO_FRAME_MS` → a real freeze (render hiccups never span
///   10s) → reconnect.
/// - CODEC FALLBACK: ICE connected AND zero frames presented for
///   `NO_DECODE_FALLBACK_MS` after connect (true no-decode) → switch the
///   codec mode (at most once per page load) and reconnect.
/// - No first frame yet otherwise: WAIT — a connected frameless consumer
///   must not reconnect (multi-consumer churn spiral, see Watchdog doc).
#[allow(clippy::too_many_arguments)]
fn start_health_ticker<F: Fn() + 'static>(
    video: &HtmlVideoElement,
    pc: &RtcPeerConnection,
    source_id: &str,
    active: &Rc<Cell<bool>>,
    stats: &Rc<FrameStats>,
    rvfc_supported: bool,
    on_failure: Rc<F>,
) {
    let active = Rc::clone(active);
    let stats = Rc::clone(stats);
    let video = video.clone();
    let pc = pc.clone();
    let source_id = source_id.to_string();
    let tick_count = Cell::new(0u32);
    let last_current_time = Cell::new(0.0f64);
    let cb = Closure::<dyn FnMut()>::new(move || {
        if !active.get() {
            return;
        }
        // Beacon first: the healthy-path early returns below must not
        // starve it during normal playback.
        maybe_post_beacon(&tick_count, &pc, &source_id);
        if !rvfc_supported {
            approximate_frame_from_current_time(&video, &stats, &last_current_time);
        }
        let now = now_ms();
        let frames = stats.frames_presented.get();
        if frames == 0 {
            // Pre-first-frame: only the bounded codec fallback may act.
            maybe_codec_fallback(now, &stats, &pc, &active, &on_failure);
            return;
        }
        let since_last_frame = now - stats.last_frame_at.get();
        if since_last_frame > Watchdog::STALL_NO_FRAME_MS {
            leptos::logging::warn!(
                "watchdog: no frame presented for {since_last_frame:.0}ms (frames_presented={frames}) — real freeze, reconnecting"
            );
            active.set(false);
            (on_failure)();
        }
    });
    if let Some(window) = leptos::web_sys::window() {
        let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            Watchdog::TICK_INTERVAL_MS,
        );
    }
    cb.forget();
}

/// Codec-fallback check (frame-based): a session that is ICE-connected with
/// ZERO presented frames `NO_DECODE_FALLBACK_MS` after connect has a dead
/// decoder (the broken Vestel H264 OMX symptom: connected, RTP flowing,
/// nothing presented). Switch the codec mode — bounded to ONCE per page
/// load, killing the H264↔VP8 ping-pong — and fire `on_failure` so the
/// reconnect offers the other codec.
fn maybe_codec_fallback<F: Fn() + 'static>(
    now: f64,
    stats: &FrameStats,
    pc: &RtcPeerConnection,
    active: &Rc<Cell<bool>>,
    on_failure: &Rc<F>,
) {
    if now - stats.started_at.get() < Watchdog::NO_DECODE_FALLBACK_MS {
        return;
    }
    // Only a CONNECTED session gets a codec verdict: pre-connect states mean
    // media never had a chance (ICE problems are the ICE listener's job).
    if !matches!(
        pc.ice_connection_state(),
        RtcIceConnectionState::Connected | RtcIceConnectionState::Completed
    ) {
        return;
    }
    let Some(new_mode) = switch_codec_mode_once() else {
        // One-shot spent this page load — keep waiting, never ping-pong.
        return;
    };
    leptos::logging::warn!(
        "codec fallback: 0 frames presented {}s after connect — switching to {new_mode} (once per page load)",
        Watchdog::NO_DECODE_FALLBACK_MS / 1000.0
    );
    active.set(false);
    (on_failure)();
}

/// rVFC-less fallback (non-Chromium browsers): treat currentTime advancing
/// between ticks as one presented frame. Coarse (≤1 "frame" per tick) but
/// keeps the stall and no-decode rules functional with identical semantics.
fn approximate_frame_from_current_time(
    video: &HtmlVideoElement,
    stats: &FrameStats,
    last_current_time: &Cell<f64>,
) {
    let t = video.current_time();
    if t > 0.0 && (t - last_current_time.get()).abs() > 0.001 {
        last_current_time.set(t);
        let n = stats.frames_presented.get().saturating_add(1);
        stats.frames_presented.set(n);
        stats.last_frame_at.set(now_ms());
        if n == Watchdog::PROVEN_MODE_FRAMES {
            persist_proven_codec_mode();
        }
    }
}

/// Sample `pc.getStats()` and POST a beacon. Fire-and-forget; the beacon
/// must never disturb playback.
fn post_stats_beacon(pc: &RtcPeerConnection, source_id: &str) {
    let pc = pc.clone();
    let source_id = source_id.to_string();
    spawn_local(async move {
        if let Ok(report) = JsFuture::from(pc.get_stats()).await {
            post_client_stats(&source_id, &report).await;
        }
    });
}

/// Every 15th watchdog tick (~15s at 1s ticks — slower on throttled TVs,
/// where the rVFC frame-count beacon is the reliable channel instead),
/// post a stats beacon for `source_id`.
fn maybe_post_beacon(tick_count: &Cell<u32>, pc: &RtcPeerConnection, source_id: &str) {
    tick_count.set(tick_count.get().wrapping_add(1));
    if tick_count.get() % 15 != 0 {
        return;
    }
    post_stats_beacon(pc, source_id);
}

/// Extract inbound-video stats from an RtcStatsReport (a JS Map) and POST a
/// compact summary to /ndi/client-stats. Fire-and-forget; errors ignored —
/// the beacon must never disturb playback.
async fn post_client_stats(source_id: &str, report: &JsValue) {
    let mut frames_decoded = JsValue::NULL;
    let mut fps = JsValue::NULL;
    let mut jb_delay = JsValue::NULL;
    let mut jb_emitted = JsValue::NULL;
    let mut freeze_count = JsValue::NULL;
    let mut frames_dropped = JsValue::NULL;
    let mut codec_id = JsValue::NULL;

    let map: &js_sys::Map = report.unchecked_ref();
    let entries = js_sys::try_iter(&map.values()).ok().flatten();
    if let Some(entries) = entries {
        for entry in entries.flatten() {
            let get = |k: &str| {
                js_sys::Reflect::get(&entry, &JsValue::from_str(k)).unwrap_or(JsValue::NULL)
            };
            if get("type").as_string().as_deref() == Some("inbound-rtp")
                && get("kind").as_string().as_deref() == Some("video")
            {
                frames_decoded = get("framesDecoded");
                fps = get("framesPerSecond");
                jb_delay = get("jitterBufferDelay");
                jb_emitted = get("jitterBufferEmittedCount");
                freeze_count = get("freezeCount");
                frames_dropped = get("framesDropped");
                codec_id = get("codecId");
            }
        }
    }

    // The negotiated codec: inbound-rtp's codecId is the report-map KEY of
    // the matching "codec" entry — look it up directly and read mimeType.
    let codec = codec_id
        .as_string()
        .map(|id| map.get(&JsValue::from_str(&id)))
        .and_then(|entry| js_sys::Reflect::get(&entry, &JsValue::from_str("mimeType")).ok())
        .and_then(|v| v.as_string());

    // Physical screen size, for telling TV models apart in the logs.
    let screen = leptos::web_sys::window()
        .and_then(|w| w.screen().ok())
        .and_then(|s| match (s.width(), s.height()) {
            (Ok(w), Ok(h)) => Some(format!("{w}x{h}")),
            _ => None,
        });

    // jitterBufferDelay is a cumulative sum of seconds each emitted frame
    // spent in the buffer; divide by the emitted count for the average, in ms.
    let jitter_buffer_ms = match (jb_delay.as_f64(), jb_emitted.as_f64()) {
        (Some(d), Some(n)) if n > 0.0 => Some(d / n * 1000.0),
        _ => None,
    };
    let body = serde_json::json!({
        "sourceId": source_id,
        "displayId": display_id(),
        "codec": codec,
        "screen": screen,
        "framesDecoded": frames_decoded.as_f64(),
        "fps": fps.as_f64(),
        "jitterBufferMs": jitter_buffer_ms,
        "freezeCount": freeze_count.as_f64(),
        "framesDropped": frames_dropped.as_f64(),
    })
    .to_string();

    let init = leptos::web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(&body));
    let Ok(headers) = leptos::web_sys::Headers::new() else {
        return;
    };
    let _ = headers.set("Content-Type", "application/json");
    init.set_headers(&headers);
    let Ok(request) = leptos::web_sys::Request::new_with_str_and_init("/ndi/client-stats", &init)
    else {
        return;
    };
    if let Some(window) = leptos::web_sys::window() {
        let _ = JsFuture::from(window.fetch_with_request(&request)).await;
    }
}

/// Build the WHEP endpoint URL for a given source.
pub fn whep_url(source_id: &str) -> String {
    format!("/ndi/whep/{source_id}")
}

#[component]
pub fn NdiVideo(source_id: String, #[prop(optional)] class: Option<&'static str>) -> impl IntoView {
    let video_ref = NodeRef::<leptos::html::Video>::new();
    let source_id_for_effect = source_id.clone();

    // Holds the active connection: the WHEP session + the watchdog observing
    // its health. Cleanup must close both — see on_cleanup below.
    struct ActiveConnection {
        session: WhepSession,
        watchdog: Watchdog,
    }
    // Use new_local() instead of new() because Watchdog holds Rc<Cell<bool>>
    // which is !Send + !Sync. LocalStorage drops the Send+Sync requirement;
    // safe here because we're single-threaded WASM.
    let session_holder: StoredValue<Option<ActiveConnection>, LocalStorage> =
        StoredValue::new_local(None);

    // Cancellation flag covering the race where the component unmounts BEFORE
    // `connect_whep` resolves.
    let cancelled = Arc::new(AtomicBool::new(false));

    let cancelled_for_effect = Arc::clone(&cancelled);
    Effect::new(move |_| {
        let Some(video) = video_ref.get() else { return };
        let source_id = source_id_for_effect.clone();
        let cancelled = Arc::clone(&cancelled_for_effect);
        spawn_local(async move {
            // The reconnect-trigger flag: when a watchdog fires, it sets this
            // flag; the loop drains it and reconnects.
            let reconnect_flag = std::rc::Rc::new(std::cell::Cell::new(false));

            loop {
                if cancelled.load(Ordering::Acquire) {
                    return;
                }
                match connect_whep(&video, &source_id).await {
                    Ok(session) => {
                        if cancelled.load(Ordering::Acquire) {
                            // Unmounted between POST and now — clean up server
                            // session and bail.
                            if let Some(url) = &session.resource_url {
                                dispatch_delete(url);
                            }
                            session.pc.close();
                            return;
                        }
                        // Install watchdog: on failure, set the reconnect flag.
                        let flag = std::rc::Rc::clone(&reconnect_flag);
                        let watchdog =
                            Watchdog::install(&video, &session.pc, &source_id, move || {
                                flag.set(true)
                            });

                        install_pagehide_teardown(&session);
                        session_holder.set_value(Some(ActiveConnection { session, watchdog }));

                        // Wait until either cancellation OR a watchdog fire.
                        loop {
                            if cancelled.load(Ordering::Acquire) {
                                return;
                            }
                            if reconnect_flag.get() {
                                reconnect_flag.set(false);
                                break;
                            }
                            // Poll every 100ms.
                            let promise =
                                leptos::web_sys::js_sys::Promise::new(&mut |resolve, _| {
                                    if let Some(w) = leptos::web_sys::window() {
                                        let _ = w
                                            .set_timeout_with_callback_and_timeout_and_arguments_0(
                                                &resolve, 100,
                                            );
                                    }
                                });
                            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
                        }

                        // Tear down old session before reconnecting.
                        if let Some(active) =
                            session_holder.try_update_value(|v| v.take()).flatten()
                        {
                            active.watchdog.stop();
                            if let Some(url) = &active.session.resource_url {
                                dispatch_delete(url);
                            }
                            active.session.pc.close();
                        }
                        // Loop falls through to connect_whep again with no
                        // additional backoff (first retry is immediate).
                    }
                    Err(e) => {
                        leptos::logging::warn!(
                            "reconnect_loop: connect_whep failed: {e:?}, backing off"
                        );
                        sleep_for_backoff().await;
                    }
                }
            }
        });
    });

    let cancelled_for_cleanup = Arc::clone(&cancelled);
    on_cleanup(move || {
        cancelled_for_cleanup.store(true, Ordering::Release);
        let active = session_holder.try_update_value(|opt| opt.take()).flatten();
        if let Some(active) = active {
            active.watchdog.stop();
            if let Some(url) = &active.session.resource_url {
                dispatch_delete(url);
            }
            active.session.pc.close();
        }
    });

    let class_attr = class.unwrap_or("");
    let data_source_id = source_id.clone();
    view! {
        <video
            node_ref=video_ref
            data-role="ndi-video"
            data-source-id=data_source_id
            class=class_attr
            autoplay
            muted
            playsinline
        />
    }
}

/// Sleep for an exponentially increasing duration, capped at 5s. Uses a
/// static atomic to track the current step across calls. The schedule is
/// reset implicitly when `connect_whep` succeeds and the supervising loop
/// breaks out (the static doesn't reset — but the cap at 5s makes the
/// occasional "long delay after a long failure run" harmless).
///
/// Note: `STEP` is process-global and shared across all `<NdiVideo>`
/// instances on the page. This is harmless for the current ndi-fullscreen
/// layout (single video element). If a future multi-tile layout mounts
/// multiple `<NdiVideo>` components, instance A's failure streak will
/// inflate instance B's first retry delay (still capped at 5s). When that
/// layout ships, move STEP into a per-component Rc<Cell<usize>>.
async fn sleep_for_backoff() {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    static STEP: AtomicUsize = AtomicUsize::new(0);
    let schedule_ms: [i32; 7] = [500, 1000, 2000, 4000, 5000, 5000, 5000];
    let i = STEP
        .fetch_add(1, AtomicOrdering::Relaxed)
        .min(schedule_ms.len() - 1);
    let ms = schedule_ms[i];
    let promise = leptos::web_sys::js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = leptos::web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Fire-and-forget DELETE to the WHEP session resource. SYNCHRONOUS dispatch —
/// we do NOT await the future. spawn_local-wrapped fetches do not start when
/// called from a page-unload context (the microtask queue is destroyed before
/// the future polls). Calling `window.fetch_with_request` directly enqueues
/// the request immediately; `keepalive: true` keeps it alive after unload.
///
/// Safe to call multiple times for the same URL — the server's WHEP shim
/// maps "session not found" to a 4xx (`SOURCE_NOT_ACTIVE_ERR` → 404 in
/// `ndi_whep.rs::map_signaller_error`) and we drop the Promise rather than
/// inspecting the response, so a double-DELETE produces no console noise.
/// Both the `on_cleanup` path and the `pagehide` listener may dispatch the
/// same URL when both fire on normal navigation; this is idempotent.
fn dispatch_delete(url: &str) {
    let init = leptos::web_sys::RequestInit::new();
    init.set_method("DELETE");
    let _ = js_sys::Reflect::set(&init, &"keepalive".into(), &JsValue::TRUE);
    match leptos::web_sys::Request::new_with_str_and_init(url, &init) {
        Ok(request) => {
            if let Some(window) = leptos::web_sys::window() {
                // Promise dropped intentionally — keepalive carries it through.
                let _ = window.fetch_with_request(&request);
            } else {
                leptos::logging::error!("dispatch_delete: no window object");
            }
        }
        Err(e) => {
            leptos::logging::error!("dispatch_delete: failed to build Request for {url}: {e:?}");
        }
    }
}

/// ICE state listener for the Watchdog: fires `on_failure` once on
/// Failed / Disconnected / Closed (gated by the shared `active` flag).
fn install_ice_failure_listener<F: Fn() + 'static>(
    pc: &RtcPeerConnection,
    active: std::rc::Rc<std::cell::Cell<bool>>,
    on_failure: std::rc::Rc<F>,
) {
    let pc_clone = pc.clone();
    let cb = Closure::<dyn FnMut(JsValue)>::new(move |_ev: JsValue| {
        if !active.get() {
            return;
        }
        let s = pc_clone.ice_connection_state();
        if matches!(
            s,
            RtcIceConnectionState::Failed
                | RtcIceConnectionState::Disconnected
                | RtcIceConnectionState::Closed
        ) {
            leptos::logging::warn!("watchdog: ICE state={s:?}, triggering reconnect");
            active.set(false);
            (on_failure)();
        }
    });
    pc.set_oniceconnectionstatechange(Some(cb.as_ref().unchecked_ref()));
    cb.forget();
}

/// Install a `pagehide` window listener that fires DELETE if the page is
/// being unloaded. Some browsers (and Playwright's page.goto navigation)
/// tear down the JS context before Leptos's `on_cleanup` runs; pagehide
/// fires earlier in the unload sequence so the DELETE makes it out the door.
///
/// The closure is intentionally `forget()`-leaked into JS. Storing the
/// handle on `WhepSession` would require `Closure<dyn FnMut()>: Send +
/// Sync` (Leptos `StoredValue` bound) which the wasm-bindgen type doesn't
/// implement and can't safely be forced via SendWrapper without unsafe
/// markers. The leak IS bounded: one closure per `WhepSession` lifetime,
/// each capturing a short `url: String`. Per-page-load magnitude on the
/// stage display use case is ≪1 KB total — the same as a single icon —
/// and pagehide fires only at page unload, releasing the leaked state
/// along with everything else.
fn install_pagehide_teardown(session: &WhepSession) {
    let Some(window) = leptos::web_sys::window() else {
        return;
    };
    let Some(url) = session.resource_url.clone() else {
        return;
    };
    let cb = Closure::<dyn FnMut()>::new(move || {
        dispatch_delete(&url);
    });
    let _ = window.add_event_listener_with_callback("pagehide", cb.as_ref().unchecked_ref());
    cb.forget();
}

/// Shared holder for an event-listener `Closure` that a `Promise` constructor
/// closure populates and the awaiting function later drops (clippy
/// `type_complexity` alias).
type SharedClosureHolder = std::rc::Rc<std::cell::RefCell<Option<Closure<dyn FnMut()>>>>;

/// Await ICE gathering completion (state == `Complete`) so the local
/// description carries our candidates before we POST the WHEP offer. Returns
/// immediately if already complete. Bounded by a 3 s timeout: a gather that
/// never completes (rare on LAN) resolves the wait anyway, falling back to a
/// partially-gathered offer that still connects via peer-reflexive.
async fn wait_for_ice_gathering_complete(pc: &RtcPeerConnection) {
    if pc.ice_gathering_state() == RtcIceGatheringState::Complete {
        return;
    }
    // Keep the state-change Closure in this function's scope (NOT `forget()`-ed)
    // so it is freed when we return. connect_whep is re-invoked by the watchdog
    // reconnect loop, so a forgotten closure here would leak unbounded over a
    // persistent stall; binding it to a holder drops it once per call.
    let cb_holder: SharedClosureHolder = std::rc::Rc::new(std::cell::RefCell::new(None));
    let cb_holder_for_promise = std::rc::Rc::clone(&cb_holder);
    let pc_for_promise = pc.clone();
    let promise = leptos::web_sys::js_sys::Promise::new(&mut |resolve, _reject| {
        // Resolve when gathering reaches Complete.
        let pc_inner = pc_for_promise.clone();
        let resolve_state = resolve.clone();
        let cb = Closure::<dyn FnMut()>::new(move || {
            if pc_inner.ice_gathering_state() == RtcIceGatheringState::Complete {
                let _ = resolve_state.call0(&JsValue::NULL);
            }
        });
        pc_for_promise.set_onicegatheringstatechange(Some(cb.as_ref().unchecked_ref()));
        *cb_holder_for_promise.borrow_mut() = Some(cb);
        // Timeout fallback so a stuck gather can't hang the connect.
        if let Some(window) = leptos::web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 3000);
        }
    });
    let _ = JsFuture::from(promise).await;
    // Detach the handler so it doesn't fire for later state changes, then drop
    // the closure (cb_holder goes out of scope at function end).
    pc.set_onicegatheringstatechange(None);
    drop(cb_holder);
}

async fn connect_whep(video: &HtmlVideoElement, source_id: &str) -> Result<WhepSession, JsValue> {
    // Default RTCPeerConnection config (no explicit bundle-policy). A plain
    // default-bundle client is proven to decode this server's stream in CI
    // (e2e check 1). Forcing max-bundle here was a REGRESSION — CI showed the
    // max-bundle client received ZERO frames (#372). Keep the browser default.
    let cfg = RtcConfiguration::new();
    let pc = RtcPeerConnection::new_with_configuration(&cfg)?;

    let video_init = RtcRtpTransceiverInit::new();
    video_init.set_direction(RtcRtpTransceiverDirection::Recvonly);
    let video_transceiver = pc.add_transceiver_with_str_and_init("video", &video_init);

    // Codec fallback (spec addendum 2): in "vp8" mode, strip H264 from the
    // offer so the server's offer-driven codec selection serves the VP8
    // branch. Default mode leaves the offer unchanged (H264 served).
    if codec_mode_is_vp8() {
        apply_vp8_codec_preferences(&video_transceiver);
    }

    let audio_init = RtcRtpTransceiverInit::new();
    audio_init.set_direction(RtcRtpTransceiverDirection::Recvonly);
    pc.add_transceiver_with_str_and_init("audio", &audio_init);

    attach_ontrack(&pc, video);

    let offer = JsFuture::from(pc.create_offer()).await?;
    let offer_init: RtcSessionDescriptionInit = offer.unchecked_into();
    JsFuture::from(pc.set_local_description(&offer_init)).await?;

    // Wait for ICE gathering to complete so the offer SDP we POST carries our
    // host candidates (LAN: no STUN/TURN, gathers in <1s). This makes the
    // server's webrtcbin receive our candidates directly in the offer instead
    // of relying solely on peer-reflexive discovery — more robust ICE. Bounded
    // by a timeout inside the helper so a stuck gather can't hang the connect;
    // on timeout we fall back to whatever was gathered (still works).
    wait_for_ice_gathering_complete(&pc).await;

    // Prefer the post-gather local description (includes a=candidate lines);
    // fall back to the pre-gather offer if local_description is unavailable.
    let offer_sdp = pc
        .local_description()
        .map(|d| d.sdp())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            js_sys::Reflect::get(&offer_init, &"sdp".into())
                .ok()
                .and_then(|v| v.as_string())
        })
        .unwrap_or_default();

    let (answer_text, resource_url) = post_whep_offer(source_id, &offer_sdp).await?;
    let answer = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
    answer.set_sdp(&answer_text);
    JsFuture::from(pc.set_remote_description(&answer)).await?;
    Ok(WhepSession { pc, resource_url })
}

/// Attach the `ontrack` handler: on the first inbound MediaStream, set it as the
/// `<video>` srcObject (muted, to satisfy Chrome's autoplay policy) and play.
fn attach_ontrack(pc: &RtcPeerConnection, video: &HtmlVideoElement) {
    let video_clone = video.clone();
    let ontrack = Closure::<dyn FnMut(RtcTrackEvent)>::new(move |ev: RtcTrackEvent| {
        let streams = ev.streams();
        if let Ok(s) = streams.get(0).dyn_into::<MediaStream>() {
            // Pin the receiver's jitter buffer to its minimum and let it
            // shrink back after spikes. On low-end TV WebViews the adaptive
            // buffer otherwise only ratchets UP ("delayed + choppy" stage).
            // jitterBufferTarget (ms) is the standard knob (Chrome/WebView
            // 122+); playoutDelayHint (s) is the legacy fallback. Both set
            // via Reflect (no web_sys bindings); unsupported = silent no-op.
            let receiver = ev.receiver();
            let _ = js_sys::Reflect::set(
                receiver.as_ref(),
                &JsValue::from_str("jitterBufferTarget"),
                &JsValue::from_f64(0.0),
            );
            let _ = js_sys::Reflect::set(
                receiver.as_ref(),
                &JsValue::from_str("playoutDelayHint"),
                &JsValue::from_f64(0.0),
            );

            // CRITICAL: explicitly set `muted = true` at the PROPERTY level
            // BEFORE assigning srcObject. The HTML attribute `muted=""` only
            // initializes `defaultMuted=true` — once `srcObject` is assigned
            // with a MediaStream that has audio tracks, the LIVE `muted`
            // property is reset based on the stream's audio state (typically
            // `muted=false`). Chrome's autoplay policy ONLY permits
            // programmatic `.play()` without a user gesture on muted media;
            // an unmuted video.play() throws
            //   NotAllowedError: play() can only be initiated by a user gesture
            // exactly matching what the user reported on Windows Chrome
            // (Playwright with default-disabled autoplay-policy hid this).
            //
            // Setting `el.muted = true` programmatically after srcObject is
            // the documented fix:
            // https://developer.chrome.com/blog/autoplay/
            // "Muted autoplay for video is supported by Chrome [...]"
            video_clone.set_muted(true);
            video_clone.set_src_object(Some(&s));
            // Re-assert in case the srcObject assignment racing flipped it.
            video_clone.set_muted(true);

            // Programmatic `.play()` is then permitted on muted + playsinline
            // video without user interaction.
            let play_promise = video_clone.play();
            match play_promise {
                Ok(promise) => {
                    spawn_local(async move {
                        if let Err(e) = JsFuture::from(promise).await {
                            leptos::logging::warn!(
                                "video.play() rejected by browser autoplay policy: {e:?}"
                            );
                        }
                    });
                }
                Err(e) => {
                    leptos::logging::warn!("video.play() threw: {e:?}");
                }
            }
        }
    });
    pc.set_ontrack(Some(ontrack.as_ref().unchecked_ref()));
    ontrack.forget();
}

/// POST the WHEP offer SDP and return `(answer_sdp, resource_url)`. The resource
/// URL comes from the `Location` header (resolved against the page origin) and
/// is DELETEd on cleanup so server-side sessions don't leak — after ~10 leaked
/// sessions webrtcsink's discovery starts failing for new consumers (transient
/// `failed to set sps/pps` errors that don't recover).
async fn post_whep_offer(
    source_id: &str,
    offer_sdp: &str,
) -> Result<(String, Option<String>), JsValue> {
    let url = whep_url(source_id);
    let init = leptos::web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(offer_sdp));
    let headers = leptos::web_sys::Headers::new()?;
    headers.set("Content-Type", "application/sdp")?;
    init.set_headers(&headers);
    let request = leptos::web_sys::Request::new_with_str_and_init(&url, &init)?;
    let window = leptos::web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let resp_val = JsFuture::from(window.fetch_with_request(&request)).await?;
    let resp: leptos::web_sys::Response = resp_val.dyn_into()?;
    if !resp.ok() {
        return Err(JsValue::from_str(&format!(
            "WHEP POST returned {}",
            resp.status()
        )));
    }
    // WHEP RFC 9725: server returns 201 Created with a `Location` header
    // pointing at the session resource.
    let location_header = resp
        .headers()
        .get("Location")
        .ok()
        .flatten()
        .or_else(|| resp.headers().get("location").ok().flatten());
    let resource_url = location_header.map(|loc| {
        // Location can be relative (e.g. "/whep/resource/<id>") — resolve
        // against the page origin.
        if loc.starts_with("http://") || loc.starts_with("https://") {
            loc
        } else {
            let origin = window.location().origin().unwrap_or_default();
            format!("{origin}{loc}")
        }
    });
    let answer_text = JsFuture::from(resp.text()?)
        .await?
        .as_string()
        .unwrap_or_default();
    Ok((answer_text, resource_url))
}
