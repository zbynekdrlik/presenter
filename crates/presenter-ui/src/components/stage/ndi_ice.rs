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
/// When servers are present AND the page is on a PUBLIC origin (domain / remote
/// / Tailscale), force `iceTransportPolicy = relay` (#502 follow-up). With the
/// default `all` policy the browser prefers a connectable host/srflx pair over a
/// relay pair — and a remote / Tailscale client's only *connectable* non-relay
/// pair is the lossy Tailscale-DERP path, which the browser latches onto and
/// never falls back from → black. Forcing `relay` removes that lossy pair so the
/// clean Cloudflare relay is used. On-LAN stage displays load via a private IP
/// (`should_force_relay` = false) → keep `all` so the direct path wins (low
/// latency, lip-sync preserved).
pub(super) fn apply_ice_servers(cfg: &RtcConfiguration, ice_servers: &Option<JsValue>) {
    let Some(servers) = ice_servers else { return };
    cfg.set_ice_servers(servers);
    let host = leptos::web_sys::window()
        .and_then(|w| w.location().hostname().ok())
        .unwrap_or_default();
    if should_force_relay(&host) {
        // web_sys' RtcConfiguration binding doesn't expose
        // `iceTransportPolicy` (feature not enabled), so set it via Reflect —
        // `RtcConfiguration` is a plain JS object. "relay" forces TURN-only.
        let _ = js_sys::Reflect::set(
            cfg.as_ref(),
            &JsValue::from_str("iceTransportPolicy"),
            &JsValue::from_str("relay"),
        );
    }
}

/// True when the page is on a PUBLIC origin (domain or public IP) → force the
/// TURN relay. A private/loopback host (the on-LAN stage displays, e.g.
/// `10.77.9.205`) returns false → keep the default `all` policy (direct LAN path
/// wins, low latency). Pure + unit-tested; `apply_ice_servers` feeds it
/// `window.location.hostname`.
pub(super) fn should_force_relay(hostname: &str) -> bool {
    !is_private_host(hostname)
}

/// RFC1918 / loopback host check (the on-LAN displays). Anything else (a domain
/// like `prsnv.newlevel.media`, or a public IP) is treated as public.
fn is_private_host(h: &str) -> bool {
    if h == "localhost"
        || h == "::1"
        || h.starts_with("127.")
        || h.starts_with("10.")
        || h.starts_with("192.168.")
    {
        return true;
    }
    // 172.16.0.0 – 172.31.255.255
    if let Some(rest) = h.strip_prefix("172.") {
        if let Ok(n) = rest.split('.').next().unwrap_or("").parse::<u8>() {
            return (16..=31).contains(&n);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::should_force_relay;

    #[test]
    fn public_domain_forces_relay() {
        // The user's case: operator opened via the public domain over Tailscale.
        assert!(should_force_relay("prsnv.newlevel.media"));
        assert!(should_force_relay("8.8.8.8")); // public IP
    }

    #[test]
    fn private_lan_keeps_direct() {
        // On-LAN stage displays load via a private IP → keep `all` (direct).
        assert!(!should_force_relay("10.77.9.205")); // prod LAN
        assert!(!should_force_relay("10.77.8.134")); // dev LAN
        assert!(!should_force_relay("192.168.1.50"));
        assert!(!should_force_relay("172.16.0.9"));
        assert!(!should_force_relay("172.31.255.1"));
        assert!(!should_force_relay("localhost"));
        assert!(!should_force_relay("127.0.0.1"));
    }

    #[test]
    fn public_172_outside_private_range_forces_relay() {
        // 172.32+ is PUBLIC (only 172.16–172.31 is RFC1918).
        assert!(should_force_relay("172.32.0.1"));
        assert!(should_force_relay("172.15.0.1"));
    }
}
