//! Per-consumer WebRTC negotiation helpers: the SDP offer/answer/SLD dance,
//! payload-type alignment, ICE-gathering wait, and the media-caps wait that
//! guarantees the answer announces the send SSRC. Split from `consumers.rs`
//! (which owns the consumer lifecycle) to keep both files focused.

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_webrtc as gst_webrtc;

/// Wait (bounded, event-driven) until the webrtcbin sink pad has CURRENT CAPS
/// that include the payloader's `ssrc` field — i.e. real media caps, not just
/// the link-filter caps. Called between connecting the consumer to the
/// StreamProducer and create-answer, so the answer SDP always announces the
/// send SSRC. Without this, create-answer can win the race against media
/// arrival: the answer then has NO `a=ssrc` line and the browser DROPS every
/// RTP packet as un-demuxable — transport bytes climb while inbound-rtp stays
/// at zero: connected, but black. (Observed deterministically for stragglers,
/// whose first buffer is keyframe-gated by the StreamProducer.)
///
/// Media normally arrives within one GOP (≤1s; usually ~50ms thanks to the
/// producer's force-keyunit request). The 5s bound covers a source that is
/// momentarily not delivering frames; on timeout we proceed with a generic
/// answer (same behavior as before this fix) and log loudly.
pub(super) fn await_media_caps(webrtcbin: &gst::Element, session_id: &str) {
    let Some(pad) = webrtcbin.sink_pads().into_iter().next() else {
        tracing::warn!(session_id = %session_id, "webrtcbin has no sink pad; skipping media-caps wait");
        return;
    };
    let has_media_caps = |p: &gst::Pad| {
        p.current_caps()
            .and_then(|c| c.structure(0).map(|s| s.has_field("ssrc")))
            .unwrap_or(false)
    };
    if has_media_caps(&pad) {
        return;
    }
    // Event-driven wait: a probe signals when a CAPS event passes the pad.
    let (caps_tx, caps_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let probe_id = pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
        if let Some(gst::PadProbeData::Event(ev)) = &info.data {
            if ev.type_() == gst::EventType::Caps {
                let _ = caps_tx.try_send(());
            }
        }
        gst::PadProbeReturn::Ok
    });
    // Cover the race where the caps event passed between the check and the
    // probe install.
    if !has_media_caps(&pad)
        && caps_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_err()
    {
        tracing::warn!(
            session_id = %session_id,
            "media caps did not reach webrtcbin within 5s (source not \
             delivering frames?); the answer will carry no a=ssrc and the \
             browser may fail to demux the stream until it reconnects"
        );
    }
    if let Some(id) = probe_id {
        pad.remove_probe(id);
    }
}

