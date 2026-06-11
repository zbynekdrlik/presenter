//! NdiVideo — WHEP-subscribing `<video>` element for one NDI source.
//!
//! Each `<NdiVideo>` mounts an HTMLVideoElement and connects to
//! `/ndi/whep/<source_id>` via the WHEP protocol. The browser handles
//! ICE/DTLS/SRTP/jitter-buffer/AV-sync natively. WASM is signaling glue only.

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

/// True when the codec fallback mode is "vp8". Any other value (including
/// absent) means the default H264-capable offer.
fn codec_mode_is_vp8() -> bool {
    local_storage()
        .and_then(|s| s.get_item(CODEC_MODE_KEY).ok().flatten())
        .as_deref()
        == Some("vp8")
}

/// Flip the codec mode (h264 → vp8 or vp8 → h264), persist it, and return the
/// new mode. The toggle (rather than a one-way switch) prevents lock-in: a
/// display where VP8 ALSO fails the decode check goes back to trying H264 on
/// the next attempt, alternating until one works.
fn toggle_codec_mode() -> &'static str {
    let new_mode = if codec_mode_is_vp8() { "h264" } else { "vp8" };
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(CODEC_MODE_KEY, new_mode);
    }
    new_mode
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

/// Watchdog that fires `on_failure` when EITHER:
/// - the RTCPeerConnection's iceConnectionState becomes "failed", "disconnected",
///   or "closed" (genuine connection loss), OR
/// - after playback has started, the <video> element's currentTime stops
///   advancing for STALL_THRESHOLD seconds (mid-stream freeze).
///
/// It deliberately does NOT reconnect on "connected but no first frame yet":
/// the server reliably delivers media to a stable consumer, so a frameless
/// healthy connection just waits. Reconnecting in that window drove a
/// multi-consumer churn spiral (every reconnect's tee add/remove disrupted the
/// other displays, so they stalled and reconnected too — all black forever).
///
/// ONE bounded exception to that rule: the codec-fallback decode check. Once
/// per Watchdog instance, at tick `FALLBACK_CHECK_TICK` (~12s), a single
/// getStats sample is taken; if the connection is CONNECTED but
/// `framesDecoded` is still below `FALLBACK_MIN_FRAMES`, the decoder is
/// considered dead (broken Vestel H264 OMX decodes ~1 frame per 8s GOP),
/// `ndiCodecMode` is toggled and `on_failure` fires so the reconnect rebuilds
/// the session with the other codec. Firing at most once per ~12s session
/// cannot drive the 3s-interval churn spiral above.
///
/// The closure handles are leaked via `forget()` because wasm-bindgen `Closure`
/// types are not `Send` and removing them on drop would require keeping the
/// original handles around in a `Send`-bounded `StoredValue` — which doesn't
/// fit. Instead we use an `active: Rc<Cell<bool>>` flag: closures check it
/// first and become no-ops once cleared. `Watchdog::stop()` flips the flag.
/// The closures themselves outlive the `Watchdog` instance but consume only a
/// tiny amount of memory (a few `Rc` clones).
struct Watchdog {
    active: std::rc::Rc<std::cell::Cell<bool>>,
}

impl Watchdog {
    /// Stall threshold: once playback has started, <video>.currentTime not
    /// advancing for this many seconds triggers a reconnect (mid-stream freeze).
    /// Before the first frame there is NO timeout reconnect — a connected client
    /// waits for media (see the stall-timer comment for why a no-first-frame
    /// reconnect drove a multi-consumer churn spiral).
    const STALL_THRESHOLD_SECS: f64 = 3.0;
    /// How often the stall timer ticks (ms).
    const TICK_INTERVAL_MS: i32 = 1000;
    /// Tick at which the once-per-session codec-fallback decode check samples
    /// getStats (~12s after install at 1s ticks).
    const FALLBACK_CHECK_TICK: u32 = 12;
    /// Minimum framesDecoded expected by FALLBACK_CHECK_TICK on a healthy
    /// connection (a 30fps source yields hundreds; the broken Vestel OMX
    /// yields ~1 per 8s GOP). Below this, the current codec is declared dead.
    const FALLBACK_MIN_FRAMES: f64 = 30.0;

