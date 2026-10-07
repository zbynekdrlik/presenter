//! #813: which ports each configured Resolume host owns, so the #564
//! port-drift probe never moves a host onto another host's Arena.
//!
//! PP runs two Arenas on one PC: Arena-Bridge on 8090 (host `arena bridge`)
//! and the songs Arena on 8091 (host `arena songs`). With Arena-Bridge down,
//! the bridge host's probe found the songs Arena answering `/product` on 8091,
//! adopted it and persisted it (2026-10-06 21:25:59Z). From then on both hosts
//! drove Songs PP. Arena exposes no instance id (both answer
//! `{"name": "Arena"}`), so the only reliable signal is the configuration:
//! a port another host on the same machine is configured on, or has adopted,
//! belongs to that host's Arena.
//!
//! The registry owns one [`PortClaims`] table. `ResolumeRegistry::set_hosts`
//! rebuilds it from the host configs on every create / update / delete,
//! before any worker is spawned or reconfigured, and every host worker holds
//! a clone. A worker records the port it adopts, so a sibling's RUNTIME drift
//! is excluded too, not only the persisted one the registry's config knows.
//!
//! Three rules from the round-2 review of v0.4.301:
//! - A DISABLED host still owns its configured port. Disabling a host in
//!   presenter does not stop its Arena, and PP disables idle hosts.
//! - A persisted active port is only a seed. [`PortClaims::rebuild`] keeps
//!   it only while no other host on the machine owns that port.
//! - Two hosts are on the same machine when their host strings match, or
//!   when they resolved to a shared IP (`resolume-pp.lan` vs `10.77.8.201`).

use presenter_core::{ResolumeHost, ResolumeHostId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::IpAddr;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use tracing::debug;

/// Ports owned by sibling hosts: port -> label of the sibling that owns it
/// (for the logs).
pub(super) type SiblingPorts = BTreeMap<u16, String>;

type Table = HashMap<ResolumeHostId, PortClaim>;

#[derive(Debug, Clone)]
struct PortClaim {
    label: String,
    address: Address,
    port: u16,
    active_port: Option<u16>,
    is_enabled: bool,
}

/// Where a host's Arena runs, as far as the configuration tells.
#[derive(Debug, Clone)]
struct Address {
    /// The host string, trimmed and lowercased ([`host_key`]).
    key: String,
    /// The IPs the host string resolves to: parsed at once from an IP
    /// literal, recorded by the host's worker for a hostname
    /// ([`PortClaims::record_resolved_ip`], every address of the lookup, so
    /// a machine with several IPs matches on any of them). Empty until then.
    ips: BTreeSet<IpAddr>,
}

impl Address {
    fn of(host: &str) -> Self {
        Self {
            key: host_key(host),
            ips: host.trim().parse::<IpAddr>().into_iter().collect(),
        }
    }

    /// The same machine: the same host string, or a shared resolved IP.
    fn same_machine(&self, other: &Self) -> bool {
        self.key == other.key || !self.ips.is_disjoint(&other.ips)
    }
}

/// A persisted active port that [`PortClaims::rebuild`] did not keep,
/// because another host on the same machine owns it (`owner` is that host's
/// label).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DroppedSeed {
    pub(super) id: ResolumeHostId,
    pub(super) port: u16,
    pub(super) owner: String,
}

/// The shared port-ownership table (see the module docs). Cheap to clone:
/// every clone is the same table.
#[derive(Debug, Clone, Default)]
pub(super) struct PortClaims {
    table: Arc<RwLock<Table>>,
}

/// The host string, trimmed and lowercased. Hostnames are case-insensitive
/// and the settings form stores what the operator typed.
pub(super) fn host_key(host: &str) -> String {
    host.trim().to_ascii_lowercase()
}

