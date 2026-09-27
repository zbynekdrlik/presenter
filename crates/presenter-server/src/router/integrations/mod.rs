pub(super) mod ableset;
pub(super) mod android_stage;
pub(super) mod audit;
pub(super) mod ndi;
pub(super) mod ndi_whep;
pub(super) mod osc;
pub(super) mod resolume;
pub(super) mod resolume_push_audit;
pub(super) mod video_source;

use axum::http::HeaderMap;

/// Serde default for boolean fields that should default to `true`.
pub(super) const fn default_true() -> bool {
    true
}

/// Best-effort actor identification for settings audit rows.
///
/// Prefers the first hop from `X-Forwarded-For`, falls back to `X-Real-IP`,
/// and finally returns `"anonymous"` if neither header is present.
///
/// Note: `ConnectInfo<SocketAddr>` wiring was dropped in the axum 0.8
/// migration (deviation D2 in the spec), so there is no socket peer to
/// fall back to. If the deployment relies on direct connections with no
/// reverse proxy, configure the proxy to add `X-Forwarded-For` to recover
/// the actor IP.
pub(super) fn extract_actor(headers: &HeaderMap) -> String {
    if let Some(value) = headers.get("x-forwarded-for").and_then(|h| h.to_str().ok()) {
        let first = value.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }
    if let Some(value) = headers.get("x-real-ip").and_then(|h| h.to_str().ok()) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "anonymous".to_string()
}

#[cfg(test)]
mod tests {
    use super::{extract_actor, extract_client_ip};
    use axum::http::HeaderMap;
    use std::net::SocketAddr;

    #[test]
    fn extract_actor_prefers_first_x_forwarded_for_hop() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 10.0.0.1".parse().unwrap());
        assert_eq!(extract_actor(&headers), "1.2.3.4");
    }

    #[test]
    fn extract_actor_falls_back_to_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "5.6.7.8".parse().unwrap());
        assert_eq!(extract_actor(&headers), "5.6.7.8");
    }

    #[test]
    fn extract_actor_returns_anonymous_when_no_headers_present() {
        let headers = HeaderMap::new();
        assert_eq!(extract_actor(&headers), "anonymous");
    }

    #[test]
    fn extract_actor_skips_empty_x_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "".parse().unwrap());
        headers.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        assert_eq!(extract_actor(&headers), "9.9.9.9");
    }

    // #793: live WS clients were all logged as `client_ip=anonymous` (LAN TVs
    // send no forwarding header). The socket peer is now the fallback, while a
    // proxy header (Cloudflare tunnel → loopback peer) still wins.
    #[test]
    fn client_ip_falls_back_to_socket_peer_without_proxy_headers() {
        let peer: SocketAddr = "10.77.9.33:41522".parse().unwrap();
        assert_eq!(
            extract_client_ip(&HeaderMap::new(), Some(peer)),
            "10.77.9.33"
        );
    }

    #[test]
    fn client_ip_prefers_forwarded_header_over_tunnel_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7".parse().unwrap());
        let tunnel_peer: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        assert_eq!(
            extract_client_ip(&headers, Some(tunnel_peer)),
            "203.0.113.7",
            "behind the Cloudflare tunnel the peer is loopback — the header is the client",
        );
    }

    #[test]
    fn client_ip_is_anonymous_without_header_or_peer() {
        assert_eq!(extract_client_ip(&HeaderMap::new(), None), "anonymous");
    }
}
