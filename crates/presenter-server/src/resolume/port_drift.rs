//! #564: auto-recovery from Resolume Arena/Avenue web-server port drift.
//!
//! When Arena can't bind its configured web-server port (a previous instance
//! still holding it, transient network state) it silently binds the NEXT
//! HIGHER port. Presenter kept dialing the configured port → connection
//! refused → nobody knew where Arena actually listened (#563 incident: a
//! wrong `8090` vs the real `8091` cost minutes of blind debugging mid-event).
//! This probes a small window around the CONFIGURED port on a
//! connection-refused failure and adopts (or heals back to) whichever port
//! answers as a genuine Resolume instance.

use super::driver::HostDriver;
use super::{PortDriftEvent, ResolumeConnectionSnapshot};
use reqwest::Client;
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Arena only drifts UPWARD when it can't bind its configured port (the next
/// higher port is free) — bound the scan to a small window instead of an
/// unbounded search.
const PORT_DRIFT_PROBE_RANGE: u16 = 5;
/// A probe only ever runs against a host that JUST refused a connection
/// (i.e. is definitely up and reachable) — a slow/absent reply within half a
/// second means "nothing Resolume-shaped is here", not "give it more time".
const PORT_DRIFT_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Candidate ports to probe, in order — the CONFIGURED port FIRST (so a
/// cleanly-restarted Arena that re-bound its base port is re-adopted
/// immediately; drift must heal in both directions), then
/// `port+1..=port+5`. Pure + deterministic so the ORDER is unit-tested
/// without a network call. `checked_add` drops any candidate that would
/// overflow `u16` instead of panicking (only reachable with a configured
/// port within 5 of 65535).
pub(super) fn probe_candidate_ports(configured_port: u16) -> Vec<u16> {
    (0..=PORT_DRIFT_PROBE_RANGE)
        .filter_map(|offset| configured_port.checked_add(offset))
        .collect()
}

/// Resolume's `GET /api/v1/product` identifies the running instance as
/// `{"name": "Arena" | "Avenue", "major": .., "minor": .., ...}` — confirmed
/// against the `ArenaProductResponse` / `ProductInfo` shape in the bitfocus
/// `companion-module-resolume-arena` client (never guessed). Accepting only
/// this shape stops the probe from ever adopting a random HTTP server that
/// happens to answer on a nearby port.
pub(super) fn is_resolume_product_body(body: &serde_json::Value) -> bool {
    body.get("name")
        .and_then(|v| v.as_str())
        .map(|name| matches!(name.to_ascii_lowercase().as_str(), "arena" | "avenue"))
        .unwrap_or(false)
}

/// Probe one candidate port for a genuine Resolume product response.
async fn probe_resolume_product(client: &Client, host: &str, port: u16) -> bool {
    let url = format!("http://{host}:{port}/api/v1/product");
    let Ok(response) = client
        .get(&url)
        .timeout(PORT_DRIFT_PROBE_TIMEOUT)
        .send()
        .await
    else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    let Ok(body) = response.json::<serde_json::Value>().await else {
        return false;
    };
    is_resolume_product_body(&body)
}

impl HostDriver {
    /// A CONNECT-REFUSED-class error on the currently-dialed port MAY mean
    /// Arena rebound to a different port. Scan a small window around the
    /// CONFIGURED port (never the current dial port — healing back to the
    /// configured value must be possible even while currently dialing a
    /// stale discovered one) and adopt/heal `active_port` on the first
    /// genuine Resolume hit. No-op when nothing in the window responds — an
    /// ordinary "host is down" failure, not a port drift.
    pub(super) async fn probe_port_drift(
        &mut self,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        let configured = self.config.port;
        let host = self.config.host.clone();
        for candidate in probe_candidate_ports(configured) {
            if probe_resolume_product(&self.client, &host, candidate).await {
                let new_active = (candidate != configured).then_some(candidate);
                if new_active != self.active_port {
                    self.adopt_active_port(new_active, status).await;
                }
                return;
            }
        }
    }

    /// Apply a discovered (or healed) active port: update in-memory dial
    /// state, surface it in the status snapshot immediately (so the UI sees
    /// it without waiting for the next successful fetch), force the NEXT
    /// operation to re-resolve + refetch against the new port (the cached
    /// endpoint/mapping targeted the old one), log it, and persist it
    /// best-effort (a full channel drops the event rather than blocking the
    /// push path — the in-memory dial already switched, which is what
    /// matters for the live connection; a dropped persist just means a
    /// restart re-learns it on the next refusal).
    async fn adopt_active_port(
        &mut self,
        new_active: Option<u16>,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        let old = self.active_port;
        self.active_port = new_active;
        self.endpoint = None;
        self.mapping = None;
        self.mapping_cleared_by_error = true;
        {
            let mut guard = status.write().await;
            guard.active_port = new_active;
        }
        match new_active {
            Some(p) => warn!(
                host = %self.config.host,
                configured_port = self.config.port,
                from = ?old,
                to = p,
                "resolume port drifted"
            ),
            None => info!(
                host = %self.config.host,
                configured_port = self.config.port,
                from = ?old,
                "resolume port healed back to the configured value"
            ),
        }
        if let Some(tx) = &self.port_drift_tx {
            let _ = tx.try_send(PortDriftEvent {
                host_id: self.config.id,
                old_port: old,
                new_port: new_active,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_order_puts_the_configured_port_first_then_scans_upward() {
        assert_eq!(
            probe_candidate_ports(8090),
            vec![8090, 8091, 8092, 8093, 8094, 8095]
        );
    }

    #[test]
    fn probe_window_drops_candidates_that_would_overflow_u16() {
        // Only reachable with a configured port within PORT_DRIFT_PROBE_RANGE
        // of u16::MAX — must not panic, just stop early.
        let candidates = probe_candidate_ports(u16::MAX - 2);
        assert_eq!(candidates, vec![u16::MAX - 2, u16::MAX - 1, u16::MAX]);
    }

    #[test]
    fn accepts_an_arena_product_body() {
        let body = serde_json::json!({"name": "Arena", "major": 7, "minor": 13, "micro": 2, "revision": 0});
        assert!(is_resolume_product_body(&body));
    }

    #[test]
    fn accepts_an_avenue_product_body_case_insensitively() {
        let body = serde_json::json!({"name": "AVENUE", "major": 7});
        assert!(is_resolume_product_body(&body));
    }

    #[test]
    fn rejects_a_body_from_an_unrelated_http_server() {
        assert!(!is_resolume_product_body(
            &serde_json::json!({"status": "ok"})
        ));
        assert!(!is_resolume_product_body(
            &serde_json::json!({"name": "nginx"})
        ));
        assert!(!is_resolume_product_body(&serde_json::json!(null)));
    }
}