impl PortClaims {
    /// No lock holder can leave the table half-written (every write is a
    /// plain field or map assignment), so a poisoned lock is still valid.
    fn read(&self) -> RwLockReadGuard<'_, Table> {
        self.table.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Table> {
        self.table.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replace the table with `hosts`. A host for which `keeps_runtime_port`
    /// is true (its worker keeps running with its current dial state, no
    /// `RefreshConfig`) keeps the active port its worker recorded: a live
    /// claim. Every other host starts from its persisted `active_port`: a
    /// seed, which no probe has verified since it was written.
    ///
    /// An enabled host's seed is kept only if no other host on the same
    /// machine owns that port (its configured port, its kept runtime claim or
    /// its own seed). Otherwise it is dropped and returned, and the caller
    /// starts that host on its configured port. A seed therefore never
    /// outranks a live claim, two seeds on one port both drop, and the result
    /// does not depend on the order in which the workers start. A host keeps
    /// the IP its worker resolved while its host string stays the same.
    pub(super) fn rebuild<'a>(
        &self,
        hosts: impl IntoIterator<Item = &'a ResolumeHost>,
        keeps_runtime_port: impl Fn(&ResolumeHost) -> bool,
    ) -> Vec<DroppedSeed> {
        let mut table = self.write();
        let mut seeded = Vec::new();
        let mut rebuilt = Table::new();
        for host in hosts {
            let previous = table.get(&host.id);
            let active_port = match previous {
                Some(claim) if keeps_runtime_port(host) => claim.active_port,
                _ => {
                    seeded.push(host.id);
                    host.active_port
                }
            };
            let claim = PortClaim {
                label: host.label.clone(),
                address: address_of(previous, &host.host),
                port: host.port,
                active_port,
                is_enabled: host.is_enabled,
            };
            rebuilt.insert(host.id, claim);
        }
        let dropped = colliding_seeds(&rebuilt, &seeded);
        for seed in &dropped {
            if let Some(claim) = rebuilt.get_mut(&seed.id) {
                claim.active_port = None;
            }
        }
        *table = rebuilt;
        dropped
    }

    /// Ports that sibling hosts own (see `sibling_ports_in`).
    pub(super) fn sibling_ports(
        &self,
        id: ResolumeHostId,
        host: &str,
        configured_port: u16,
    ) -> SiblingPorts {
        let table = self.read();
        let address = address_of(table.get(&id), host);
        sibling_ports_in(&table, id, &address, configured_port)
    }

