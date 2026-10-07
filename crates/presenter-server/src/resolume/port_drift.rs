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
//!
//! #813: never a port another Resolume host on the same machine owns
//! (`port_claims.rs`). PP runs two Arenas on one PC on adjacent ports; with
//! one of them down, its host found the other Arena on the next port and
//! both hosts drove the same composition.

use super::driver::{should_log_error, FetchReason, HostDriver};
use super::port_claims::SiblingPorts;
use super::{PortDriftEvent, ResolumeConnectionSnapshot};
use reqwest::Client;
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Arena only drifts UPWARD when it can't bind its configured port (the next
/// higher port is free) — bound the scan to a small window instead of an
/// unbounded search.
const PORT_DRIFT_PROBE_RANGE: u16 = 5;
/// A probe only ever runs against a host that JUST refused a connection
/// (i.e. is definitely up and reachable) — a slow/absent reply within half a
/// second means "nothing Resolume-shaped is here", not "give it more time".
const PORT_DRIFT_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
/// #813: bound for the one host lookup at worker start / `RefreshConfig`
/// (`learn_resolved_ip`), the same as the HTTP connect timeout.
const RESOLVE_AT_CONFIG_TIMEOUT: Duration = super::CONNECT_TIMEOUT;

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

/// #813: the probe window split into the ports to probe and the ports a
/// sibling host on the same machine owns (with that sibling's label).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct DriftCandidates {
    pub(super) probe: Vec<u16>,
    pub(super) sibling_owned: Vec<(u16, String)>,
}

/// #813: [`probe_candidate_ports`] minus every port in `siblings`, in the
/// same order. The configured port is never dropped: it is this host's own
/// intent, and healing back to it must stay possible. Pure, so the split is
/// unit-tested without a network call.
pub(super) fn drift_candidates(configured_port: u16, siblings: &SiblingPorts) -> DriftCandidates {
    let mut candidates = DriftCandidates {
        probe: Vec::new(),
        sibling_owned: Vec::new(),
    };
    for port in probe_candidate_ports(configured_port) {
        match siblings.get(&port) {
            Some(owner) if port != configured_port => {
                candidates.sibling_owned.push((port, owner.clone()));
            }
            _ => candidates.probe.push(port),
        }
    }
    candidates
}

