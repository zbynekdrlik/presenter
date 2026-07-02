//! Client-side stats beacons posted to `/ndi/client-stats` for per-TV health
//! attribution.
//!
//! Two beacon channels (both fire-and-forget — a beacon must NEVER disturb
//! playback): a frame-count-driven one from the rVFC observer (reliable on
//! TVs whose setInterval is throttled) and a tick-driven one from the health
//! ticker. Both call `post_stats_beacon`, which snapshots the present-gap
//! accumulators synchronously and then POSTs the getStats summary on a spawned
//! task. Split out of `ndi_watchdog.rs` to keep that file under the size cap
//! (#418).

use std::cell::Cell;

use leptos::wasm_bindgen::{JsCast, JsValue};
use leptos::web_sys::RtcPeerConnection;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use super::ndi_frame_stats::{snapshot_present_gaps, DroppedFramesSetter, FrameStats};
use super::ndi_profile::{local_storage, profile_mode_is_compat, profile_mode_name};

/// localStorage key for the persistent per-display identity used in stats
/// beacons (per-TV health attribution server-side).
const DISPLAY_ID_KEY: &str = "ndiDisplayId";

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

/// Sample `pc.getStats()` and POST a beacon. Fire-and-forget; the beacon
/// must never disturb playback.
///
/// The present-gap accumulators are snapshotted-and-reset SYNCHRONOUSLY here
/// (before the async getStats), so each beacon reports exactly the interval
/// since the previous beacon — even though the actual POST happens later on
/// the spawned task. The smoothed true latency and the clock offset/RTT
/// (#514) are likewise sampled synchronously so they describe THIS beacon's
/// moment, not the post-await one.
pub(crate) fn post_stats_beacon(
    pc: &RtcPeerConnection,
    source_id: &str,
    stats: &FrameStats,
    clock_offset: Option<(f64, f64)>,
    dropped_frames_setter: Option<DroppedFramesSetter>,
) {
    let (max_gap, over100, fps) = snapshot_present_gaps(stats);
    let video_latency_ms = stats.video_latency_ms.get();
    let pc = pc.clone();
    let source_id = source_id.to_string();
    spawn_local(async move {
        if let Ok(report) = JsFuture::from(pc.get_stats()).await {
            post_client_stats(
                &source_id,
                &report,
                max_gap,
                over100,
                fps,
                video_latency_ms,
                clock_offset,
                dropped_frames_setter,
            )
            .await;
        }
    });
}

/// Every 15th watchdog tick (~15s at 1s ticks — slower on throttled TVs,
/// where the rVFC frame-count beacon is the reliable channel instead),
/// post a stats beacon for `source_id`.
pub(crate) fn maybe_post_beacon(
    tick_count: &Cell<u32>,
    pc: &RtcPeerConnection,
    source_id: &str,
    stats: &FrameStats,
    clock_offset: Option<(f64, f64)>,
    dropped_frames_setter: Option<DroppedFramesSetter>,
) {
    tick_count.set(tick_count.get().wrapping_add(1));
    if tick_count.get() % 15 != 0 {
        return;
    }
    post_stats_beacon(pc, source_id, stats, clock_offset, dropped_frames_setter);
}

/// Extract inbound-video stats from an RtcStatsReport (a JS Map) and POST a
/// compact summary to /ndi/client-stats. Fire-and-forget; errors ignored —
/// the beacon must never disturb playback.
///
/// `max_present_gap_ms` / `present_gaps_over100` / `presented_fps` are the
/// render-side presentation-cadence metrics for the interval since the last
/// beacon (already snapshotted-and-reset by the caller). They sit alongside
/// the decode-side getStats fields so a reader can tell a frame that decoded
/// on time but reached the screen late from a genuine decode stall.
/// The inbound-video getStats fields a beacon reports, pulled out of the
/// RtcStatsReport map in one pass (`extract_inbound_video`). Split from
/// `post_client_stats` to keep that function under the 120-line fn cap.
#[derive(Default)]
struct InboundVideoStats {
    frames_decoded: Option<f64>,
    fps: Option<f64>,
    jitter_buffer_ms: Option<f64>,
    freeze_count: Option<f64>,
    frames_dropped: Option<f64>,
    codec: Option<String>,
    // #509 (T0) device-capability probe: the field T4's true server→display
    // metric would read, plus the inbound-rtp report's OWN Unix-epoch timestamp
    // from the SAME snapshot (the epoch reference the playout value is checked
    // against server-side via `classify_playout`). Absent → undefined →
    // `as_f64()` None → null on the wire; a literal 0 → Some(0.0) (the
    // pre-first-SR gotcha), distinguished server-side.
    estimated_playout: Option<f64>,
    report_ts: Option<f64>,
    // #525 (step 1) device-decode probe: is this display decoding H264 in
    // HARDWARE or falling back to SOFTWARE? A software decode on a "smart TV"
    // box is the prime suspect for a large decode+present residual (SD1 read
    // ~90ms above sd2-4). Purely diagnostic — read-only, no playback change.
    decoder_implementation: Option<String>,
    power_efficient_decoder: Option<bool>,
}