    /// Install ICE-state listener + stall timer. `on_failure` is called at
    /// most ONCE per Watchdog instance — after firing, both observers become
    /// no-ops (gated by the `active` flag). The stall timer also posts a
    /// stats beacon for `source_id` every 15th tick (see `maybe_post_beacon`).
    fn install<F: Fn() + 'static>(
        video: &HtmlVideoElement,
        pc: &RtcPeerConnection,
        source_id: &str,
        on_failure: F,
    ) -> Self {
        use std::cell::Cell;
        use std::rc::Rc;

        let active: Rc<Cell<bool>> = Rc::new(Cell::new(true));
        let on_failure = Rc::new(on_failure);

        // ICE state listener: fire on Failed / Disconnected / Closed.
        {
            let active = Rc::clone(&active);
            let on_failure = Rc::clone(&on_failure);
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

        // Stall timer: every TICK_INTERVAL_MS check if currentTime advanced.
        //
        // CRITICAL — only the AFTER-PLAYBACK freeze triggers a reconnect here.
        // Before the first frame we DO NOT reconnect at all, no matter how long
        // it takes, as long as the connection is otherwise healthy (ICE failures
        // are handled by the separate listener above). Reason: a "connected but
        // no first frame yet" reconnect drives a multi-consumer CHURN SPIRAL —
        // when several stage displays connect at once, each fresh consumer that
        // hasn't rendered yet tears its session down and reconnects, and every
        // reconnect's tee add/remove disrupts the OTHER consumers' streams, so
        // they stall and reconnect too → all displays churn black forever. The
        // server reliably delivers media to a STABLE consumer (verified), so the
        // right behaviour for a frameless-but-connected client is to WAIT and let
        // the fan-out settle, not to reconnect. Genuine connect failures surface
        // as ICE failed/disconnected/closed and are handled above.
        {
            let active = Rc::clone(&active);
            let on_failure = Rc::clone(&on_failure);
            let video_clone = video.clone();
            let last_time: std::rc::Rc<std::cell::Cell<f64>> =
                std::rc::Rc::new(std::cell::Cell::new(0.0));
            let last_change_at: std::rc::Rc<std::cell::Cell<f64>> =
                std::rc::Rc::new(std::cell::Cell::new(0.0));
            let playback_started: std::rc::Rc<std::cell::Cell<bool>> =
                std::rc::Rc::new(std::cell::Cell::new(false));
            let tick_count: std::rc::Rc<std::cell::Cell<u32>> =
                std::rc::Rc::new(std::cell::Cell::new(0));
            let fallback_checked: std::rc::Rc<std::cell::Cell<bool>> =
                std::rc::Rc::new(std::cell::Cell::new(false));
            let pc_for_stats = pc.clone();
            let source_id_for_stats = source_id.to_string();
            let cb = Closure::<dyn FnMut()>::new(move || {
                if !active.get() {
                    return;
                }
                // Every 15th tick: post a stats beacon so server logs capture
                // this display's real view (fps, jitter buffer, freezes) —
                // "stage is laggy" reports become diagnosable from data.
                // Runs BEFORE the stall checks below: their early returns on
                // the healthy path (frame advanced / no first frame yet)
                // would otherwise starve the beacon during normal playback.
                maybe_post_beacon(&tick_count, &pc_for_stats, &source_id_for_stats);
                // Once per session at ~12s: codec-fallback decode check (see
                // struct doc). Also placed before the stall early-returns.
                maybe_check_codec_fallback(
                    &tick_count,
                    &fallback_checked,
                    &pc_for_stats,
                    &active,
                    &on_failure,
                );
                let now_secs = leptos::web_sys::js_sys::Date::now() / 1000.0;
                let t = video_clone.current_time();
                if t > 0.0 && (t - last_time.get()).abs() > 0.001 {
                    // A frame rendered — playback is live. Reset the stall window.
                    last_time.set(t);
                    last_change_at.set(now_secs);
                    playback_started.set(true);
                    return;
                }
                if !playback_started.get() {
                    // No first frame yet — WAIT (do not reconnect). See above.
                    return;
                }
                if now_secs - last_change_at.get() >= Self::STALL_THRESHOLD_SECS {
                    leptos::logging::warn!(
                        "watchdog: <video> stalled for >{}s after playback (currentTime={t}), triggering reconnect",
                        Self::STALL_THRESHOLD_SECS
                    );
                    active.set(false);
                    (on_failure)();
                }
            });
            if let Some(window) = leptos::web_sys::window() {
                let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    Self::TICK_INTERVAL_MS,
                );
            }
            cb.forget();
        }

        Self { active }
    }

    /// Disable both observers. Idempotent. Calling `stop` after `on_failure`
    /// has already fired is a safe no-op.
    fn stop(&self) {
        self.active.set(false);
    }
}