/// Perform the SDP handshake on `webrtcbin`: set-remote-description (the
/// browser's offer), create-answer, set-local-description (our answer). Each
/// step waits on its GStreamer promise; a timeout is propagated as an error
/// (set-local is observability-only — the final answer is re-read later).
pub(super) fn negotiate_sdp(
    webrtcbin: &gst::Element,
    offer_desc: &gst_webrtc::WebRTCSessionDescription,
) -> Result<()> {
    let (remote_desc_tx, remote_desc_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let promise = gst::Promise::with_change_func(move |_reply| {
        let _ = remote_desc_tx.send(());
    });
    webrtcbin.emit_by_name::<()>("set-remote-description", &[offer_desc, &promise]);
    remote_desc_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .context("set-remote-description promise timed out or sender dropped")?;

    let (answer_sdp_tx, answer_sdp_rx) =
        std::sync::mpsc::sync_channel::<Option<gst_webrtc::WebRTCSessionDescription>>(1);
    let promise = gst::Promise::with_change_func(move |reply| {
        let answer = reply
            .ok()
            .and_then(|r| r)
            .and_then(|r| r.value("answer").ok())
            .and_then(|v| v.get::<gst_webrtc::WebRTCSessionDescription>().ok());
        let _ = answer_sdp_tx.send(answer);
    });
    webrtcbin.emit_by_name::<()>("create-answer", &[&None::<gst::Structure>, &promise]);
    let answer_desc = answer_sdp_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .context("create-answer promise timed out")?
        .ok_or_else(|| anyhow!("create-answer returned no answer"))?;

    let (sld_tx, sld_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let sld_promise = gst::Promise::with_change_func(move |_reply| {
        let _ = sld_tx.try_send(());
    });
    webrtcbin.emit_by_name::<()>("set-local-description", &[&answer_desc, &sld_promise]);
    if sld_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .is_err()
    {
        // The payload-type read in align_payload_type depends on the
        // local-description being in place; the final WHEP answer is re-read
        // after the ICE-gather wait, so this is observability, not a failure.
        tracing::warn!("set-local-description did not confirm within 2s; payload-type alignment may read a stale SDP");
    }
    Ok(())
}

/// Align the payloader's RTP payload type with the one webrtcbin negotiated
/// in the answer. The browser assigns a dynamic PT to the codec (Chrome uses
/// e.g. 103 for H264, 96 for VP8) and webrtcbin answers with THAT pt — but
/// the payloaders default to pt=96, so their caps wouldn't match webrtcbin's
/// negotiated sink and ZERO RTP would flow (connected, but black).
/// Codec-aware: `encoding_name` is the RTP encoding-name of the codec this
/// consumer's profile streams ("H264" or "VP8" — see
/// `StreamProfile::encoding_name`).
pub(super) fn align_payload_type(
    webrtcbin: &gst::Element,
    payloader: &gst::Element,
    encoding_name: &str,
    session_id: &str,
) {
    let local_sdp = webrtcbin
        .property::<gst_webrtc::WebRTCSessionDescription>("local-description")
        .sdp()
        .as_text()
        .unwrap_or_default();
    // parse_rtpmap_payload_type returns the FIRST `a=rtpmap:<pt> <codec>/…`.
    // The pay→webrtc caps filter omits `payload`, so a multi-pt mismatch
    // can't stall.
    let prefix = format!("{}/", encoding_name.to_ascii_uppercase());
    if let Some(pt) = parse_rtpmap_payload_type(&local_sdp, &prefix) {
        payloader.set_property("pt", pt);
        tracing::debug!(
            session_id = %session_id,
            pt,
            codec = encoding_name,
            "aligned payloader pt to negotiated payload type"
        );
    } else {
        tracing::warn!(
            session_id = %session_id,
            codec = encoding_name,
            "could not find negotiated payload type in answer SDP; \
             leaving payloader at default pt (media may not flow)"
        );
    }
}

/// Non-trickle ICE: wait for candidate gathering to COMPLETE so the returned
/// local-description SDP carries `a=candidate` lines. LAN-only (host
/// candidates, no STUN/TURN) → completes well under a second. Bounded by 5 s.
pub(super) fn await_ice_gathering(webrtcbin: &gst::Element, session_id: &str) {
    let (gather_tx, gather_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let gather_tx_signal = gather_tx.clone();
    let _ = webrtcbin.connect_notify(Some("ice-gathering-state"), move |wb, _| {
        let st = wb.property::<gst_webrtc::WebRTCICEGatheringState>("ice-gathering-state");
        if st == gst_webrtc::WebRTCICEGatheringState::Complete {
            let _ = gather_tx_signal.try_send(());
        }
    });
    // Cover the race where gathering already completed before the handler.
    if webrtcbin.property::<gst_webrtc::WebRTCICEGatheringState>("ice-gathering-state")
        == gst_webrtc::WebRTCICEGatheringState::Complete
    {
        let _ = gather_tx.try_send(());
    }
    if gather_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .is_err()
    {
        tracing::warn!(
            session_id = %session_id,
            "ICE gathering did not reach Complete within 5s; \
             returning answer with whatever candidates were gathered"
        );
    }
}

/// Extract the H264 RTP payload type negotiated in an SDP answer.
///
/// Scans for the first `a=rtpmap:<pt> H264/90000` line and returns `<pt>`.
/// WebRTC assigns H264 a *dynamic* payload type (96–127) that varies per
/// browser (Chrome commonly picks 102/103/108…), so the per-consumer
/// `rtph264pay` must be told this value or its RTP caps won't match
/// webrtcbin's negotiated sink and no media flows.
pub(crate) fn parse_h264_payload_type(sdp: &str) -> Option<u32> {
    parse_rtpmap_payload_type(sdp, "H264/")
}

/// VP8 mirror of [`parse_h264_payload_type`]: the payload type of the first
/// `a=rtpmap:<pt> VP8/90000` line. Used by the compat profile (realtime-VP8
/// pivot): a `?profile=compat` consumer is served the VP8 branch, and its
/// per-consumer `rtpvp8pay` must be seated on the browser's dynamic VP8 pt
/// for media to flow. Every browser's default offer carries VP8.
pub(crate) fn parse_vp8_payload_type(sdp: &str) -> Option<u32> {
    parse_rtpmap_payload_type(sdp, "VP8/")
}

/// Shared rtpmap scanner: payload type of the first `a=rtpmap:<pt>
/// <codec_prefix>…` line (codec match is case-insensitive). `codec_prefix`
/// MUST include the trailing `/` (e.g. `"H264/"`, `"VP8/"`) so `VP8` can
/// never match a `VP9/90000` rtpmap.
fn parse_rtpmap_payload_type(sdp: &str, codec_prefix: &str) -> Option<u32> {
    for line in sdp.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            let mut parts = rest.splitn(2, ' ');
            let pt = parts.next()?;
            let codec = parts.next().unwrap_or("");
            if codec.to_ascii_uppercase().starts_with(codec_prefix) {
                if let Ok(pt) = pt.trim().parse::<u32>() {
                    return Some(pt);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod pt_parse_tests {
    use super::{parse_h264_payload_type, parse_vp8_payload_type};

    #[test]
    fn finds_h264_payload_type_when_other_codecs_listed_first() {
        // Chrome's default offer lists VP8/VP9 before H264 — the scanner
        // must find H264's pt regardless of rtpmap order.
        let sdp = "m=video 9 UDP/TLS/RTP/SAVPF 100 98 103\r\n\
                   a=rtpmap:100 VP8/90000\r\n\
                   a=rtpmap:98 VP9/90000\r\n\
                   a=rtpmap:103 H264/90000\r\n";
        assert_eq!(parse_h264_payload_type(sdp), Some(103));
    }

    #[test]
    fn finds_dynamic_h264_payload_type() {
        let sdp = "v=0\r\n\
                   m=video 9 UDP/TLS/RTP/SAVPF 103 104\r\n\
                   a=rtpmap:103 H264/90000\r\n\
                   a=fmtp:103 packetization-mode=1\r\n\
                   a=rtpmap:104 rtx/90000\r\n";
        assert_eq!(parse_h264_payload_type(sdp), Some(103));
    }

    #[test]
    fn handles_alternate_pt_value() {
        let sdp = "m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 H264/90000\r\n";
        assert_eq!(parse_h264_payload_type(sdp), Some(96));
    }

    #[test]
    fn returns_none_without_h264() {
        let sdp = "m=video 9 UDP/TLS/RTP/SAVPF 100\r\na=rtpmap:100 VP8/90000\r\n";
        assert_eq!(parse_h264_payload_type(sdp), None);
    }

    #[test]
    fn finds_vp8_payload_type_in_chrome_default_offer() {
        // Chrome's default offer: VP8 first, then VP9/H264 — the compat
        // consumer's rtpvp8pay must be seated on VP8's dynamic pt.
        let sdp = "m=video 9 UDP/TLS/RTP/SAVPF 96 98 102\r\n\
                   a=rtpmap:96 VP8/90000\r\n\
                   a=rtpmap:98 VP9/90000\r\n\
                   a=rtpmap:102 H264/90000\r\n";
        assert_eq!(parse_vp8_payload_type(sdp), Some(96));
    }

    #[test]
    fn vp8_prefix_does_not_match_vp9() {
        // The trailing '/' in the scanner prefix is load-bearing: an offer
        // with ONLY VP9 must not satisfy the VP8 lookup.
        let sdp = "m=video 9 UDP/TLS/RTP/SAVPF 98\r\na=rtpmap:98 VP9/90000\r\n";
        assert_eq!(parse_vp8_payload_type(sdp), None);
    }

    #[test]
    fn returns_none_without_vp8() {
        let sdp = "m=video 9 UDP/TLS/RTP/SAVPF 103\r\na=rtpmap:103 H264/90000\r\n";
        assert_eq!(parse_vp8_payload_type(sdp), None);
    }
}