/// One pass over the RtcStatsReport map: the inbound-rtp video entry's fields,
/// the negotiated codec (via the codecId → "codec"-entry mimeType lookup), and
/// the average jitter-buffer depth in ms (cumulative delay / emitted count).
fn extract_inbound_video(report: &JsValue) -> InboundVideoStats {
    let mut out = InboundVideoStats::default();
    let mut jb_delay = None;
    let mut jb_emitted = None;
    let mut codec_id = None;
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
                out.frames_decoded = get("framesDecoded").as_f64();
                out.fps = get("framesPerSecond").as_f64();
                jb_delay = get("jitterBufferDelay").as_f64();
                jb_emitted = get("jitterBufferEmittedCount").as_f64();
                out.freeze_count = get("freezeCount").as_f64();
                out.frames_dropped = get("framesDropped").as_f64();
                codec_id = get("codecId").as_string();
                out.estimated_playout = get("estimatedPlayoutTimestamp").as_f64();
                out.report_ts = get("timestamp").as_f64();
                out.decoder_implementation = get("decoderImplementation").as_string();
                out.power_efficient_decoder = get("powerEfficientDecoder").as_bool();
            }
        }
    }
    // jitterBufferDelay is a cumulative sum of seconds each emitted frame
    // spent in the buffer; divide by the emitted count for the average, in ms.
    out.jitter_buffer_ms = match (jb_delay, jb_emitted) {
        (Some(d), Some(n)) if n > 0.0 => Some(d / n * 1000.0),
        _ => None,
    };
    out.codec = codec_id
        .map(|id| map.get(&JsValue::from_str(&id)))
        .and_then(|entry| js_sys::Reflect::get(&entry, &JsValue::from_str("mimeType")).ok())
        .and_then(|v| v.as_string());
    out
}

/// #523: push this beacon's dropped-frame + freeze counts to the on-screen
/// readout. `None` only when the browser's getStats reports NEITHER field
/// (honest "no data" rather than a misleading 0); a report with at least one
/// of the two fields present treats the other as 0 (freezeCount and
/// framesDropped are both cumulative counters that start at 0, so an absent
/// sibling field alongside a present one is "not yet incremented", not
/// "unknown"). Split out of `post_client_stats` to keep that function under
/// the 80-line size-warning threshold.
fn notify_dropped_frames(inbound: &InboundVideoStats, setter: &Option<DroppedFramesSetter>) {
    let Some(setter) = setter else { return };
    let counts = match (inbound.frames_dropped, inbound.freeze_count) {
        (None, None) => None,
        (dropped, freeze) => Some((
            dropped.unwrap_or(0.0).round() as u32,
            freeze.unwrap_or(0.0).round() as u32,
        )),
    };
    setter(counts);
}

