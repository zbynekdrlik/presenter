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
    RtcPeerConnection, RtcRtpTransceiverDirection, RtcRtpTransceiverInit, RtcSdpType,
    RtcSessionDescriptionInit, RtcTrackEvent,
};
use wasm_bindgen_futures::{spawn_local, JsFuture};

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

/// Watchdog that fires `on_failure` when ANY of:
/// - the RTCPeerConnection's iceConnectionState becomes "failed", "disconnected",
///   or "closed", OR
/// - no first frame ever renders within INITIAL_CONNECT_TIMEOUT (total-connect
///   failure), OR
/// - after playback has started, the <video> element's currentTime stops
///   advancing for STALL_THRESHOLD seconds (mid-stream freeze).
///
/// The before-first-frame vs after-playback split is deliberate: the initial
/// WebRTC connect (ICE + DTLS + first keyframe) takes a few seconds, and
/// treating that as a stall caused a reconnect spiral that prevented any frame
/// from ever rendering.
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
    const STALL_THRESHOLD_SECS: f64 = 3.0;
    /// Initial-connect budget: BEFORE the first frame renders, allow this long
    /// for ICE + DTLS + the first keyframe before giving up and reconnecting.
    /// Much longer than STALL_THRESHOLD so normal connect latency (a few
    /// seconds) is never mistaken for a stall — that mistake caused a reconnect
    /// spiral that prevented any frame from ever rendering.
    const INITIAL_CONNECT_TIMEOUT_SECS: f64 = 12.0;
    /// How often the stall timer ticks (ms).
    const TICK_INTERVAL_MS: i32 = 1000;

    /// Install ICE-state listener + stall timer. `on_failure` is called at
    /// most ONCE per Watchdog instance — after firing, both observers become
    /// no-ops (gated by the `active` flag).
    fn install<F: Fn() + 'static>(
        video: &HtmlVideoElement,
        pc: &RtcPeerConnection,
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
                    leptos::logging::warn!(
                        "watchdog: ICE state={s:?}, triggering reconnect"
                    );
                    active.set(false);
                    (on_failure)();
                }
            });
            pc.set_oniceconnectionstatechange(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        }

        // Stall timer: every TICK_INTERVAL_MS check if currentTime advanced.
        //
        // Two distinct windows, because "no frames yet" and "frames stopped"
        // need different timeouts:
        //   - BEFORE the first frame renders, the initial WebRTC connect (ICE
        //     gather + DTLS + waiting for the encoder's next keyframe) legitimately
        //     takes a few seconds. Treating that as a 3s stall caused a reconnect
        //     SPIRAL — the watchdog tore the session down right as the first
        //     keyframe was arriving, every cycle, so the <video> never rendered
        //     a single frame (the regression symptom: connected, track live, but
        //     videoWidth=0 forever). So before playback starts we only reconnect
        //     after a much longer INITIAL_CONNECT_TIMEOUT (total-failure recovery).
        //   - AFTER playback has started (currentTime advanced at least once), a
        //     3s freeze is a real stall worth reconnecting on.
        {
            let active = Rc::clone(&active);
            let on_failure = Rc::clone(&on_failure);
            let video_clone = video.clone();
            let last_time: std::rc::Rc<std::cell::Cell<f64>> =
                std::rc::Rc::new(std::cell::Cell::new(0.0));
            let last_change_at: std::rc::Rc<std::cell::Cell<f64>> =
                std::rc::Rc::new(std::cell::Cell::new(0.0));
            let installed_at: std::rc::Rc<std::cell::Cell<f64>> =
                std::rc::Rc::new(std::cell::Cell::new(0.0));
            let playback_started: std::rc::Rc<std::cell::Cell<bool>> =
                std::rc::Rc::new(std::cell::Cell::new(false));
            let cb = Closure::<dyn FnMut()>::new(move || {
                if !active.get() {
                    return;
                }
                let now_secs = leptos::web_sys::js_sys::Date::now() / 1000.0;
                if installed_at.get() == 0.0 {
                    installed_at.set(now_secs);
                }
                let t = video_clone.current_time();
                if t > 0.0 && (t - last_time.get()).abs() > 0.001 {
                    // A frame rendered — playback is live. Reset the stall window.
                    last_time.set(t);
                    last_change_at.set(now_secs);
                    playback_started.set(true);
                    return;
                }
                if !playback_started.get() {
                    // Still waiting for the FIRST frame. Do NOT treat the connect
                    // latency as a stall. Only reconnect if no frame EVER arrives
                    // within the generous initial-connect budget.
                    if now_secs - installed_at.get() >= Self::INITIAL_CONNECT_TIMEOUT_SECS {
                        leptos::logging::warn!(
                            "watchdog: no first frame within {}s, triggering reconnect",
                            Self::INITIAL_CONNECT_TIMEOUT_SECS
                        );
                        active.set(false);
                        (on_failure)();
                    }
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
                            Watchdog::install(&video, &session.pc, move || flag.set(true));

                        install_pagehide_teardown(&session);
                        session_holder.set_value(Some(ActiveConnection {
                            session,
                            watchdog,
                        }));

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
            let _ = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
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
    let cb_holder: std::rc::Rc<std::cell::RefCell<Option<Closure<dyn FnMut()>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
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
            let _ = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 3000);
        }
    });
    let _ = JsFuture::from(promise).await;
    // Detach the handler so it doesn't fire for later state changes, then drop
    // the closure (cb_holder goes out of scope at function end).
    pc.set_onicegatheringstatechange(None);
    drop(cb_holder);
}

async fn connect_whep(video: &HtmlVideoElement, source_id: &str) -> Result<WhepSession, JsValue> {
    let cfg = RtcConfiguration::new();
    let pc = RtcPeerConnection::new_with_configuration(&cfg)?;

    let video_init = RtcRtpTransceiverInit::new();
    video_init.set_direction(RtcRtpTransceiverDirection::Recvonly);
    pc.add_transceiver_with_str_and_init("video", &video_init);

    let audio_init = RtcRtpTransceiverInit::new();
    audio_init.set_direction(RtcRtpTransceiverDirection::Recvonly);
    pc.add_transceiver_with_str_and_init("audio", &audio_init);

    let video_clone = video.clone();
    let ontrack = Closure::<dyn FnMut(RtcTrackEvent)>::new(move |ev: RtcTrackEvent| {
        let streams = ev.streams();
        if let Ok(s) = streams.get(0).dyn_into::<MediaStream>() {
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

    let url = whep_url(source_id);
    let init = leptos::web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(&offer_sdp));
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
    // pointing at the session resource. We MUST store this URL and DELETE
    // it on cleanup; otherwise the server-side session leaks and after
    // ~10 leaked sessions webrtcsink's discovery starts failing for new
    // consumers (transient `failed to set sps/pps` errors that don't
    // recover).
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
    let answer = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
    answer.set_sdp(&answer_text);
    JsFuture::from(pc.set_remote_description(&answer)).await?;
    Ok(WhepSession { pc, resource_url })
}
