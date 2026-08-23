//! #732 — stage-display NDI `<video>` diagnostics collector (client side).
//!
//! The grey native play-arrow the owner keeps seeing at events never
//! reproduced on the emulated Android System WebViews, and the real Vestel/TCL
//! TVs are powered only during events — so the product must SELF-REPORT the
//! per-TV WebView + NDI `<video>` runtime state. This module reads that state
//! straight off the mounted `<video data-role="ndi-video">` element and two
//! window globals stamped by the existing rVFC frame observer + #568 playback
//! guard, packages it into a [`NdiVideoDiag`], and the stage WS client
//! (`ws/stage.rs`) ships it over the presence/heartbeat socket (see the design
//! comment on #732).
//!
//! Everything is read via `js_sys::Reflect` rather than typed `web_sys`
//! accessors so the collector is decoupled from which `web_sys` feature flags
//! happen to be enabled — a field a given WebView cannot expose simply reads
//! `undefined` → `None`, degrading gracefully instead of failing to compile or
//! dropping the whole snapshot.

use leptos::wasm_bindgen::{JsCast, JsValue};
use presenter_core::NdiVideoDiag;

/// Window global the rVFC frame observer stamps with `Date.now()` on every
/// presented frame (`components/stage/ndi_frame_stats.rs`).
pub(crate) const LAST_FRAME_AT_GLOBAL: &str = "__presenterNdiLastFrameAt";
/// Window global the #568 playback guard maintains as a cumulative replay
/// count (`components/stage/ndi_playback_guard.rs`).
pub(crate) const GUARD_REPLAYS_GLOBAL: &str = "__presenterNdiGuardReplays";

/// The subset of [`NdiVideoDiag`] whose change triggers an immediate on-change
/// diagnostics push between heartbeats (mirrors the server's log-key rule:
/// paused / error / cover — video-dimension changes ride the heartbeat).
pub(crate) type DiagChangeKey = (Option<bool>, Option<u16>, Option<bool>);

/// Extract the on-change key from a snapshot.
pub(crate) fn diag_change_key(diag: &NdiVideoDiag) -> DiagChangeKey {
    (diag.paused, diag.error_code, diag.cover_visible)
}

fn reflect_get(obj: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(obj, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

/// Read a numeric window global (set via `js_sys::global()`), `None` if absent.
fn global_f64(key: &str) -> Option<f64> {
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_f64())
}

/// The mounted NDI `<video>` element, or `None` when the active layout has no
/// NDI video (nothing to report — the caller then sends no snapshot).
fn ndi_video_element() -> Option<JsValue> {
    let document = leptos::web_sys::window()?.document()?;
    let el = document
        .query_selector("video[data-role=\"ndi-video\"]")
        .ok()
        .flatten()?;
    Some(JsValue::from(el))
}

/// `getVideoPlaybackQuality()` → (totalVideoFrames, droppedVideoFrames), both
/// `None` when the WebView lacks the API (guarded for absence per the ticket).
fn playback_quality(video: &JsValue) -> (Option<f64>, Option<f64>) {
    let f = reflect_get(video, "getVideoPlaybackQuality");
    if !f.is_function() {
        return (None, None);
    }
    let func: &js_sys::Function = f.unchecked_ref();
    match func.call0(video) {
        Ok(quality) if quality.is_object() => (
            reflect_get(&quality, "totalVideoFrames").as_f64(),
            reflect_get(&quality, "droppedVideoFrames").as_f64(),
        ),
        _ => (None, None),
    }
}

/// `MediaError.code` (1..=4) from the element's `error` property, or `None`
/// when `error` is null/undefined (no media error).
fn error_code(video: &JsValue) -> Option<u16> {
    let err = reflect_get(video, "error");
    if err.is_null() || err.is_undefined() {
        return None;
    }
    reflect_get(&err, "code").as_f64().map(|c| c as u16)
}

/// Whether the neutral "waiting/connecting" cover is currently mounted over the
/// video (`components/stage/ndi_fullscreen.rs` `<Show>` mounts the element only
/// while the cover should show, so its presence in the DOM == visible).
fn cover_visible() -> Option<bool> {
    let document = leptos::web_sys::window()?.document()?;
    Some(
        document
            .query_selector(".stage-ndi__placeholder--cover")
            .ok()
            .flatten()
            .is_some(),
    )
}

/// The stage page's active layout code from `body[data-layout-code]`
/// (set by `pages/stage.rs`).
fn layout_code() -> Option<String> {
    leptos::web_sys::window()?
        .document()?
        .body()?
        .get_attribute("data-layout-code")
}

/// Collect the current NDI `<video>` diagnostics snapshot, or `None` when no
/// NDI video element is mounted (nothing to report). Never throws — a field the
/// engine cannot expose reads as `None`.
pub fn collect_ndi_video_diag() -> Option<NdiVideoDiag> {
    let video = ndi_video_element()?;
    let (frames_decoded, frames_dropped) = playback_quality(&video);
    let src_object = reflect_get(&video, "srcObject");
    let last_frame_age_ms =
        global_f64(LAST_FRAME_AT_GLOBAL).map(|stamp| js_sys::Date::now() - stamp);
    Some(NdiVideoDiag {
        paused: reflect_get(&video, "paused").as_bool(),
        ready_state: reflect_get(&video, "readyState").as_f64().map(|r| r as u8),
        video_width: reflect_get(&video, "videoWidth").as_f64().map(|w| w as u32),
        video_height: reflect_get(&video, "videoHeight")
            .as_f64()
            .map(|h| h as u32),
        current_time: reflect_get(&video, "currentTime").as_f64(),
        error_code: error_code(&video),
        has_src_object: Some(!src_object.is_null() && !src_object.is_undefined()),
        muted: reflect_get(&video, "muted").as_bool(),
        controls: reflect_get(&video, "controls").as_bool(),
        frames_decoded,
        frames_dropped,
        last_frame_age_ms,
        playback_guard_replays: global_f64(GUARD_REPLAYS_GLOBAL).map(|r| r as u32),
        cover_visible: cover_visible(),
        layout_code: layout_code(),
    })
}
