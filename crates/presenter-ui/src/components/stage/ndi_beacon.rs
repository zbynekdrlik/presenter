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

use super::ndi_frame_stats::{snapshot_present_gaps, FrameStats};
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
/// the spawned task.
pub(crate) fn post_stats_beacon(pc: &RtcPeerConnection, source_id: &str, stats: &FrameStats) {
    let (max_gap, over100, fps) = snapshot_present_gaps(stats);
    let pc = pc.clone();
    let source_id = source_id.to_string();
    spawn_local(async move {
        if let Ok(report) = JsFuture::from(pc.get_stats()).await {
            post_client_stats(&source_id, &report, max_gap, over100, fps).await;
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
) {
    tick_count.set(tick_count.get().wrapping_add(1));
    if tick_count.get() % 15 != 0 {
        return;
    }
    post_stats_beacon(pc, source_id, stats);
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
async fn post_client_stats(
    source_id: &str,
    report: &JsValue,
    max_present_gap_ms: f64,
    present_gaps_over100: u32,
    presented_fps: Option<f64>,
) {
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
        // Which stream profile this display requested ("default"/"compat").
        // The server serves ONE 720p H264 stream regardless of this value
        // (see `StreamProfile::from_query`); it is reported only to record
        // which watchdog mode the display was in when it sent this beacon —
        // there is no 640×480 / VP8 branch.
        "profile": profile_mode_name(profile_mode_is_compat()),
        "screen": screen,
        "framesDecoded": frames_decoded.as_f64(),
        "fps": fps.as_f64(),
        "jitterBufferMs": jitter_buffer_ms,
        "freezeCount": freeze_count.as_f64(),
        "framesDropped": frames_dropped.as_f64(),
        // Render-side presentation-cadence metrics for this beacon interval
        // (the decode-side fields above can't see a frame presented late).
        "maxPresentGapMs": max_present_gap_ms,
        "presentGapsOver100": present_gaps_over100,
        "presentedFps": presented_fps,
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
