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
    HtmlVideoElement, MediaStream, RtcConfiguration, RtcIceGatheringState, RtcPeerConnection,
    RtcRtpTransceiverDirection, RtcRtpTransceiverInit, RtcSdpType, RtcSessionDescriptionInit,
    RtcTrackEvent,
};
use wasm_bindgen_futures::{spawn_local, JsFuture};

use super::ndi_clock_offset::ClockOffsetSetter;
use super::ndi_frame_stats::{FramesLiveSetter, VideoLatencySetter};
use super::ndi_watchdog::{now_ms, profile_mode_is_compat, ReloadEscalation, Watchdog};
use crate::state::stage::StageContext;

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

/// Build the WHEP endpoint URL for a given source. In compat mode the URL
/// carries `?profile=compat`; otherwise it posts the bare URL. NOTE: the
/// server now serves ONE 720p H264 stream regardless of `?profile=` (see
/// `StreamProfile::from_query`), so the profile value itself is a no-op
/// server-side. The compat flip is retained ONLY because changing the URL
/// triggers a reconnect, and that reconnect re-establishes a stuck session
/// (see `ndi_watchdog`).
pub fn whep_url(source_id: &str, compat: bool) -> String {
    if compat {
        format!("/ndi/whep/{source_id}?profile=compat")
    } else {
        format!("/ndi/whep/{source_id}")
    }
}

