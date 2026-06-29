//! #502: fetch the server's WebRTC ICE servers (Cloudflare Realtime TURN) and
//! apply them to an `RtcConfiguration`.
//!
//! Without a relay candidate, the stream had exactly one media path — direct to
//! the server's LAN host candidates (`10.77.x`). A client off the LAN, or one
//! whose packets to `10.77.x` are hijacked through a Tailscale subnet route /
//! DERP relay, never received media → black + watchdog reconnect spiral. The
//! server mints short-lived ICE servers; the browser sets them on its
//! `RTCPeerConnection` so a Cloudflare relay candidate exists as a fallback.
//!
//! Fetched ONCE per page (in `NdiVideo`'s effect, before the reconnect loop)
//! and reused across every reconnect, so reconnects don't re-mint. Empty body
//! (`[]`, TURN unconfigured) or any fetch failure → `None`, and the caller uses
//! a plain default config — exactly today's LAN-only behavior.

use leptos::wasm_bindgen::{JsCast, JsValue};
use leptos::web_sys::{Response, RtcConfiguration};
use wasm_bindgen_futures::JsFuture;

/// `GET /ndi/ice-servers` → the parsed `iceServers` JS array, or `None` when the
/// request fails, the body is not a non-empty array, or TURN is unconfigured
/// (server returns `[]`). The value is already
/// `RTCConfiguration.iceServers`-compatible — the server passes Cloudflare's
/// `iceServers` array through verbatim.
pub(super) async fn fetch_ice_servers() -> Option<JsValue> {
    let window = leptos::web_sys::window()?;
    let resp_val = JsFuture::from(window.fetch_with_str("/ndi/ice-servers"))
        .await
        .ok()?;
    let resp: Response = resp_val.dyn_into().ok()?;
    if !resp.ok() {
        return None;
    }
    let text = JsFuture::from(resp.text().ok()?).await.ok()?.as_string()?;
    let value = js_sys::JSON::parse(&text).ok()?;
    let arr: js_sys::Array = value.dyn_into().ok()?;
    if arr.length() == 0 {
        None
    } else {
        Some(arr.into())
    }
}

/// Apply fetched ICE servers to an `RtcConfiguration` (no-op when `None`).
///
/// `iceTransportPolicy` is left at the browser default (`all`): the direct LAN
/// candidate still wins where reachable, so on-LAN latency is unchanged — TURN
/// is a fallback, never forced relay.
pub(super) fn apply_ice_servers(cfg: &RtcConfiguration, ice_servers: &Option<JsValue>) {
    if let Some(servers) = ice_servers {
        cfg.set_ice_servers(servers);
    }
}