/// #813: whether the "probe skipped ports owned by another host" line is a
/// WARN for this failure streak. A host whose Arena stays down while its
/// sibling's runs probes on every backoff retry, so the WARN rides the #484
/// power-of-two gate on the host's failure streak, which resets on recovery
/// (the 1st, 2nd, 4th, 8th failure and so on); DEBUG otherwise. Pure, so the
/// gate is unit-tested.
pub(super) fn sibling_skip_is_warn(consecutive_failures: u32) -> bool {
    should_log_error(consecutive_failures)
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
    ///
    /// #813: ports owned by a sibling host on the same machine are never
    /// probed nor adopted (`drift_candidates`); the skip is logged.
    pub(super) async fn probe_port_drift(
        &mut self,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        let configured = self.config.port;
        let host = self.config.host.clone();
        let siblings = self
            .port_claims
            .sibling_ports(self.config.id, &host, configured);
        let candidates = drift_candidates(configured, &siblings);
        if !candidates.sibling_owned.is_empty() {
            let failures = status.read().await.consecutive_failures;
            self.log_sibling_owned_skip(&candidates.sibling_owned, failures);
        }
        for candidate in candidates.probe {
            if !probe_resolume_product(&self.client, &host, candidate).await {
                continue;
            }
            let new_active = (candidate != configured).then_some(candidate);
            if new_active == self.active_port {
                return;
            }
            if !self.claim_dial_port(new_active) {
                continue;
            }
            self.adopt_active_port(new_active, status).await;
            return;
        }
    }

    /// #813: record the port this host is about to dial in the shared
    /// `PortClaims` table. `false` when a sibling claimed that port since the
    /// probe read the table (two workers probing at the same moment); the
    /// probe then moves on. A heal-back (`None`) releases the claim.
    pub(super) fn claim_dial_port(&self, new_active: Option<u16>) -> bool {
        let Some(port) = new_active else {
            self.port_claims.release(self.config.id);
            return true;
        };
        match self
            .port_claims
            .try_claim(self.config.id, &self.config.host, self.config.port, port)
        {
            Ok(()) => true,
            Err(owner) => {
                warn!(
                    host = %self.config.host,
                    configured_port = self.config.port,
                    port,
                    sibling = %owner,
                    "resolume port-drift candidate was just taken by another host on the same machine; not adopting it"
                );
                false
            }
        }
    }

    /// #813: the probe skipped ports a sibling owns, at WARN or DEBUG per
    /// [`sibling_skip_is_warn`], never silent.
    fn log_sibling_owned_skip(&self, skipped: &[(u16, String)], consecutive_failures: u32) {
        if sibling_skip_is_warn(consecutive_failures) {
            warn!(
                host = %self.config.host,
                configured_port = self.config.port,
                skipped = ?skipped,
                consecutive_failures,
                "resolume port-drift probe skipped ports owned by another host on the same machine"
            );
        } else {
            debug!(
                host = %self.config.host,
                configured_port = self.config.port,
                skipped = ?skipped,
                consecutive_failures,
                "resolume port-drift probe skipped ports owned by another host on the same machine (suppressed)"
            );
        }
    }

    /// #813: drop an adopted `active_port` that a sibling host on the same
    /// machine owns (its configured port, or a port it adopted), and persist
    /// that like any heal-back. Runs at worker start and after a config
    /// refresh, and when another host was added, changed or removed
    /// (`HostCommand::SiblingsChanged`: a running host's port that a
    /// sibling's new configuration now owns). At start, `set_hosts` has
    /// already dropped a persisted value another host owns (PP's bridge host
    /// on 8091 before #813); this check also catches one that only shows as
    /// a sibling's once the host's IP is resolved. It also re-records this
    /// host's dial port in the shared table, so the table matches the driver
    /// after every (re)configuration.
    pub(super) async fn drop_sibling_port(
        &mut self,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        let Some(active) = self.active_port else {
            self.port_claims.release(self.config.id);
            return;
        };
        let Err(owner) =
            self.port_claims
                .try_claim(self.config.id, &self.config.host, self.config.port, active)
        else {
            return;
        };
        warn!(
            host = %self.config.host,
            configured_port = self.config.port,
            active_port = active,
            sibling = %owner,
            "resolume active port belongs to another host on the same machine; dialing the configured port again"
        );
        self.port_claims.release(self.config.id);
        self.adopt_active_port(None, status).await;
    }

    /// Worker start and `RefreshConfig`: settle the dial target (#813), then
    /// publish the (re)configured state.
    pub(super) async fn apply_dial_config(
        &mut self,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        self.learn_resolved_ip().await;
        self.drop_sibling_port(status).await;
        self.refresh_status(status).await;
    }

    /// #813: resolve the host once, so `resolve_endpoint` has recorded its IP
    /// in the shared table before the start-up check. A sibling that names
    /// the same machine differently (`resolume-pp.lan` vs `10.77.8.201`) is
    /// then matched by IP. The endpoint stays cached for the first push.
    /// Bounded by [`RESOLVE_AT_CONFIG_TIMEOUT`]: a disabled host never dials,
    /// so its worker must not sit on an unreachable DNS server. A failure is
    /// left to the push path, which retries and reports it; until the host
    /// resolves, siblings are matched by host string only.
    async fn learn_resolved_ip(&mut self) {
        let error = match tokio::time::timeout(RESOLVE_AT_CONFIG_TIMEOUT, self.endpoint()).await {
            Ok(Ok(_)) => return,
            Ok(Err(err)) => format!("{err:#}"),
            Err(_) => format!("no answer within {RESOLVE_AT_CONFIG_TIMEOUT:?}"),
        };
        debug!(
            host = %self.config.host,
            error = %error,
            "resolume host not resolved at (re)configuration; matching siblings by host string until it resolves"
        );
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
        self.invalidate_mapping(FetchReason::ErrorInvalidated);
        self.product_verified = false;
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

    fn siblings(owned: &[(u16, &str)]) -> SiblingPorts {
        owned
            .iter()
            .map(|(port, label)| (*port, label.to_string()))
            .collect()
    }

    #[test]
    fn drift_candidates_without_siblings_is_the_whole_window() {
        let candidates = drift_candidates(8090, &SiblingPorts::new());
        assert_eq!(candidates.probe, probe_candidate_ports(8090));
        assert!(candidates.sibling_owned.is_empty());
    }

    #[test]
    fn drift_candidates_skip_sibling_ports_in_window_order() {
        // PP: the songs host on 8091, plus a port it adopted.
        let owned = siblings(&[(8091, "arena songs"), (8094, "arena songs"), (9000, "far")]);
        let candidates = drift_candidates(8090, &owned);
        assert_eq!(candidates.probe, vec![8090, 8092, 8093, 8095]);
        assert_eq!(
            candidates.sibling_owned,
            vec![
                (8091, "arena songs".to_string()),
                (8094, "arena songs".to_string())
            ]
        );
    }

    #[test]
    fn drift_candidates_never_drop_the_configured_port() {
        // A sibling that adopted this host's own port must not lock this
        // host out of healing back to it.
        let candidates = drift_candidates(8090, &siblings(&[(8090, "arena songs")]));
        assert_eq!(candidates.probe, probe_candidate_ports(8090));
        assert!(candidates.sibling_owned.is_empty());
    }

    #[test]
    fn the_sibling_skip_is_a_warn_on_the_power_of_two_failures_only() {
        let gate: Vec<(u32, bool)> = (1..=9).map(|n| (n, sibling_skip_is_warn(n))).collect();
        assert_eq!(
            gate,
            vec![
                (1, true),
                (2, true),
                (3, false),
                (4, true),
                (5, false),
                (6, false),
                (7, false),
                (8, true),
                (9, false),
            ]
        );
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
