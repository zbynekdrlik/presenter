//! Per-session client frame-stats reporter (#768 D6).
//!
//! A dedicated ~5s `setInterval` POSTs a compact frame-stats sample for THIS
//! WHEP session to `POST /ndi/sessions/{session_id}/client-stats`, so a
//! stuttering stage TV is visible server-side (in `GET /ndi/snapshot/{id}`)
//! without physical presence — the exact gap the 2026-09-13 incident exposed.
//!
//! Dedicated (not folded into the ~15s `ndi_beacon` path) for two reasons:
//! the boot-restore E2E needs a fresh sample within ~10s, and this reporter
//! reads the frame stats NON-DESTRUCTIVELY (its own cumulative-count window;
//! `max_present_gap_ms` / `frames_live` read without reset) so it never
//! disturbs the existing beacon's interval accumulators.
//!
//! Fire-and-forget: a failed POST is tolerated silently (no `console.warn`),
//! per the stage's zero-console-noise rule.

use std::cell::Cell;
use std::rc::Rc;

use leptos::wasm_bindgen::{closure::Closure, JsCast, JsValue};
use leptos::web_sys::RtcPeerConnection;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use super::ndi_beacon::extract_inbound_video;
use super::ndi_frame_stats::FrameStats;
use super::ndi_watchdog::now_ms;

/// Report cadence. ~5s keeps the snapshot fresh (the boot-restore E2E asserts a
/// live sample within ~10s) without adding meaningful load — one small POST per
/// session per interval.
const SESSION_STATS_INTERVAL_MS: i32 = 5_000;

/// Minimum window (ms) for a trustworthy presented-fps figure — below this the
/// frame delta is too small for a stable rate (mirrors `ndi_frame_stats`'s own
/// 1s floor). A tick shorter than this skips the report rather than send noise.
const MIN_FPS_WINDOW_MS: f64 = 1_000.0;

/// Extract the WHEP `session_id` (the last path segment) from a resource URL
/// like `http://host/ndi/whep/<source>/<session_id>` — the value the server
/// keys `POST /ndi/sessions/{session_id}/client-stats` on. Strips any query /
/// fragment first and rejects an empty trailing segment. Pure — host-testable.
pub(crate) fn session_id_from_resource_url(url: &str) -> Option<&str> {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .find(|segment| !segment.is_empty())
}

/// Presented frames/s over `[prev_count, count_now]` across `elapsed_ms`, or
/// `None` when the window is shorter than `MIN_FPS_WINDOW_MS` or the count went
/// backwards (a fresh session's counter reset). Pure — host-testable without a
/// clock.
pub(crate) fn session_presented_fps(
    count_now: u32,
    prev_count: u32,
    elapsed_ms: f64,
) -> Option<f64> {
    if elapsed_ms < MIN_FPS_WINDOW_MS || count_now < prev_count {
        return None;
    }
    Some(f64::from(count_now - prev_count) / (elapsed_ms / 1000.0))
}

/// Build the compact `POST /ndi/sessions/{id}/client-stats` body. Pure —
/// host-testable; camelCase keys match the server `NdiClientStatsReport` DTO.
pub(crate) fn build_client_stats_body(
    presented_fps: f64,
    max_present_gap_ms: f64,
    jitter_buffer_ms: Option<f64>,
    frames_decoded: f64,
    frames_live: bool,
) -> String {
    serde_json::json!({
        "presentedFps": presented_fps,
        "maxPresentGapMs": max_present_gap_ms,
        "jitterBufferMs": jitter_buffer_ms,
        "framesDecoded": frames_decoded,
        "framesLive": frames_live,
    })
    .to_string()
}

/// POST the compact sample to the session-keyed endpoint. Fire-and-forget; any
/// error (including an expired session's 404) is swallowed silently.
async fn post_session_stats(session_id: &str, body: String) {
    let init = leptos::web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(&body));
    let Ok(headers) = leptos::web_sys::Headers::new() else {
        return;
    };
    let _ = headers.set("Content-Type", "application/json");
    init.set_headers(&headers);
    let url = format!("/ndi/sessions/{session_id}/client-stats");
    let Ok(request) = leptos::web_sys::Request::new_with_str_and_init(&url, &init) else {
        return;
    };
    if let Some(window) = leptos::web_sys::window() {
        let _ = JsFuture::from(window.fetch_with_request(&request)).await;
    }
}