#[component]
pub fn NdiVideo(source_id: String, #[prop(optional)] class: Option<&'static str>) -> impl IntoView {
    let video_ref = NodeRef::<leptos::html::Video>::new();
    let source_id_for_effect = source_id.clone();

    // #479: surface stage-side VIDEO latency to the StatusBar's separate
    // "video · N ms" readout. The figure lives on the shared `StageContext`
    // signal (StatusBar reads it); the rVFC frame observer inside the watchdog
    // writes it via the `setter` below. The signal is owned by the parent
    // StagePage, so it stays alive across this <NdiVideo>'s mount/unmount —
    // safe to clear from `on_cleanup`. A stray <NdiVideo> with no StageContext
    // simply gets no setter (None) and never shows a readout.
    let video_latency_sig = use_context::<StageContext>().map(|ctx| ctx.video_latency_ms);
    let video_latency_setter: Option<VideoLatencySetter> = video_latency_sig
        .map(|sig| std::rc::Rc::new(move |v: Option<f64>| sig.set(v)) as VideoLatencySetter);

    // #500: the same shared-signal pattern for "frames are presenting". The rVFC
    // observer / proxy set it true per frame; the health ticker flips it false on
    // staleness. It gates the neutral covering placeholder so a late-joining
    // client whose status is a stale `connecting` doesn't hide already-decoding
    // video. Owned by the parent StagePage, so it survives this mount/unmount and
    // is safe to clear from `on_cleanup`. No StageContext → no setter (None).
    let frames_live_sig = use_context::<StageContext>().map(|ctx| ctx.ndi_frames_live);
    let frames_live_setter: Option<FramesLiveSetter> =
        frames_live_sig.map(|sig| std::rc::Rc::new(move |v: bool| sig.set(v)) as FramesLiveSetter);

    // #510 (T3): same shared-signal pattern for the browser<->server
    // pipeline-clock offset estimate. Written by the independent rVFC-driven
    // handshake loop inside `Watchdog::install`, once per completed round
    // trip (success or failure — so a run of failures also ages the reading
    // out to `None`/`n/a`). No StageContext → no setter (None), same as above.
    let clock_offset_sig = use_context::<StageContext>().map(|ctx| ctx.clock_offset);
    let clock_offset_setter: Option<ClockOffsetSetter> = clock_offset_sig
        .map(|sig| std::rc::Rc::new(move |v: Option<(f64, f64)>| sig.set(v)) as ClockOffsetSetter);

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

    // PAGE-SESSION reload escalation (#401), created ONCE here so it survives
    // every reconnect cycle inside the effect below (each Watchdog shares it by
    // &Rc). Held in a !Send-friendly StoredValue (like `session_holder`) so
    // BOTH the effect and `on_cleanup` can reach it: the effect installs it into
    // each Watchdog; on_cleanup calls `escalation.cancel()` so an in-flight
    // /healthz check spawned just before teardown does NOT reload the page after
    // it unmounts (#417). It is created out here (not in the effect) for exactly
    // that on_cleanup reach — flipping its flag on PAGE teardown, NOT on the
    // per-reconnect Watchdog::stop()/Drop, which would suppress the #401 reload.
    let escalation_holder: StoredValue<Option<std::rc::Rc<ReloadEscalation>>, LocalStorage> =
        StoredValue::new_local(Some(ReloadEscalation::new()));

    let cancelled_for_effect = Arc::clone(&cancelled);
    let video_latency_setter_for_effect = video_latency_setter;
    let frames_live_setter_for_effect = frames_live_setter;
    let clock_offset_setter_for_effect = clock_offset_setter;
    Effect::new(move |_| {
        let Some(video) = video_ref.get() else { return };
        let source_id = source_id_for_effect.clone();
        let cancelled = Arc::clone(&cancelled_for_effect);
        let video_latency_setter = video_latency_setter_for_effect.clone();
        let frames_live_setter = frames_live_setter_for_effect.clone();
        let clock_offset_setter = clock_offset_setter_for_effect.clone();
        spawn_local(async move {
            // The reconnect-trigger flag: when a watchdog fires, it sets this
            // flag; the loop drains it and reconnects.
            let reconnect_flag = std::rc::Rc::new(std::cell::Cell::new(false));

            // PAGE-SESSION reload escalation (#401): created ONCE outside the
            // effect (in `escalation_holder`) so it survives every reconnect
            // cycle below AND is reachable from `on_cleanup`. Each Watchdog
            // shares it: the frame observer resets its timer on decoded frames,
            // and the health ticker performs a one-shot full-page reload when
            // video has been dead long enough that reconnect alone has failed
            // (the Fully Kiosk auto-reload replacement, adb-independent).
            let Some(escalation) = escalation_holder.try_get_value().flatten() else {
                return;
            };

            // Per-instance reconnect backoff step (#369), created ONCE here so
            // it survives reconnect cycles. Shared by BOTH the connect-error
            // branch and the watchdog-reconnect fall-through so neither retries
            // with no delay. Reset to 0 after a session that was clearly
            // healthy (see should_reset_backoff).
            let backoff_step: BackoffStep = std::rc::Rc::new(std::cell::Cell::new(0));

            // #502: fetch the Cloudflare TURN ICE servers for this page and
            // reuse them across reconnects (no re-mint per reconnect). The minted
            // credential has a 24h TTL, so on a long-lived stage display refresh
            // it when stale (>6h) at the top of a reconnect — otherwise a
            // reconnect after the credential expires would relay-fail (black)
            // until the #401 page reload. None (TURN unconfigured / fetch failed)
            // → default config = today's LAN-only behavior; no point re-fetching
            // a None (TURN is off), so refresh only when we already have some.
            const ICE_REFRESH_MS: f64 = 6.0 * 60.0 * 60.0 * 1000.0;
            let mut ice_servers = super::ndi_ice::fetch_ice_servers().await;
            let mut ice_fetched_at = now_ms();

            loop {
                if cancelled.load(Ordering::Acquire) {
                    return;
                }
                if ice_servers.is_some() && now_ms() - ice_fetched_at > ICE_REFRESH_MS {
                    ice_servers = super::ndi_ice::fetch_ice_servers().await;
                    ice_fetched_at = now_ms();
                }
                match connect_whep(&video, &source_id, &ice_servers).await {
                    // #431: source configured but not producing yet (server
                    // replied 204). NOT an error — show the placeholder (no
                    // srcObject is set) and back off quietly, with NO console
                    // warning/error. This is the path that eliminated the prod
                    // stage's repeated "/ndi/whep 404" console spam.
                    Ok(ConnectOutcome::NotProducing) => {
                        sleep_for_backoff(&backoff_step).await;
                    }
                    Ok(ConnectOutcome::Connected(session)) => {
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
                        // It shares the page-session `escalation` so the
                        // last-resort reload spans reconnect cycles (#401).
                        let flag = std::rc::Rc::clone(&reconnect_flag);
                        let watchdog = Watchdog::install(
                            &video,
                            &session.pc,
                            &source_id,
                            &escalation,
                            video_latency_setter.clone(),
                            frames_live_setter.clone(),
                            clock_offset_setter.clone(),
                            move || flag.set(true),
                        );

                        install_pagehide_teardown(&session);
                        session_holder.set_value(Some(ActiveConnection { session, watchdog }));

                        // When this session was established — used to decide
                        // whether its later drop was a transient blip (reset
                        // backoff) or a connect-but-never-decode source (let
                        // backoff escalate) — see should_reset_backoff (#369).
                        let session_started_at = now_ms();

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

                        // #369: back off before the watchdog-triggered
                        // reconnect re-POSTs WHEP. A session that lived long
                        // enough to clearly be decoding (a transient blip)
                        // resets the step so the reconnect is prompt; a
                        // connect-but-never-decode source keeps the step
                        // climbing toward the 5s cap, so it no longer reconnects
                        // immediately every ~12s cycle.
                        if should_reset_backoff(now_ms() - session_started_at) {
                            backoff_step.set(0);
                        }
                        sleep_for_backoff(&backoff_step).await;
                    }
                    Err(e) => {
                        leptos::logging::warn!(
                            "reconnect_loop: connect_whep failed: {e:?}, backing off"
                        );
                        sleep_for_backoff(&backoff_step).await;
                    }
                }
            }
        });
    });

    let cancelled_for_cleanup = Arc::clone(&cancelled);
    on_cleanup(move || {
        cancelled_for_cleanup.store(true, Ordering::Release);
        // #479: this <NdiVideo> is unmounting (source deactivated / layout
        // change) — no more frames, so clear the "video · N ms" readout. The
        // signal is owned by the still-alive parent StageContext, so this set
        // is safe (not a disposed-signal write).
        if let Some(sig) = video_latency_sig {
            sig.set(None);
        }
        // #500: this <NdiVideo> is unmounting — no frames are presenting, so
        // clear the live-frames flag. The neutral cover then reflects the
        // next source's true (no-frames-yet) state instead of a stale `true`.
        if let Some(sig) = frames_live_sig {
            sig.set(false);
        }
        // #417: signal PAGE teardown to the page-session escalation so any
        // /healthz check spawned by `maybe_reload` just before this cleanup does
        // NOT call window.location().reload() after the page unmounts. This is
        // the PAGE-level teardown — distinct from `Watchdog::stop()` below, which
        // fires on every reconnect (tying the cancel to it would permanently
        // suppress the #401 last-resort reload after the first reconnect).
        if let Some(escalation) = escalation_holder.try_get_value().flatten() {
            escalation.cancel();
        }
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
/// Capped-exponential backoff delay (ms) for a given reconnect retry step.
/// Step 0 is the FIRST retry and is deliberately NON-ZERO (#369).
///
/// Both reconnect paths in the supervising loop use this:
/// - the connect-error `Err` branch (a POST that failed / a 204 not-producing
///   reply), and
/// - the watchdog-triggered reconnect `Ok`-branch fall-through (a source that
///   connected but stalled / never decoded a frame).
///
/// Before #369 the watchdog-reconnect path re-POSTed WHEP IMMEDIATELY on the
/// first retry, so a connect-but-never-decode source reconnected every ~12s
/// cycle with no delay — creating + tearing down a server-side WHEP session
/// each time. Applying this schedule (500ms → 5s, capped) to that path settles
/// a persistently-broken source into one reconnect every 5s.
pub(crate) fn reconnect_backoff_for_watchdog_step(step: usize) -> i32 {
    const SCHEDULE_MS: [i32; 7] = [500, 1000, 2000, 4000, 5000, 5000, 5000];
    SCHEDULE_MS[step.min(SCHEDULE_MS.len() - 1)]
}

/// Per-`<NdiVideo>`-instance reconnect backoff step counter. Created ONCE in
/// the effect so it survives every reconnect cycle of that instance, and shared
/// by BOTH reconnect branches (#369). Per-instance (an `Rc<Cell<usize>>`, not
/// the old process-global `static`) so a future multi-tile layout's instance A
/// failure streak never inflates instance B's first retry delay.
type BackoffStep = std::rc::Rc<std::cell::Cell<usize>>;

/// A session that stayed up at least this long before the watchdog fired was
/// CLEARLY decoding fine (it lived well past `NO_DECODE_FALLBACK_MS` 15s, the
/// never-decode horizon), so its drop is a transient blip — reset the backoff
/// step so the reconnect is prompt. A session that died sooner (≈ within the
/// no-decode window) is a connect-but-never-decode source: do NOT reset, let
/// the backoff escalate toward the 5s cap (#369).
pub(crate) const HEALTHY_SESSION_MS: f64 = 20_000.0;

/// True if a session that lived `session_lifetime_ms` before the watchdog fired
/// was healthy enough to reset the reconnect backoff (#369). Pure + unit-tested.
pub(crate) fn should_reset_backoff(session_lifetime_ms: f64) -> bool {
    session_lifetime_ms >= HEALTHY_SESSION_MS
}

/// Sleep for this instance's next capped-exponential backoff duration, then
/// advance the step. Used by the connect-error branch AND the watchdog-
/// reconnect fall-through so BOTH back off (#369). The caller resets the step
/// (`step.set(0)`) after a connect that actually started decoding, so a healthy
/// reconnect doesn't carry a stale long delay.
async fn sleep_for_backoff(step: &BackoffStep) {
    let i = step.get();
    let ms = reconnect_backoff_for_watchdog_step(i);
    step.set(i.saturating_add(1));
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
/// Safe to call multiple times for the same URL — WHEP DELETE is idempotent
/// server-side (an already-gone session or inactive source returns 204, the
/// desired end state) and we drop the Promise rather than inspecting the
/// response, so a double-DELETE produces no console noise.
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

/// Connect outcome: a live session, or "the source is configured but not
/// currently producing" (#431). The not-producing case is an EXPECTED,
/// non-error state (server replies 204) — it must NOT surface as a console
/// error/warning; the supervising loop shows the placeholder and backs off.
enum ConnectOutcome {
    Connected(WhepSession),
    NotProducing,
}

async fn connect_whep(
    video: &HtmlVideoElement,
    source_id: &str,
    ice_servers: &Option<JsValue>,
) -> Result<ConnectOutcome, JsValue> {
    // Default RTCPeerConnection config (no explicit bundle-policy). A plain
    // default-bundle client is proven to decode this server's stream in CI
    // (e2e check 1). Forcing max-bundle here was a REGRESSION — CI showed the
    // max-bundle client received ZERO frames (#372). Keep the browser default.
    let cfg = RtcConfiguration::new();
    // #502: set the Cloudflare TURN ICE servers (when configured) so a relay
    // candidate exists when the direct LAN path is unreachable (Tailscale /
    // remote). On a PUBLIC origin (domain/remote) this also forces
    // iceTransportPolicy=`relay` so the browser uses the clean Cloudflare relay
    // instead of latching onto a lossy Tailscale pair; on-LAN (private IP) it
    // stays `all` (direct wins, low latency). See ndi_ice::apply_ice_servers.
    super::ndi_ice::apply_ice_servers(&cfg, ice_servers);
    let pc = RtcPeerConnection::new_with_configuration(&cfg)?;

    let video_init = RtcRtpTransceiverInit::new();
    video_init.set_direction(RtcRtpTransceiverDirection::Recvonly);
    // NO codec games on the offer (the retired VP8 fallback stripped H264
    // via setCodecPreferences): both server profiles are H264 now, and the
    // profile is requested via the WHEP URL query instead — see whep_url.
    pc.add_transceiver_with_str_and_init("video", &video_init);

    // VIDEO-ONLY offer — deliberately NO audio m-line. The server never
    // sends audio; a dead audio track in the SDP makes weak TV WebViews
    // stall video presentation on unsatisfiable A/V sync (proven by the
    // 2026-06-12 VDO.Ninja A/B measurement).

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

    // #431: a configured-but-not-producing source replies 204 No Content (no
    // SDP answer). That is NOT an error — close this peer connection and report
    // NotProducing so the loop shows the placeholder and backs off quietly,
    // with no "Failed to load resource" / "POST returned 404" console noise.
    let Some((answer_text, resource_url)) = post_whep_offer(source_id, &offer_sdp).await? else {
        pc.close();
        return Ok(ConnectOutcome::NotProducing);
    };
    // #431: a pipeline that is still STARTING (the source is being brought up
    // but hasn't produced frames yet) can answer 201 with an EMPTY or non-SDP
    // body. Feeding that to setRemoteDescription throws "Failed to parse
    // SessionDescription. Expect line: v=", which the browser logs as a console
    // error — the same zero-console-errors violation as the 404. A valid SDP
    // always begins with "v=" (RFC 4566 §5); anything else is the not-producing
    // state, handled quietly like the 204.
    if !answer_text.trim_start().starts_with("v=") {
        pc.close();
        return Ok(ConnectOutcome::NotProducing);
    }
    let answer = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
    answer.set_sdp(&answer_text);
    JsFuture::from(pc.set_remote_description(&answer)).await?;
    Ok(ConnectOutcome::Connected(WhepSession { pc, resource_url }))
}

/// Attach the `ontrack` handler: on the first inbound MediaStream, set it as the
/// `<video>` srcObject (muted, to satisfy Chrome's autoplay policy) and play.
fn attach_ontrack(pc: &RtcPeerConnection, video: &HtmlVideoElement) {
    let video_clone = video.clone();
    let ontrack = Closure::<dyn FnMut(RtcTrackEvent)>::new(move |ev: RtcTrackEvent| {
        let streams = ev.streams();
        if let Ok(s) = streams.get(0).dyn_into::<MediaStream>() {
            // Jitter-buffer policy is PROFILE-DEPENDENT (VDO.Ninja source
            // study, #387): a minimal buffer is a latency win on strong
            // decoders, but on a marginal decoder every late frame becomes a
            // dropped frame -> PLI storm -> IDR flood -> collapse spiral.
            // VDO.Ninja never forces 0 - the browser's adaptive buffer is the
            // weak device's only shock absorber. So: default profile (strong
            // HW path) pins the buffer low for lip-sync latency; compat
            // profile (weak device) leaves the browser default untouched.
            // Set via Reflect (no web_sys bindings); unsupported = no-op.
            if !profile_mode_is_compat() {
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
            }

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

/// POST the WHEP offer SDP. On 201 the answer comes back with a `Location`
/// header (resolved against the page origin) and is DELETEd on cleanup so
/// server-side sessions don't leak — after ~10 leaked sessions webrtcsink's
/// discovery starts failing for new consumers (transient `failed to set
/// sps/pps` errors that don't recover).
///
/// Returns:
/// - `Ok(Some((answer_sdp, resource_url)))` — 201 Created with an SDP answer,
/// - `Ok(None)` — 204 No Content: the source is configured but not currently
///   producing (#431); an EXPECTED transient state, NOT an error,
/// - `Err(_)` — a genuine failure (network, or a non-2xx the client can't
///   recover from).
async fn post_whep_offer(
    source_id: &str,
    offer_sdp: &str,
) -> Result<Option<(String, Option<String>)>, JsValue> {
    // Profile fallback (spec addendum 2 pivot): compat mode requests the
    // server's 640×480 H264 branch via the URL query.
    let url = whep_url(source_id, profile_mode_is_compat());
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
    // #431: 204 No Content = the source is configured but not currently
    // producing — an expected transient state, not an error. Report it as
    // NotProducing (Ok(None)) so the loop backs off quietly; never log it.
    if resp.status() == 204 {
        return Ok(None);
    }
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
    Ok(Some((answer_text, resource_url)))
}

#[cfg(test)]
mod tests {
    use super::reconnect_backoff_for_watchdog_step as backoff;

    /// #369: the watchdog-triggered reconnect path MUST back off before
    /// re-POSTing WHEP — even on the FIRST retry. A source that connects (DTLS
    /// ok) but never decodes a frame trips the watchdog every ~12s; with an
    /// immediate (0ms) first retry it re-POSTs with no delay each cycle,
    /// creating + tearing down a server-side WHEP session every time. The
    /// first watchdog reconnect must wait at least the base 500ms.
    #[test]
    fn first_watchdog_reconnect_is_not_immediate() {
        assert!(
            backoff(0) >= 500,
            "first watchdog reconnect must back off >=500ms, got {}ms (#369: \
             Ok-branch reconnect was immediate)",
            backoff(0)
        );
    }

    /// The backoff grows then CAPS at 5s — a persistently-broken source settles
    /// into one reconnect every 5s, never a tight no-delay spiral and never an
    /// unbounded delay.
    #[test]
    fn backoff_is_monotonic_then_capped_at_5s() {
        let mut prev = backoff(0);
        assert!(prev > 0);
        for step in 1..10 {
            let cur = backoff(step);
            assert!(cur >= prev, "backoff must not decrease at step {step}");
            assert!(
                cur <= 5000,
                "backoff must cap at 5000ms, got {cur} at step {step}"
            );
            prev = cur;
        }
        // Well past the schedule length it stays capped, not panicking/0.
        assert_eq!(
            backoff(100),
            5000,
            "backoff must stay capped at 5s for large steps"
        );
        // The escalation is real: a later step is strictly slower than step 0.
        assert!(
            backoff(3) > backoff(0),
            "backoff must escalate across steps"
        );
    }

    /// #369 reset rule: a session that lived past the healthy threshold (it was
    /// clearly decoding) resets the backoff so its reconnect is prompt; one that
    /// died within the no-decode window (connect-but-never-decode) does NOT
    /// reset, so the backoff keeps escalating toward the 5s cap.
    #[test]
    fn backoff_resets_only_for_healthy_sessions() {
        use super::{should_reset_backoff, HEALTHY_SESSION_MS};
        // A connect-but-never-decode source drops ~10-15s in — do NOT reset.
        assert!(!should_reset_backoff(0.0));
        assert!(!should_reset_backoff(12_000.0));
        assert!(!should_reset_backoff(HEALTHY_SESSION_MS - 1.0));
        // A long-lived session that blipped — reset (prompt reconnect).
        assert!(should_reset_backoff(HEALTHY_SESSION_MS));
        assert!(should_reset_backoff(300_000.0));
    }
}