    /// Record that host `id` now dials `port`, unless a sibling owns it. The
    /// check and the write happen under one lock, so two workers probing at
    /// the same moment cannot both take the same port. `Err` carries the
    /// owning sibling's label. For a host missing from the table (a host
    /// deleted meanwhile) nothing is recorded, but every other host on its
    /// machine still counts as a sibling, so their ports are still refused.
    /// A driver without a registry has an empty table: no siblings.
    pub(super) fn try_claim(
        &self,
        id: ResolumeHostId,
        host: &str,
        configured_port: u16,
        port: u16,
    ) -> Result<(), String> {
        let mut table = self.write();
        let address = address_of(table.get(&id), host);
        let siblings = sibling_ports_in(&table, id, &address, configured_port);
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

    /// Record every IP host `id` resolved `host` to (`resolve_endpoint`), so
    /// a sibling that names the same machine differently is matched by IP
    /// from now on. All of them, not only the one dialed: a machine with
    /// several IPs (wired and Wi-Fi) must match a sibling named by either,
    /// whatever order the lookup returns them in. Replaces the previous
    /// set. Ignored while the table holds another host string for `id` (a
    /// worker that resolved its old config before it applied
    /// `RefreshConfig`).
    pub(super) fn record_resolved_ip(&self, id: ResolumeHostId, host: &str, ips: BTreeSet<IpAddr>) {
        let key = host_key(host);
        let mut table = self.write();
        let Some(claim) = table.get_mut(&id).filter(|claim| claim.address.key == key) else {
            return;
        };
        if claim.address.ips != ips {
            debug!(
                host_id = %id,
                host = %host,
                from = ?claim.address.ips,
                to = ?ips,
                "resolume host resolved; hosts sharing an IP count as one machine"
            );
            claim.address.ips = ips;
        }
    }

    /// Whether host `id` is in the table. `set_hosts` rebuilds it before any
    /// worker is spawned or gets `RefreshConfig`; debug builds assert that.
    pub(super) fn contains(&self, id: ResolumeHostId) -> bool {
        self.read().contains_key(&id)
    }

    /// The active port the table holds for host `id`.
    #[cfg(test)]
    pub(super) fn active_port_of(&self, id: ResolumeHostId) -> Option<u16> {
        self.read().get(&id).and_then(|claim| claim.active_port)
    }
}

/// `host`'s address. It keeps the IPs that the host's worker resolved
/// earlier, when `previous` (the host's claim so far) has the same host
/// string (an IP literal's own IP is already in the set).
fn address_of(previous: Option<&PortClaim>, host: &str) -> Address {
    let mut address = Address::of(host);
    if let Some(claim) = previous.filter(|claim| claim.address.key == address.key) {
        address.ips.extend(claim.address.ips.iter().copied());
    }
    address
}

/// Ports that sibling hosts own. Every other host on the same machine as
/// `address` contributes its configured port, enabled or not (disabling a
/// host in presenter does not stop its Arena). When enabled, it also
/// contributes its active port. A disabled host never probes, so its
/// persisted active port is unverified. A host configured on this host's
/// own `configured_port` is the same Arena by intent, not another one, and
/// contributes nothing.
fn sibling_ports_in(
    table: &Table,
    id: ResolumeHostId,
    address: &Address,
    configured_port: u16,
) -> SiblingPorts {
    let mut owned = SiblingPorts::new();
    let siblings = table
        .iter()
        .filter(|(other, _)| **other != id)
        .map(|(_, claim)| claim)
        .filter(|claim| claim.port != configured_port && address.same_machine(&claim.address));
    for claim in siblings {
        let active_port = claim.active_port.filter(|_| claim.is_enabled);
        for port in std::iter::once(claim.port).chain(active_port) {
            owned.entry(port).or_insert_with(|| claim.label.clone());
        }
    }
    owned
}

/// The seeds in `seeded` that another host on the same machine owns (see
/// [`PortClaims::rebuild`]). A disabled host's seed is left alone: the host
/// never dials, and its active port owns nothing.
fn colliding_seeds(table: &Table, seeded: &[ResolumeHostId]) -> Vec<DroppedSeed> {
    seeded
        .iter()
        .filter_map(|id| {
            let claim = table.get(id).filter(|claim| claim.is_enabled)?;
            let port = claim.active_port.filter(|port| *port != claim.port)?;
            let owners = sibling_ports_in(table, *id, &claim.address, claim.port);
            let owner = owners.get(&port).cloned()?;
            Some(DroppedSeed {
                id: *id,
                port,
                owner,
            })
        })
        .collect()
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
    fn hosts_on_other_addresses_and_the_host_itself_own_nothing() {
        let bridge = host("arena bridge", "10.77.8.201", 8090).with_active_port(Some(8092));
        let elsewhere = host("arena stream", "10.77.8.202", 8091);
        let claims = claims_of(&[bridge.clone(), elsewhere]);

        assert!(claims
            .sibling_ports(bridge.id, "10.77.8.201", 8090)
            .is_empty());
    }

    #[test]
    fn a_disabled_sibling_owns_its_configured_port_but_not_its_active_port() {
        // Disabling a host in presenter does not stop its Arena (PP disables
        // idle hosts). A disabled host never probes, so its persisted active
        // port is unverified and owns nothing.
        let bridge = host("arena bridge", "10.77.8.201", 8090);
        let mut songs = host("arena songs", "10.77.8.201", 8091).with_active_port(Some(8093));
        songs.is_enabled = false;
        let claims = claims_of(&[bridge.clone(), songs]);

        assert_eq!(
            ports(&claims.sibling_ports(bridge.id, "10.77.8.201", 8090)),
            vec![8091]
        );
        assert_eq!(
            claims.try_claim(bridge.id, "10.77.8.201", 8090, 8091),
            Err("arena songs".to_string())
        );
        assert_eq!(
            claims.try_claim(bridge.id, "10.77.8.201", 8090, 8093),
            Ok(())
        );
    }

    #[test]
    fn ip_literals_spelled_differently_are_the_same_machine() {
        let bridge = host("arena bridge", "::1", 8090);
        let songs = host("arena songs", "0:0:0:0:0:0:0:1", 8091);
        let claims = claims_of(&[bridge.clone(), songs]);

        assert_eq!(
            ports(&claims.sibling_ports(bridge.id, "::1", 8090)),
            vec![8091]
        );
    }

    #[test]
    fn a_seeded_active_port_on_another_hosts_configured_port_is_not_kept() {
        // The PP DB after the incident: the bridge persisted the songs host's
        // configured port as its active port.
        let bridge = host("arena bridge", "10.77.8.201", 8090).with_active_port(Some(8091));
        let songs = host("arena songs", "10.77.8.201", 8091);
        let claims = claims_of(&[bridge, songs.clone()]);

        assert_eq!(
            ports(&claims.sibling_ports(songs.id, "10.77.8.201", 8091)),
            vec![8090]
        );
    }

    #[test]
    fn a_seeded_active_port_on_a_disabled_hosts_configured_port_is_not_kept() {
        let bridge = host("arena bridge", "10.77.8.201", 8090).with_active_port(Some(8091));
        let mut songs = host("arena songs", "10.77.8.201", 8091);
        songs.is_enabled = false;
        let claims = claims_of(&[bridge, songs.clone()]);

        assert_eq!(
            ports(&claims.sibling_ports(songs.id, "10.77.8.201", 8091)),
            vec![8090]
        );
    }

    #[test]
    fn a_seeded_active_port_never_outranks_a_live_runtime_claim() {
        let live = host("arena songs", "10.77.8.201", 8089);
        let claims = claims_of(&[live.clone()]);
        // The songs host's worker found its Arena on 8092 and claimed it.
        assert_eq!(claims.try_claim(live.id, "10.77.8.201", 8089, 8092), Ok(()));

        // A host is added (or re-pointed) whose persisted, unverified active
        // port is that same 8092. The songs worker keeps running.
        let seeded = host("arena bridge", "10.77.8.201", 8090).with_active_port(Some(8092));
        claims.rebuild([&live, &seeded], |h| h.id == live.id);

        // The live claim stays, the seed does not.
        assert_eq!(
            ports(&claims.sibling_ports(live.id, "10.77.8.201", 8089)),
            vec![8090]
        );
        assert_eq!(claims.try_claim(live.id, "10.77.8.201", 8089, 8092), Ok(()));
        assert_eq!(
            ports(&claims.sibling_ports(seeded.id, "10.77.8.201", 8090)),
            vec![8089, 8092]
        );
    }

    #[test]
    fn two_seeded_active_ports_on_the_same_port_are_both_dropped() {
        // Neither value is verified, so neither outranks the other; both
        // hosts re-probe and `try_claim` serializes the adoption.
        let a = host("arena a", "10.77.8.201", 8090).with_active_port(Some(8093));
        let b = host("arena b", "10.77.8.201", 8091).with_active_port(Some(8093));
        let claims = claims_of(&[a.clone(), b.clone()]);

        assert_eq!(
            ports(&claims.sibling_ports(a.id, "10.77.8.201", 8090)),
            vec![8091]
        );
        assert_eq!(
            ports(&claims.sibling_ports(b.id, "10.77.8.201", 8091)),
            vec![8090]
        );
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
        // A sibling that adopted this host's configured port (it did so
        // before this host was added) never locks this host out of its own
        // Arena.
        let songs = host("arena songs", "10.77.8.201", 8089);
        let claims = claims_of(&[songs.clone()]);
        assert_eq!(
            claims.try_claim(songs.id, "10.77.8.201", 8089, 8090),
            Ok(())
        );
        let bridge = host("arena bridge", "10.77.8.201", 8090);
        claims.rebuild([&bridge, &songs], |h| h.id == songs.id);

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

    #[test]
    fn rebuild_returns_each_dropped_seed_with_its_owner() {
        let bridge = host("arena bridge", "10.77.8.201", 8090).with_active_port(Some(8091));
        let songs = host("arena songs", "10.77.8.201", 8091).with_active_port(Some(8093));
        let claims = PortClaims::default();

        let dropped = claims.rebuild([&bridge, &songs], |_| false);

        // The songs host's seed collides with nothing and is kept.
        assert_eq!(
            dropped,
            vec![DroppedSeed {
                id: bridge.id,
                port: 8091,
                owner: "arena songs".to_string(),
            }]
        );
        assert_eq!(claims.active_port_of(bridge.id), None);
        assert_eq!(claims.active_port_of(songs.id), Some(8093));
    }

    #[test]
    fn a_disabled_hosts_seed_is_neither_dropped_nor_drops_another_seed() {
        let mut idle = host("arena idle", "10.77.8.201", 8090).with_active_port(Some(8093));
        idle.is_enabled = false;
        let songs = host("arena songs", "10.77.8.201", 8091).with_active_port(Some(8093));
        let claims = PortClaims::default();

        assert!(claims.rebuild([&idle, &songs], |_| false).is_empty());
        assert_eq!(claims.active_port_of(idle.id), Some(8093));
        assert_eq!(claims.active_port_of(songs.id), Some(8093));
    }

    #[test]
    fn a_seed_on_the_hosts_own_configured_port_is_never_dropped() {
        let bridge = host("arena bridge", "10.77.8.201", 8090).with_active_port(Some(8090));
        let songs = host("arena songs", "10.77.8.201", 8089).with_active_port(Some(8090));
        let claims = PortClaims::default();

        // The songs seed sits on the bridge's configured port and drops; the
        // bridge's own "seed" is its configured port, which no one outranks.
        let dropped = claims.rebuild([&bridge, &songs], |_| false);
        assert_eq!(
            dropped.iter().map(|seed| seed.id).collect::<Vec<_>>(),
            vec![songs.id]
        );
        assert_eq!(claims.active_port_of(bridge.id), Some(8090));
    }

    #[test]
    fn a_hostname_matches_an_ip_literal_once_its_worker_resolved_it() {
        let bridge = host("arena bridge", "resolume-pp.lan", 8090);
        let songs = host("arena songs", "10.77.8.201", 8091);
        let claims = claims_of(&[bridge.clone(), songs.clone()]);
        // Not resolved yet: two machines as far as the table can tell.
        assert!(claims
            .sibling_ports(bridge.id, "resolume-pp.lan", 8090)
            .is_empty());

        claims.record_resolved_ip(bridge.id, "Resolume-PP.lan ", ips(&["10.77.8.201"]));

        assert_eq!(
            ports(&claims.sibling_ports(bridge.id, "resolume-pp.lan", 8090)),
            vec![8091]
        );
        assert_eq!(
            ports(&claims.sibling_ports(songs.id, "10.77.8.201", 8091)),
            vec![8090]
        );
        assert_eq!(
            claims.try_claim(bridge.id, "resolume-pp.lan", 8090, 8091),
            Err("arena songs".to_string())
        );
    }

    #[test]
    fn two_hostnames_match_only_when_both_resolved_to_the_same_ip() {
        let bridge = host("arena bridge", "resolume-pp.lan", 8090);
        let songs = host("arena songs", "arena-pc.lan", 8091);
        let claims = claims_of(&[bridge.clone(), songs.clone()]);

        claims.record_resolved_ip(bridge.id, "resolume-pp.lan", ips(&["10.77.8.201"]));
        assert!(claims
            .sibling_ports(bridge.id, "resolume-pp.lan", 8090)
            .is_empty());

        claims.record_resolved_ip(songs.id, "arena-pc.lan", ips(&["10.77.8.202"]));
        assert!(claims
            .sibling_ports(bridge.id, "resolume-pp.lan", 8090)
            .is_empty());

        claims.record_resolved_ip(songs.id, "arena-pc.lan", ips(&["10.77.8.201"]));
        assert_eq!(
            ports(&claims.sibling_ports(bridge.id, "resolume-pp.lan", 8090)),
            vec![8091]
        );
    }

    #[test]
    fn a_resolved_ip_survives_a_rebuild_only_while_the_host_string_is_unchanged() {
        let bridge = host("arena bridge", "resolume-pp.lan", 8090);
        let songs = host("arena songs", "10.77.8.201", 8091);
        let claims = claims_of(&[bridge.clone(), songs.clone()]);
        claims.record_resolved_ip(bridge.id, "resolume-pp.lan", ips(&["10.77.8.201"]));

        // A port edit (RefreshConfig): same host string, the IP stays.
        let mut moved = bridge.clone();
        moved.port = 8095;
        claims.rebuild([&moved, &songs], |_| false);
        assert_eq!(
            ports(&claims.sibling_ports(bridge.id, "resolume-pp.lan", 8095)),
            vec![8091]
        );

        // Re-pointed to another machine: the old IP is gone.
        let mut elsewhere = moved.clone();
        elsewhere.host = "other-pc.lan".to_string();
        claims.rebuild([&elsewhere, &songs], |_| false);
        assert!(claims
            .sibling_ports(bridge.id, "other-pc.lan", 8095)
            .is_empty());
    }

    #[test]
    fn an_ip_resolved_for_an_old_host_string_is_ignored() {
        let bridge = host("arena bridge", "other-pc.lan", 8090);
        let songs = host("arena songs", "10.77.8.201", 8091);
        let claims = claims_of(&[bridge.clone(), songs]);

        // The worker resolved its previous host before applying the new
        // config.
        claims.record_resolved_ip(bridge.id, "resolume-pp.lan", ips(&["10.77.8.201"]));

        assert!(claims
            .sibling_ports(bridge.id, "other-pc.lan", 8090)
            .is_empty());
    }

    #[test]
    fn contains_reports_the_hosts_of_the_last_rebuild() {
        let bridge = host("arena bridge", "10.77.8.201", 8090);
        let songs = host("arena songs", "10.77.8.201", 8091);
        let claims = claims_of(&[bridge.clone(), songs.clone()]);
        assert!(claims.contains(bridge.id) && claims.contains(songs.id));

        claims.rebuild([&bridge], |_| true);
        assert!(claims.contains(bridge.id));
        assert!(!claims.contains(songs.id));
    }

    #[test]
    fn a_machine_with_several_ips_matches_a_sibling_named_by_any_of_them() {
        // The Arena PC is on wired and Wi-Fi; its name resolves to both, in
        // whatever order. The sibling is named by the Wi-Fi IP.
        let bridge = host("arena bridge", "resolume-pp.lan", 8090);
        let songs = host("arena songs", "10.77.9.201", 8091);
        let claims = claims_of(&[bridge.clone(), songs]);

        claims.record_resolved_ip(
            bridge.id,
            "resolume-pp.lan",
            ips(&["10.77.8.201", "10.77.9.201"]),
        );

        assert_eq!(
            ports(&claims.sibling_ports(bridge.id, "resolume-pp.lan", 8090)),
            vec![8091]
        );
    }

    fn ips(texts: &[&str]) -> BTreeSet<IpAddr> {
        texts
            .iter()
            .map(|text| text.parse().expect("an IP literal"))
            .collect()
    }
}
