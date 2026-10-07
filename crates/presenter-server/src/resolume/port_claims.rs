//! #813: which ports each configured Resolume host owns, so the #564
//! port-drift probe never moves a host onto another host's Arena.
//!
//! PP runs two Arenas on one PC: Arena-Bridge on 8090 (host `arena bridge`)
//! and the songs Arena on 8091 (host `arena songs`). With Arena-Bridge down,
//! the bridge host's probe found the songs Arena answering `/product` on 8091,
//! adopted it and persisted it (2026-10-06 21:25:59Z). From then on both hosts
//! drove Songs PP. Arena exposes no instance id (both answer
//! `{"name": "Arena"}`), so the only reliable signal is the configuration:
//! a port another host on the same address is configured on, or has adopted,
//! belongs to that host's Arena.
//!
//! The registry owns one [`PortClaims`] table. `ResolumeRegistry::set_hosts`
//! rebuilds it from the host configs on every create / update / delete,
//! before any worker is spawned or reconfigured, and every host worker holds
//! a clone. A worker records the port it adopts, so a sibling's RUNTIME drift
//! is excluded too, not only the persisted one the registry's config knows.

use presenter_core::{ResolumeHost, ResolumeHostId};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Ports owned by sibling hosts: port -> label of the sibling that owns it
/// (for the logs).
pub(super) type SiblingPorts = BTreeMap<u16, String>;

#[derive(Debug, Clone)]
struct PortClaim {
    label: String,
    host_key: String,
    port: u16,
    active_port: Option<u16>,
    is_enabled: bool,
}

/// The shared port-ownership table (see the module docs). Cheap to clone:
/// every clone is the same table.
#[derive(Debug, Clone, Default)]
pub(super) struct PortClaims {
    table: Arc<RwLock<HashMap<ResolumeHostId, PortClaim>>>,
}

/// Two hosts are on the same machine when their host strings match after
/// trimming, case-insensitively. Hostnames are case-insensitive and the
/// settings form stores what the operator typed.
pub(super) fn host_key(host: &str) -> String {
    host.trim().to_ascii_lowercase()
}

impl PortClaims {
    /// No lock holder can leave the table half-written (every write is a
    /// plain field or map assignment), so a poisoned lock is still valid.
    fn read(&self) -> RwLockReadGuard<'_, HashMap<ResolumeHostId, PortClaim>> {
        self.table.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<ResolumeHostId, PortClaim>> {
        self.table.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replace the table with `hosts`. A host for which `keeps_runtime_port`
    /// is true (its worker keeps running with its current dial state, no
    /// `RefreshConfig`) keeps the active port its worker recorded; every other
    /// host starts from its persisted `active_port`, the value its worker is
    /// seeded with.
    pub(super) fn rebuild<'a>(
        &self,
        hosts: impl IntoIterator<Item = &'a ResolumeHost>,
        keeps_runtime_port: impl Fn(&ResolumeHost) -> bool,
    ) {
        let mut table = self.write();
        let rebuilt: HashMap<ResolumeHostId, PortClaim> = hosts
            .into_iter()
            .map(|host| {
                let active_port = match table.get(&host.id) {
                    Some(claim) if keeps_runtime_port(host) => claim.active_port,
                    _ => host.active_port,
                };
                let claim = PortClaim {
                    label: host.label.clone(),
                    host_key: host_key(&host.host),
                    port: host.port,
                    active_port,
                    is_enabled: host.is_enabled,
                };
                (host.id, claim)
            })
            .collect();
        *table = rebuilt;
    }

    /// Ports that ENABLED sibling hosts own: every other host on the same
    /// address (`host_key`) contributes its configured port and its active
    /// port. A sibling configured on this host's own `configured_port` is the
    /// same Arena by intent, not another one, and contributes nothing.
    pub(super) fn sibling_ports(
        &self,
        id: ResolumeHostId,
        host: &str,
        configured_port: u16,
    ) -> SiblingPorts {
        sibling_ports_in(&self.read(), id, &host_key(host), configured_port)
    }

