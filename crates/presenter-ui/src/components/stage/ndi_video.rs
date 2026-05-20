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
    HtmlVideoElement, MediaStream, RtcConfiguration, RtcPeerConnection, RtcRtpTransceiverDirection,
    RtcRtpTransceiverInit, RtcSdpType, RtcSessionDescriptionInit, RtcTrackEvent,
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

/// Build the WHEP endpoint URL for a given source.
pub fn whep_url(source_id: &str) -> String {
    format!("/ndi/whep/{source_id}")
}

#[component]
pub fn NdiVideo(source_id: String, #[prop(optional)] class: Option<&'static str>) -> impl IntoView {
    let video_ref = NodeRef::<leptos::html::Video>::new();
    let source_id_for_effect = source_id.clone();
    // Holds the full WHEP session (pc + resource URL) so we can DELETE the
    // session on the server when the component unmounts. Without DELETE the
    // server-side webrtcsink accumulates sessions forever (every browser
    // navigation = one session) and breaks after enough buildup.
    let session_holder: StoredValue<Option<WhepSession>> = StoredValue::new(None);
    // Cancellation flag covering the race where the component unmounts BEFORE
    // `connect_whep` resolves.
    let cancelled = Arc::new(AtomicBool::new(false));

    let cancelled_for_effect = Arc::clone(&cancelled);
    Effect::new(move |_| {
        let Some(video) = video_ref.get() else { return };
        let source_id = source_id_for_effect.clone();
        let cancelled = Arc::clone(&cancelled_for_effect);
        spawn_local(async move {
            match connect_whep(&video, &source_id).await {
                Ok(session) => {
                    if cancelled.load(Ordering::Acquire) {
                        // Component unmounted before connect_whep finished —
                        // synchronously fire DELETE and close pc. Same
                        // mechanism as on_cleanup below.
                        if let Some(url) = &session.resource_url {
                            dispatch_delete(url);
                        }
                        session.pc.close();
                    } else {
                        // Also wire a window pagehide listener: some browsers
                        // (and Playwright's page.goto()) tear down the JS
                        // context before Leptos's on_cleanup runs. pagehide
                        // fires earlier in the unload sequence and gives us a
                        // chance to dispatch the DELETE while the window is
                        // still alive.
                        install_pagehide_teardown(&session);
                        session_holder.set_value(Some(session));
                    }
                }
                Err(e) => {
                    leptos::logging::error!("WHEP connect for {source_id} failed: {e:?}");
                }
            }
        });
    });

    let cancelled_for_cleanup = Arc::clone(&cancelled);
    on_cleanup(move || {
        cancelled_for_cleanup.store(true, Ordering::Release);
        let session = session_holder.try_update_value(|opt| opt.take()).flatten();
        if let Some(session) = session {
            if let Some(url) = &session.resource_url {
                dispatch_delete(url);
            }
            session.pc.close();
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

/// Fire-and-forget DELETE to the WHEP session resource. SYNCHRONOUS dispatch —
/// we do NOT await the future. spawn_local-wrapped fetches do not start when
/// called from a page-unload context (the microtask queue is destroyed before
/// the future polls). Calling `window.fetch_with_request` directly enqueues
/// the request immediately; `keepalive: true` keeps it alive after unload.
fn dispatch_delete(url: &str) {
    let init = leptos::web_sys::RequestInit::new();
    init.set_method("DELETE");
    let _ = js_sys::Reflect::set(&init, &"keepalive".into(), &JsValue::TRUE);
    if let Ok(request) = leptos::web_sys::Request::new_with_str_and_init(url, &init) {
        if let Some(window) = leptos::web_sys::window() {
            // Promise dropped intentionally — keepalive carries it through.
            let _ = window.fetch_with_request(&request);
        }
    }
}

/// Install a `pagehide` window listener that fires DELETE if the page is
/// being unloaded. Some browsers (and Playwright's page.goto navigation)
/// tear down the JS context before Leptos's `on_cleanup` runs; pagehide
/// fires earlier in the unload sequence so the DELETE makes it out the door.
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
    // Leak the closure into JS — it must outlive Rust scope to be callable
    // from the pagehide event. The listener fires at most once per session
    // (page unloads exactly once), so the leak is bounded.
    cb.forget();
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
            video_clone.set_src_object(Some(&s));
            // `autoplay muted playsinline` HTML attributes alone are NOT
            // enough — Chrome's autoplay policy still blocks playback when
            // the <video> element is mounted via DOM mutation (Leptos
            // creates it reactively) on a domain the user has never
            // interacted with. The element ends up in `paused=true` state
            // after `srcObject` is set, and the user has to right-click ->
            // "Show all controls" -> Play to actually see video. (Playwright
            // disables the autoplay policy by default, which is why the
            // server-side TDD never caught this.)
            //
            // Calling `.play()` explicitly after setting srcObject is the
            // documented workaround — Chrome allows programmatic play() on
            // muted + playsinline video without user interaction. The
            // returned Promise rejects only if the policy STILL blocks
            // (e.g. user has explicitly disabled autoplay site-wide); we
            // log and continue rather than panic since there's no
            // automatic recovery from that state anyway.
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
    let offer_sdp = js_sys::Reflect::get(&offer_init, &"sdp".into())?
        .as_string()
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