/// Start the ~5s per-session stats reporter. Returns the `setInterval` handle
/// (or `-1`), which the `Watchdog` clears on `stop()`/drop like the health
/// ticker. Reads the frame stats non-destructively so the existing beacon's
/// accumulators are untouched; getStats supplies `framesDecoded` /
/// `jitterBufferMs`, this session's own count window supplies `presentedFps`.
pub(crate) fn start_session_stats_reporter(
    pc: &RtcPeerConnection,
    session_id: String,
    stats: &Rc<FrameStats>,
    active: &Rc<Cell<bool>>,
) -> i32 {
    let pc = pc.clone();
    let stats = Rc::clone(stats);
    let active = Rc::clone(active);
    // This reporter's OWN presented-fps window: (last cumulative count, last ts).
    let window: Rc<Cell<(u32, f64)>> = Rc::new(Cell::new((stats.frames_presented.get(), now_ms())));
    let cb = Closure::<dyn FnMut()>::new(move || {
        if !active.get() {
            return;
        }
        let now = now_ms();
        let count_now = stats.frames_presented.get();
        let (prev_count, prev_ts) = window.get();
        let Some(presented_fps) = session_presented_fps(count_now, prev_count, now - prev_ts) else {
            return;
        };
        window.set((count_now, now));
        // Non-destructive reads — never reset the shared accumulators / cell.
        let max_present_gap_ms = stats.max_present_gap_ms.get();
        let frames_live = stats.frames_live.get();
        let pc = pc.clone();
        let session_id = session_id.clone();
        spawn_local(async move {
            if let Ok(report) = JsFuture::from(pc.get_stats()).await {
                let inbound = extract_inbound_video(&report);
                let body = build_client_stats_body(
                    presented_fps,
                    max_present_gap_ms,
                    inbound.jitter_buffer_ms,
                    inbound.frames_decoded.unwrap_or(0.0),
                    frames_live,
                );
                post_session_stats(&session_id, body).await;
            }
        });
    });
    let handle = leptos::web_sys::window()
        .and_then(|window| {
            window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    SESSION_STATS_INTERVAL_MS,
                )
                .ok()
        })
        .unwrap_or(-1);
    cb.forget();
    handle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_the_last_path_segment() {
        assert_eq!(
            session_id_from_resource_url("http://10.77.9.205/ndi/whep/cam1/abc-123"),
            Some("abc-123")
        );
        assert_eq!(
            session_id_from_resource_url("/ndi/whep/cam1/xyz"),
            Some("xyz")
        );
    }

    #[test]
    fn session_id_strips_query_and_trailing_slash() {
        assert_eq!(
            session_id_from_resource_url("http://h/ndi/whep/s/sess-9?foo=1"),
            Some("sess-9")
        );
        assert_eq!(
            session_id_from_resource_url("http://h/ndi/whep/s/sess-9/"),
            Some("sess-9"),
            "an empty trailing segment must be skipped"
        );
    }

    #[test]
    fn presented_fps_over_a_real_window() {
        // 150 frames over 5s = 30fps.
        assert_eq!(session_presented_fps(150, 0, 5_000.0), Some(30.0));
        // 30 frames over 1s = 30fps.
        assert_eq!(session_presented_fps(180, 150, 1_000.0), Some(30.0));
    }

    #[test]
    fn presented_fps_none_for_a_too_short_window() {
        assert_eq!(
            session_presented_fps(150, 100, 500.0),
            None,
            "a sub-1s window is too short for a stable rate"
        );
    }

    #[test]
    fn presented_fps_none_when_the_counter_reset() {
        assert_eq!(
            session_presented_fps(5, 200, 5_000.0),
            None,
            "a fresh session's reset counter must not report a negative-delta fps"
        );
    }

    #[test]
    fn body_is_camelcase_matching_the_server_dto() {
        let body = build_client_stats_body(29.5, 42.0, Some(11.0), 900.0, true);
        assert!(body.contains("\"presentedFps\":29.5"), "{body}");
        assert!(body.contains("\"maxPresentGapMs\":42.0"), "{body}");
        assert!(body.contains("\"jitterBufferMs\":11.0"), "{body}");
        assert!(body.contains("\"framesDecoded\":900.0"), "{body}");
        assert!(body.contains("\"framesLive\":true"), "{body}");
    }

    #[test]
    fn body_serializes_absent_jitter_as_null() {
        let body = build_client_stats_body(30.0, 33.0, None, 100.0, false);
        assert!(body.contains("\"jitterBufferMs\":null"), "{body}");
    }
}