    /// Record that host `id` now dials `port`, unless a sibling owns it. The
    /// check and the write happen under one lock, so two workers probing at
    /// the same moment cannot both take the same port. `Err` carries the
    /// owning sibling's label. A host missing from the table (a driver
    /// without a registry, or a host deleted meanwhile) has no siblings.
    pub(super) fn try_claim(
        &self,
        id: ResolumeHostId,
        host: &str,
        configured_port: u16,
        port: u16,
    ) -> Result<(), String> {
        let mut table = self.write();
        let siblings = sibling_ports_in(&table, id, &host_key(host), configured_port);
        // The configured port is this host's own intent: never refused.
        if let Some(owner) = siblings.get(&port).filter(|_| port != configured_port) {
            return Err(owner.clone());
        }
        if let Some(claim) = table.get_mut(&id) {
            claim.active_port = Some(port);
        }
        Ok(())
    }

    /// Record that host `id` dials its configured port again.
    pub(super) fn release(&self, id: ResolumeHostId) {
        if let Some(claim) = self.write().get_mut(&id) {
            claim.active_port = None;
        }
    }
}

fn sibling_ports_in(
    table: &HashMap<ResolumeHostId, PortClaim>,
    id: ResolumeHostId,
    key: &str,
    configured_port: u16,
) -> SiblingPorts {
    let mut owned = SiblingPorts::new();
    let siblings = table
        .iter()
        .filter(|(other, _)| **other != id)
        .map(|(_, claim)| claim)
        .filter(|claim| claim.is_enabled && claim.host_key == key && claim.port != configured_port);
    for claim in siblings {
        for port in std::iter::once(claim.port).chain(claim.active_port) {
            owned.entry(port).or_insert_with(|| claim.label.clone());
        }
    }
    owned
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn host(label: &str, address: &str, port: u16) -> ResolumeHost {
        let now = Utc::now();
        ResolumeHost::new(
            ResolumeHostId::new(),
            label.to_string(),
            address.to_string(),
            port,
            true,
            now,
            now,
        )
    }

    fn claims_of(hosts: &[ResolumeHost]) -> PortClaims {
        let claims = PortClaims::default();
        claims.rebuild(hosts, |_| false);
        claims
    }

    fn ports(siblings: &SiblingPorts) -> Vec<u16> {
        siblings.keys().copied().collect()
    }

    #[test]
    fn host_key_trims_and_ignores_case() {
        assert_eq!(host_key("  Resolume-PP.lan "), "resolume-pp.lan");
        assert_eq!(host_key("10.77.8.201"), "10.77.8.201");
    }

    #[test]
    fn a_sibling_on_the_same_address_owns_its_configured_and_active_ports() {
        let bridge = host("arena bridge", "10.77.8.201", 8090);
        let songs = host("arena songs", "10.77.8.201", 8091).with_active_port(Some(8093));
        let claims = claims_of(&[bridge.clone(), songs]);

        let siblings = claims.sibling_ports(bridge.id, "10.77.8.201", 8090);
        assert_eq!(ports(&siblings), vec![8091, 8093]);
        assert_eq!(siblings.get(&8091).map(String::as_str), Some("arena songs"));
    }

    #[test]
    fn the_address_match_trims_and_ignores_case() {
        let bridge = host("arena bridge", "resolume-pp.lan", 8090);
        let songs = host("arena songs", " Resolume-PP.LAN ", 8091);
        let claims = claims_of(&[bridge.clone(), songs]);

        assert_eq!(
            ports(&claims.sibling_ports(bridge.id, "resolume-pp.lan", 8090)),
            vec![8091]
        );
    }

    #[test]
    fn hosts_on_other_addresses_disabled_hosts_and_the_host_itself_own_nothing() {
        let bridge = host("arena bridge", "10.77.8.201", 8090).with_active_port(Some(8092));
        let elsewhere = host("arena stream", "10.77.8.202", 8091);
        let mut disabled = host("old songs", "10.77.8.201", 8094);
        disabled.is_enabled = false;
        let claims = claims_of(&[bridge.clone(), elsewhere, disabled]);

        assert!(claims
            .sibling_ports(bridge.id, "10.77.8.201", 8090)
            .is_empty());
    }

    #[test]
    fn a_sibling_configured_on_the_same_port_is_the_same_arena() {
        let lyrics = host("lyrics", "10.77.8.201", 8090);
        let duplicate = host("duplicate", "10.77.8.201", 8090).with_active_port(Some(8091));
        let claims = claims_of(&[lyrics.clone(), duplicate]);

        assert!(claims
            .sibling_ports(lyrics.id, "10.77.8.201", 8090)
            .is_empty());
    }

    #[test]
    fn a_host_unknown_to_the_table_sees_every_host_on_its_address() {
        let songs = host("arena songs", "10.77.8.201", 8091);
        let claims = claims_of(&[songs]);

        assert_eq!(
            ports(&claims.sibling_ports(ResolumeHostId::new(), "10.77.8.201", 8090)),
            vec![8091]
        );
    }

    #[test]
    fn try_claim_refuses_a_sibling_port_and_records_a_free_one() {
        let bridge = host("arena bridge", "10.77.8.201", 8090);
        let songs = host("arena songs", "10.77.8.201", 8091);
        let claims = claims_of(&[bridge.clone(), songs.clone()]);

        assert_eq!(
            claims.try_claim(bridge.id, "10.77.8.201", 8090, 8091),
            Err("arena songs".to_string())
        );
        assert_eq!(
            claims.try_claim(bridge.id, "10.77.8.201", 8090, 8092),
            Ok(())
        );
        // The songs host now sees the bridge's adopted port as owned.
        assert_eq!(
            ports(&claims.sibling_ports(songs.id, "10.77.8.201", 8091)),
            vec![8090, 8092]
        );
        claims.release(bridge.id);
        assert_eq!(
            ports(&claims.sibling_ports(songs.id, "10.77.8.201", 8091)),
            vec![8090]
        );
    }

    #[test]
    fn try_claim_always_allows_the_configured_port() {
        // A sibling that (wrongly) adopted this host's configured port never
        // locks this host out of its own Arena.
        let bridge = host("arena bridge", "10.77.8.201", 8090);
        let songs = host("arena songs", "10.77.8.201", 8089).with_active_port(Some(8090));
        let claims = claims_of(&[bridge.clone(), songs]);

        assert_eq!(
            claims.try_claim(bridge.id, "10.77.8.201", 8090, 8090),
            Ok(())
        );
    }

    #[test]
    fn rebuild_keeps_a_runtime_claim_only_for_hosts_whose_worker_keeps_running() {
        let bridge = host("arena bridge", "10.77.8.201", 8090);
        let songs = host("arena songs", "10.77.8.201", 8095);
        let claims = claims_of(&[bridge.clone(), songs.clone()]);
        assert_eq!(
            claims.try_claim(songs.id, "10.77.8.201", 8095, 8096),
            Ok(())
        );

        // Unchanged songs host: its worker still dials 8096.
        claims.rebuild([&bridge, &songs], |h| h.id == songs.id);
        assert_eq!(
            ports(&claims.sibling_ports(bridge.id, "10.77.8.201", 8090)),
            vec![8095, 8096]
        );

        // Reconfigured songs host: its worker restarts from the persisted
        // value (none here).
        claims.rebuild([&bridge, &songs], |_| false);
        assert_eq!(
            ports(&claims.sibling_ports(bridge.id, "10.77.8.201", 8090)),
            vec![8095]
        );
    }

    #[test]
    fn rebuild_drops_deleted_hosts() {
        let bridge = host("arena bridge", "10.77.8.201", 8090);
        let songs = host("arena songs", "10.77.8.201", 8091);
        let claims = claims_of(&[bridge.clone(), songs]);

        claims.rebuild([&bridge], |_| true);
        assert!(claims
            .sibling_ports(bridge.id, "10.77.8.201", 8090)
            .is_empty());
    }
}
