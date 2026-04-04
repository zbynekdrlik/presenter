use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::components::stage::status_bar::StatusBar;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

#[component]
pub fn NdiFullscreen(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_status = ctx.ndi_status;
    let video_ref = NodeRef::<leptos::html::Video>::new();

    // When ndi_active becomes true, connect via WHEP
    {
        let video_ref = video_ref.clone();
        Effect::new(move |_| {
            let active = ndi_active.get();
            if active {
                let video_ref = video_ref.clone();
                leptos::task::spawn_local(async move {
                    if let Err(e) = connect_whep(video_ref).await {
                        web_sys::console::error_1(&format!("WHEP connect failed: {e:?}").into());
                    }
                });
            }
        });
    }

    view! {
        <div class="stage-ndi">
            <video
                node_ref=video_ref
                class="stage-ndi__video"
                autoplay=true
                playsinline=true
                muted=true
            />

            <Show when=move || !ndi_active.get()>
                <div class="stage-ndi__placeholder">
                    "No video source configured"
                </div>
            </Show>

            <Show when=move || {
                let status = ndi_status.get();
                status == "disconnected" || status == "connecting"
            }>
                <div class="stage-ndi__overlay">
                    {move || {
                        let status = ndi_status.get();
                        if status == "disconnected" {
                            "Signal Lost — Reconnecting..."
                        } else if status == "connecting" {
                            "Connecting..."
                        } else {
                            ""
                        }
                    }}
                </div>
            </Show>

            <StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}

/// Connect to the WHEP endpoint and attach the resulting MediaStream to the video element.
async fn connect_whep(video_ref: NodeRef<leptos::html::Video>) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;

    // Create RTCPeerConnection
    let pc = web_sys::RtcPeerConnection::new()?;

    // Set up ontrack handler BEFORE creating offer (to not miss events)
    let video_ref_clone = video_ref.clone();
    let ontrack =
        Closure::<dyn FnMut(web_sys::RtcTrackEvent)>::new(move |ev: web_sys::RtcTrackEvent| {
            let streams = ev.streams();
            if streams.length() > 0 {
                let stream: web_sys::MediaStream = streams.get(0).unchecked_into();
                if let Some(video_el) = video_ref_clone.get() {
                    let html_video: &web_sys::HtmlVideoElement = &video_el;
                    let html_media: &web_sys::HtmlMediaElement = html_video.as_ref();
                    html_media.set_src_object(Some(&stream));
                    let _ = html_media.play();
                    web_sys::console::log_1(&"NDI: video track attached".into());
                }
            }
        });
    pc.set_ontrack(Some(ontrack.as_ref().unchecked_ref()));
    ontrack.forget();

    // Add transceiver for receiving video
    pc.add_transceiver_with_str("video");

    // Create SDP offer
    let offer = JsFuture::from(pc.create_offer()).await?;
    let offer_sdp = js_sys::Reflect::get(&offer, &"sdp".into())?
        .as_string()
        .ok_or("no sdp in offer")?;

    // Set local description and wait for ICE gathering to complete
    let mut offer_init = web_sys::RtcSessionDescriptionInit::new(web_sys::RtcSdpType::Offer);
    offer_init.set_sdp(&offer_sdp);
    JsFuture::from(pc.set_local_description(&offer_init)).await?;

    // Wait for ICE gathering to finish
    if pc.ice_gathering_state() != web_sys::RtcIceGatheringState::Complete {
        let pc_clone = pc.clone();
        let promise = js_sys::Promise::new(&mut move |resolve, _reject| {
            let resolve2 = resolve.clone();
            let pc2 = pc_clone.clone();
            let cb = Closure::<dyn FnMut()>::new(move || {
                if pc2.ice_gathering_state() == web_sys::RtcIceGatheringState::Complete {
                    let _ = resolve2.call0(&JsValue::NULL);
                }
            });
            pc_clone.set_onicegatheringstatechange(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        });
        JsFuture::from(promise).await?;
    }

    // Get the complete SDP with ICE candidates
    let local_desc = pc.local_description().ok_or("no local description")?;
    let complete_sdp = local_desc.sdp();

    web_sys::console::log_1(
        &format!("NDI: sending WHEP offer ({} bytes)", complete_sdp.len()).into(),
    );

    // POST to WHEP endpoint
    let request_init = web_sys::RequestInit::new();
    request_init.set_method("POST");
    request_init.set_body(&complete_sdp.into());

    let headers = web_sys::Headers::new()?;
    headers.set("Content-Type", "application/sdp")?;
    request_init.set_headers(&headers);

    let request = web_sys::Request::new_with_str_and_init("/ndi/whep", &request_init)?;
    let resp = JsFuture::from(window.fetch_with_request(&request)).await?;
    let resp: web_sys::Response = resp.dyn_into()?;

    if !resp.ok() {
        let status = resp.status();
        let body = JsFuture::from(resp.text()?)
            .await?
            .as_string()
            .unwrap_or_default();
        return Err(format!("WHEP returned {status}: {body}").into());
    }

    let answer_sdp = JsFuture::from(resp.text()?)
        .await?
        .as_string()
        .ok_or("no text in WHEP response")?;

    web_sys::console::log_1(&format!("NDI: got WHEP answer ({} bytes)", answer_sdp.len()).into());

    // Set remote description (SDP answer)
    let mut answer_init = web_sys::RtcSessionDescriptionInit::new(web_sys::RtcSdpType::Answer);
    answer_init.set_sdp(&answer_sdp);
    JsFuture::from(pc.set_remote_description(&answer_init)).await?;

    web_sys::console::log_1(&"NDI: WebRTC connection established".into());

    Ok(())
}
