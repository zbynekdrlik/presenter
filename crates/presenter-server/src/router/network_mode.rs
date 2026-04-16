//! Network-mode classifier: determines whether a client is on the church LAN
//! (direct or via tunnel-from-same-egress) or truly remote. Used by
//! `GET /api/network-mode` and by the tablet UI's LAN/WAN pill.

use axum::{extract::State, http::HeaderMap, Json};
use serde::Serialize;

use crate::state::AppState;

/// Classifies a client from request headers. Returns `"local"` or `"remote"`.
///
/// Rules:
/// - No `CF-Connecting-IP` header → direct connection, not via tunnel → `local`.
/// - `CF-Connecting-IP` matches `local_public_ip` → same egress IP as the server,
///   so client is on the church LAN just using the tunnel URL → `local`.
/// - Otherwise, if `local_public_ip` is set → `remote`.
/// - Otherwise (no configured IP) → fall back to `is_private_ip` on the client IP.
pub fn detect_network_mode(headers: &HeaderMap, local_public_ip: Option<&str>) -> &'static str {
    let client_ip = headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string());

    match (&client_ip, local_public_ip) {
        (Some(client), Some(local)) if client == local => "local",
        (Some(_), Some(_)) => "remote",
        (None, _) => "local",
        (Some(ip), None) if is_private_ip(ip) => "local",
        (Some(_), None) => "remote",
    }
}

/// Return true for IPs in private/loopback/link-local ranges.
pub fn is_private_ip(ip: &str) -> bool {
    ip.parse::<std::net::IpAddr>().is_ok_and(|addr| match addr {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    })
}

#[derive(Debug, Serialize)]
pub struct NetworkModeResponse {
    pub mode: String,
}

pub async fn get_network_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<NetworkModeResponse> {
    let mode = detect_network_mode(&headers, state.local_public_ip.as_deref());
    Json(NetworkModeResponse {
        mode: mode.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        let key: axum::http::HeaderName = name.parse().unwrap();
        h.insert(key, HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn no_proxy_headers_is_local() {
        let h = HeaderMap::new();
        assert_eq!(detect_network_mode(&h, Some("203.0.113.50")), "local");
        assert_eq!(detect_network_mode(&h, None), "local");
    }

    #[test]
    fn cf_connecting_ip_matching_configured_is_local() {
        let h = headers_with("cf-connecting-ip", "203.0.113.50");
        assert_eq!(detect_network_mode(&h, Some("203.0.113.50")), "local");
    }

    #[test]
    fn cf_connecting_ip_different_from_configured_is_remote() {
        let h = headers_with("cf-connecting-ip", "198.51.100.10");
        assert_eq!(detect_network_mode(&h, Some("203.0.113.50")), "remote");
    }

    #[test]
    fn x_forwarded_for_falls_through_to_classification() {
        let h = headers_with("x-forwarded-for", "203.0.113.50, 10.0.0.1");
        assert_eq!(detect_network_mode(&h, Some("203.0.113.50")), "local");
    }

    #[test]
    fn no_configured_ip_falls_back_to_private_range() {
        let h = headers_with("cf-connecting-ip", "10.77.9.50");
        assert_eq!(detect_network_mode(&h, None), "local");

        let h_public = headers_with("cf-connecting-ip", "198.51.100.10");
        assert_eq!(detect_network_mode(&h_public, None), "remote");
    }

    #[test]
    fn is_private_ip_handles_ranges() {
        assert!(is_private_ip("10.1.2.3"));
        assert!(is_private_ip("192.168.0.1"));
        assert!(is_private_ip("172.20.30.40"));
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("::1"));
        assert!(!is_private_ip("8.8.8.8"));
        assert!(!is_private_ip("203.0.113.5"));
        assert!(!is_private_ip("not-an-ip"));
    }
}