/// Every 15th watchdog tick (~15s at 1s ticks), sample `pc.getStats()` and
/// POST a compact summary to `/ndi/client-stats`. Fire-and-forget; the
/// beacon must never disturb playback.
fn maybe_post_beacon(tick_count: &std::cell::Cell<u32>, pc: &RtcPeerConnection, source_id: &str) {
    tick_count.set(tick_count.get().wrapping_add(1));
    if tick_count.get() % 15 != 0 {
        return;
    }
    let pc = pc.clone();
    let source_id = source_id.to_string();
    spawn_local(async move {
        if let Ok(report) = JsFuture::from(pc.get_stats()).await {
            post_client_stats(&source_id, &report).await;
        }
    });
}

/// Once per Watchdog instance, at tick `Watchdog::FALLBACK_CHECK_TICK`,
/// sample getStats and check whether the decoder is actually producing
/// frames. A session that has been CONNECTED for ~12s with framesDecoded
/// below `Watchdog::FALLBACK_MIN_FRAMES` has a dead decoder (the broken
/// Vestel H264 OMX symptom: connected, ~1 frame per 8s GOP): toggle
/// `ndiCodecMode` and fire `on_failure` so the reconnect loop rebuilds the
/// connection offering the other codec.
fn maybe_check_codec_fallback<F: Fn() + 'static>(
    tick_count: &std::cell::Cell<u32>,
    checked: &std::cell::Cell<bool>,
    pc: &RtcPeerConnection,
    active: &std::rc::Rc<std::cell::Cell<bool>>,
    on_failure: &std::rc::Rc<F>,
) {
    if checked.get() || tick_count.get() < Watchdog::FALLBACK_CHECK_TICK {
        return;
    }
    checked.set(true);
    let pc = pc.clone();
    let active = std::rc::Rc::clone(active);
    let on_failure = std::rc::Rc::clone(on_failure);
    spawn_local(async move {
        let Ok(report) = JsFuture::from(pc.get_stats()).await else {
            return;
        };
        if !active.get() {
            return;
        }
        // Only a CONNECTED session gets a codec verdict: pre-connect states
        // mean media never had a chance (ICE problems are the ICE listener's
        // job, not the codec's fault).
        if !matches!(
            pc.ice_connection_state(),
            RtcIceConnectionState::Connected | RtcIceConnectionState::Completed
        ) {
            return;
        }
        let frames = inbound_video_frames_decoded(&report).unwrap_or(0.0);
        if frames >= Watchdog::FALLBACK_MIN_FRAMES {
            return;
        }
        let new_mode = toggle_codec_mode();
        leptos::logging::warn!(
            "codec fallback: switching to {new_mode} (framesDecoded={frames} after {}s)",
            Watchdog::FALLBACK_CHECK_TICK
        );
        active.set(false);
        (on_failure)();
    });
}

/// Extract `framesDecoded` from the inbound-rtp video entry of an
/// RtcStatsReport (a JS Map). None when no such entry exists yet.
fn inbound_video_frames_decoded(report: &JsValue) -> Option<f64> {
    let map: &js_sys::Map = report.unchecked_ref();
    let entries = js_sys::try_iter(&map.values()).ok().flatten()?;
    for entry in entries.flatten() {
        let get =
            |k: &str| js_sys::Reflect::get(&entry, &JsValue::from_str(k)).unwrap_or(JsValue::NULL);
        if get("type").as_string().as_deref() == Some("inbound-rtp")
            && get("kind").as_string().as_deref() == Some("video")
        {
            return get("framesDecoded").as_f64();
        }
    }
    None
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
