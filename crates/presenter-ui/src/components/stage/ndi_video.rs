//! NdiVideo — WHEP-subscribing `<video>` element for one NDI source.
//!
//! Each `<NdiVideo>` mounts an HTMLVideoElement and connects to
//! `/ndi/whep/<source_id>` via the WHEP protocol. The browser handles
//! ICE/DTLS/SRTP/jitter-buffer/AV-sync natively. WASM is signaling glue only.

use leptos::prelude::*;
use leptos::wasm_bindgen::{closure::Closure, JsCast, JsValue};
use leptos::web_sys::{
    HtmlVideoElement, MediaStream, RtcConfiguration, RtcPeerConnection,
    RtcRtpTransceiverDirection, RtcRtpTransceiverInit, RtcSdpType,
    RtcSessionDescriptionInit, RtcTrackEvent,
};
use wasm_bindgen_futures::{spawn_local, JsFuture};

/// Build the WHEP endpoint URL for a given source.
pub fn whep_url(source_id: &str) -> String {
    format!("/ndi/whep/{source_id}")
}

#[component]
pub fn NdiVideo(
    source_id: String,
    #[prop(optional)] class: Option<&'static str>,
) -> impl IntoView {
    let video_ref = NodeRef::<leptos::html::Video>::new();
    let source_id_for_effect = source_id.clone();
    // Hold the RtcPeerConnection so we can close it on unmount. WebRTC
    // peer connections are NOT closed when the JsValue is dropped — the
    // browser keeps them open until explicit `close()`. Without this the
    // whepserversink leaks ICE sessions every time the layout switches.
    let pc_holder: StoredValue<Option<RtcPeerConnection>> = StoredValue::new(None);

    Effect::new(move |_| {
        let Some(video) = video_ref.get() else { return };
        let source_id = source_id_for_effect.clone();
        spawn_local(async move {
            match connect_whep(&video, &source_id).await {
                Ok(pc) => {
                    pc_holder.set_value(Some(pc));
                }
                Err(e) => {
                    leptos::logging::error!("WHEP connect for {source_id} failed: {e:?}");
                }
            }
        });
    });

    on_cleanup(move || {
        if let Some(pc) = pc_holder.try_get_value().flatten() {
            pc.close();
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

async fn connect_whep(
    video: &HtmlVideoElement,
    source_id: &str,
) -> Result<RtcPeerConnection, JsValue> {
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
    let answer_text = JsFuture::from(resp.text()?)
        .await?
        .as_string()
        .unwrap_or_default();
    let answer = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
    answer.set_sdp(&answer_text);
    JsFuture::from(pc.set_remote_description(&answer)).await?;
    Ok(pc)
}
