pub(super) mod ableset;
pub(super) mod android_stage;
pub(super) mod audit;
pub(super) mod ndi;
pub(super) mod osc;
pub(super) mod resolume;
pub(super) mod video_source;

use axum::http::HeaderMap;
use std::net::SocketAddr;

/// Serde default for boolean fields that should default to `true`.
pub(super) const fn default_true() -> bool {
    true
}

/// Best-effort actor identification for settings audit rows.
///
/// Prefers the first hop from `X-Forwarded-For`, falls back to the socket
/// peer address, and finally returns `"anonymous"` if neither is present.
pub(super) fn extract_actor(headers: &HeaderMap, peer: Option<&SocketAddr>) -> String {
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
    peer.map(|s| s.ip().to_string())
        .unwrap_or_else(|| "anonymous".to_string())
}