#[allow(clippy::too_many_arguments)]
async fn post_client_stats(
    source_id: &str,
    report: &JsValue,
    max_present_gap_ms: f64,
    present_gaps_over100: u32,
    presented_fps: Option<f64>,
    video_latency_ms: Option<f64>,
    clock_offset: Option<(f64, f64)>,
    dropped_frames_setter: Option<DroppedFramesSetter>,
) {
    let inbound = extract_inbound_video(report);
    notify_dropped_frames(&inbound, &dropped_frames_setter);

    // #509 (T0): full userAgent — the exact WebView/Chrome version per real
    // stage TV, which decides whether the playout field is available there.
    let user_agent = leptos::web_sys::window()
        .map(|w| w.navigator())
        .and_then(|n| n.user_agent().ok());

    // Physical screen size, for telling TV models apart in the logs.
    let screen = leptos::web_sys::window()
        .and_then(|w| w.screen().ok())
        .and_then(|s| match (s.width(), s.height()) {
            (Ok(w), Ok(h)) => Some(format!("{w}x{h}")),
            _ => None,
        });

    let body = serde_json::json!({
        "sourceId": source_id,
        "displayId": display_id(),
        "codec": inbound.codec,
        // Which stream profile this display requested ("default"/"compat").
        // The server serves ONE 720p H264 stream regardless of this value
        // (see `StreamProfile::from_query`); it is reported only to record
        // which watchdog mode the display was in when it sent this beacon —
        // there is no 640×480 / VP8 branch.
        "profile": profile_mode_name(profile_mode_is_compat()),
        "screen": screen,
        "framesDecoded": inbound.frames_decoded,
        "fps": inbound.fps,
        "jitterBufferMs": inbound.jitter_buffer_ms,
        "freezeCount": inbound.freeze_count,
        "framesDropped": inbound.frames_dropped,
        // Render-side presentation-cadence metrics for this beacon interval
        // (the decode-side fields above can't see a frame presented late).
        "maxPresentGapMs": max_present_gap_ms,
        "presentGapsOver100": present_gaps_over100,
        "presentedFps": presented_fps,
        // #509 (T0) device-capability probe fields.
        "estimatedPlayoutTimestamp": inbound.estimated_playout,
        "reportTimestamp": inbound.report_ts,
        // #525 (step 1) device-decode probe — is this display's H264 decode
        // HARDWARE or SOFTWARE? Diagnostic only; read live to find where SD1's
        // decode+present residual actually lives before any playback change.
        "decoderImplementation": inbound.decoder_implementation,
        "powerEfficientDecoder": inbound.power_efficient_decoder,
        "userAgent": user_agent,
        // #514 observability: the smoothed TRUE server→display latency this
        // display currently SHOWS (#512; null = n/a) plus the /ndi/time clock
        // offset + RTT it was built from (#510) — for server-side cross-device
        // correlation in one clock domain.
        "videoLatencyMs": video_latency_ms,
        "clockOffsetMs": clock_offset.map(|(o, _)| o),
        "clockRttMs": clock_offset.map(|(_, r)| r),
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

#[cfg(test)]
mod tests {
    use super::{notify_dropped_frames, DroppedFramesSetter, InboundVideoStats};
    use std::cell::Cell;
    use std::rc::Rc;

    type CapturedCounts = Rc<Cell<Option<(u32, u32)>>>;

    fn capturing_setter() -> (DroppedFramesSetter, CapturedCounts) {
        let captured = Rc::new(Cell::new(None));
        let setter = {
            let captured = Rc::clone(&captured);
            Rc::new(move |v: Option<(u32, u32)>| captured.set(v)) as DroppedFramesSetter
        };
        (setter, captured)
    }

    #[test]
    fn no_data_from_getstats_reports_none_not_a_fabricated_zero() {
        let (setter, captured) = capturing_setter();
        let inbound = InboundVideoStats::default();
        notify_dropped_frames(&inbound, &Some(setter));
        assert_eq!(captured.get(), None);
    }

    #[test]
    fn dropped_present_freeze_absent_treats_freeze_as_zero() {
        // Both counters are cumulative and start at 0 — a present dropped-count
        // with no freeze field yet means "0 freezes so far", not "unknown".
        let (setter, captured) = capturing_setter();
        let inbound = InboundVideoStats {
            frames_dropped: Some(128.0),
            ..Default::default()
        };
        notify_dropped_frames(&inbound, &Some(setter));
        assert_eq!(captured.get(), Some((128, 0)));
    }

    #[test]
    fn both_present_round_to_nearest_integer() {
        let (setter, captured) = capturing_setter();
        let inbound = InboundVideoStats {
            frames_dropped: Some(128.0),
            freeze_count: Some(2.0),
            ..Default::default()
        };
        notify_dropped_frames(&inbound, &Some(setter));
        assert_eq!(captured.get(), Some((128, 2)));
    }
}
