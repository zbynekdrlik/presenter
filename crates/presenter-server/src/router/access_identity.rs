//! The Cloudflare Access e-mail of a request (#851 spec §9): an author LABEL
//! for sermons, never a permission. Trusted only from a loopback peer
//! (`cloudflared` on the same box) or with no peer at all (tests): the exact
//! rule of `integrations::extract_client_ip` (#793), because a LAN client could
//! send the header itself. On the LAN and the church Wi-Fi bypass there is no
//! e-mail, and nothing depends on one.

use std::net::SocketAddr;

use axum::http::HeaderMap;

const ACCESS_EMAIL_HEADER: &str = "cf-access-authenticated-user-email";
const MAX_EMAIL_LEN: usize = 254;

/// The trimmed, lower-case e-mail; `None` from a LAN peer, without the header,
/// or when the value is no e-mail (no `@`, longer than 254 characters).
pub(super) fn access_email(headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<String> {
    if peer.is_some_and(|addr| !addr.ip().is_loopback()) {
        return None;
    }
    let email = headers
        .get(ACCESS_EMAIL_HEADER)?
        .to_str()
        .ok()?
        .trim()
        .to_lowercase();
    (email.contains('@') && email.chars().count() <= MAX_EMAIL_LEN).then_some(email)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(email: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(ACCESS_EMAIL_HEADER, email.parse().expect("header value"));
        headers
    }

    fn peer(addr: &str) -> Option<SocketAddr> {
        Some(addr.parse().expect("socket address"))
    }

    #[test]
    fn the_tunnel_peer_and_no_peer_pass_the_email_lower_cased() {
        let header = headers("  Preacher@Example.org ");
        assert_eq!(
            access_email(&header, peer("127.0.0.1:40000")).as_deref(),
            Some("preacher@example.org")
        );
        assert_eq!(
            access_email(&header, None).as_deref(),
            Some("preacher@example.org")
        );
    }

    #[test]
    fn a_lan_peer_or_a_malformed_value_gives_none() {
        assert_eq!(
            access_email(&headers("preacher@example.org"), peer("192.0.2.7:40000")),
            None
        );
        assert_eq!(access_email(&headers("preacher"), None), None);
        let long = format!("{}@example.org", "a".repeat(250));
        assert_eq!(access_email(&headers(&long), None), None);
        assert_eq!(access_email(&HeaderMap::new(), None), None);
    }
}
