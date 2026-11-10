//! Which address the NDI receiver connects to (#925).
//!
//! `ndisrc` normally gets only the sender's NDI NAME; the NDI SDK then resolves
//! it to the address its finder saw (`NDIlib_source_t.p_url_address`). On a
//! MULTI-HOMED host that address can be one the receiver cannot reach: SNV has a
//! second NIC on the isolated Dante network (10.77.7.113/24), the SongPlayer PC
//! advertises its Dante address 10.77.7.107 too, the SDK picked it, and the SYN
//! to 10.77.7.107:5961 was never answered — the pipeline stayed silent for hours.
//!
//! The rule here:
//!
//! - The advertised address is SUSPECT only when it lies in the subnet of a local
//!   interface that is NOT the default-route interface (a side network such as
//!   Dante). Anything else — the same LAN as the default route, a routed subnet, a
//!   host without a default route — keeps today's name-only path, so single-homed
//!   hosts (dev, PP) and every normal sender are unchanged.
//! - For a suspect address the sender's other addresses come from mDNS
//!   (`<machine>.local`, the SDK reports only one url per source). The first one
//!   that is reachable over the default-route side and answers a bounded TCP
//!   connect on the sender's port becomes `url-address`.
//! - With a chosen address `ndisrc` gets `url-address` ONLY: the SDK header says
//!   the url is used when the name is NULL/empty; with a name set it may resolve
//!   the name through its finder again (`Processing.NDI.structs.h`).
//!
//! The policy (`is_off_route`, `alternatives`, `parse_*`) is pure and
//! unit-tested; [`resolve`] does the I/O (getifaddrs, `/proc/net/route`, an mDNS
//! lookup, a TCP probe) with every wait bounded.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::time::Duration;

/// How long the mDNS lookup of the sender's machine name may take.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);
/// How long one TCP connect probe to a candidate address may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
/// At most this many alternatives are probed, so the whole choice is bounded
/// (lookup + MAX_PROBES × probe = 6 s) however many A records mDNS returns.
const MAX_PROBES: usize = 4;

/// One IPv4 address of a local network interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceAddr {
    pub name: String,
    pub addr: Ipv4Addr,
    pub prefix_len: u8,
}

/// This host's IPv4 interfaces and the interface the default route uses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostNetwork {
    pub interfaces: Vec<InterfaceAddr>,
    pub default_interface: Option<String>,
}

/// What the receiver connects with: the NDI name, the address the finder
/// reported for it, and — only when that address is unreachable from here and
/// another one of the sender is — the address passed as `url-address`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NdiRoute {
    pub ndi_name: String,
    /// `p_url_address` the finder reported for this name (e.g. `10.77.7.107:5961`).
    pub advertised_address: Option<String>,
    /// The address `ndisrc` connects to instead of resolving the name.
    pub connect_address: Option<String>,
}

/// How `ndisrc` is told which sender to receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NdisrcTarget<'a> {
    /// `ndi-name` only — the SDK resolves it (today's path).
    Name(&'a str),
    /// `url-address` only — the SDK connects to exactly this address.
    UrlAddress(&'a str),
}

impl NdiRoute {
    /// Today's path: connect by name, nothing else known.
    pub fn name_only(ndi_name: &str) -> Self {
        Self {
            ndi_name: ndi_name.to_string(),
            advertised_address: None,
            connect_address: None,
        }
    }

    /// The `ndisrc` properties for this route.
    pub fn ndisrc_target(&self) -> NdisrcTarget<'_> {
        match self.connect_address.as_deref() {
            Some(url) => NdisrcTarget::UrlAddress(url),
            None => NdisrcTarget::Name(&self.ndi_name),
        }
    }
}

/// The device an interface entry belongs to: getifaddrs reports a labelled
/// IPv4 alias as `enp2s0:1`, `/proc/net/route` names the device `enp2s0`.
fn device_name(name: &str) -> &str {
    name.split_once(':').map_or(name, |(device, _)| device)
}

/// Whether `ip` lies in `iface`'s subnet.
fn in_subnet(ip: Ipv4Addr, iface: &InterfaceAddr) -> bool {
    let prefix = u32::from(iface.prefix_len.min(32));
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    (u32::from(ip) & mask) == (u32::from(iface.addr) & mask)
}

/// The finder's `p_url_address` as an IPv4 socket address (`10.77.7.107:5961`);
/// `None` for any other form (the SDK calls the format not application-readable,
/// so anything unexpected keeps the name-only path).
pub fn parse_url_address(url: &str) -> Option<SocketAddrV4> {
    url.trim().parse().ok()
}

/// The machine part of an NDI name (`"RESOLUME-SNV (SP-program)"` →
/// `"RESOLUME-SNV"`); `None` when the name has no `MACHINE (SOURCE)` form.
pub fn machine_name(ndi_name: &str) -> Option<&str> {
    let (machine, _) = ndi_name.split_once(" (")?;
    let machine = machine.trim();
    (!machine.is_empty()).then_some(machine)
}

/// The default-route interface from `/proc/net/route` (lowest metric wins).
pub fn parse_default_interface(proc_net_route: &str) -> Option<String> {
    proc_net_route
        .lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let (iface, dest, flags, metric, mask) = (
                cols.first()?,
                cols.get(1)?,
                cols.get(3)?,
                cols.get(6)?,
                cols.get(7)?,
            );
            let up = u32::from_str_radix(flags, 16).ok()? & 0x1 == 0x1;
            let default = *dest == "00000000" && *mask == "00000000";
            (up && default).then(|| (metric.parse::<u32>().unwrap_or(u32::MAX), iface.to_string()))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, iface)| iface)
}

/// Whether `ip` sits on a SIDE network of this host: inside the subnet of a
/// local interface that is not the default-route one, and not on the
/// default-route interface's own subnet. Such an address is reachable only
/// through that side interface (SNV: Dante, isolated by policy routing and a
/// firewall), so the receiver should not rely on it. `false` without a default
/// route (nothing to compare with), for loopback and for this host's own
/// addresses (a sender on this box).
pub fn is_off_route(ip: Ipv4Addr, net: &HostNetwork) -> bool {
    let Some(default) = net.default_interface.as_deref() else {
        return false;
    };
    if ip.is_loopback() || net.interfaces.iter().any(|i| i.addr == ip) {
        return false;
    }
    let on_default = net
        .interfaces
        .iter()
        .any(|i| device_name(&i.name) == default && in_subnet(ip, i));
    !on_default
        && net
            .interfaces
            .iter()
            .any(|i| device_name(&i.name) != default && !i.addr.is_loopback() && in_subnet(ip, i))
}

/// The sender's other addresses worth trying instead of `advertised`, in the
/// order the lookup returned them: IPv4, not the advertised one, not loopback /
/// unspecified, not on a side network themselves. Each keeps the advertised port.
pub fn alternatives(
    advertised: SocketAddrV4,
    candidates: &[Ipv4Addr],
    net: &HostNetwork,
) -> Vec<SocketAddrV4> {
    let mut out: Vec<SocketAddrV4> = Vec::new();
    for ip in candidates.iter().copied() {
        let usable = ip != *advertised.ip()
            && !ip.is_loopback()
            && !ip.is_unspecified()
            && !is_off_route(ip, net);
        let addr = SocketAddrV4::new(ip, advertised.port());
        if usable && !out.contains(&addr) {
            out.push(addr);
        }
    }
    out
}

/// The first alternative `probe` reaches, in order.
pub async fn first_reachable<F, Fut>(
    alternatives: &[SocketAddrV4],
    probe: F,
) -> Option<SocketAddrV4>
where
    F: Fn(SocketAddrV4) -> Fut,
    Fut: Future<Output = bool>,
{
    for addr in alternatives {
        if probe(*addr).await {
            return Some(*addr);
        }
    }
    None
}

impl HostNetwork {
    /// This host's IPv4 interfaces (getifaddrs) and default-route interface
    /// (`/proc/net/route`). An unreadable source yields an empty part, which
    /// makes [`is_off_route`] answer `false` — the name-only path.
    pub fn read() -> Self {
        Self {
            interfaces: read_interfaces(),
            default_interface: std::fs::read_to_string("/proc/net/route")
                .ok()
                .as_deref()
                .and_then(parse_default_interface),
        }
    }
}

fn read_interfaces() -> Vec<InterfaceAddr> {
    let addrs = match nix::ifaddrs::getifaddrs() {
        Ok(addrs) => addrs,
        Err(err) => {
            tracing::warn!(error = %err, "getifaddrs failed — NDI connects by name only (#925)");
            return Vec::new();
        }
    };
    addrs
        .filter_map(|ifa| {
            let addr = ifa.address.as_ref()?.as_sockaddr_in()?.ip();
            let mask = ifa.netmask.as_ref()?.as_sockaddr_in()?.ip();
            let prefix_len = u8::try_from(u32::from(mask).leading_ones()).unwrap_or(32);
            Some(InterfaceAddr {
                name: ifa.interface_name,
                addr,
                prefix_len,
            })
        })
        .collect()
}

/// The IPv4 addresses `<machine>.local` resolves to (nss-mdns), bounded.
async fn lookup_machine(machine: String, port: u16) -> Vec<Ipv4Addr> {
    let host = format!("{machine}.local:{port}");
    match tokio::time::timeout(LOOKUP_TIMEOUT, tokio::net::lookup_host(host.clone())).await {
        Ok(Ok(addrs)) => addrs
            .filter_map(|a| match a.ip() {
                IpAddr::V4(ip) => Some(ip),
                IpAddr::V6(_) => None,
            })
            .collect(),
        Ok(Err(err)) => {
            tracing::debug!(%host, error = %err, "NDI sender lookup failed (#925)");
            Vec::new()
        }
        Err(_) => {
            tracing::debug!(%host, "NDI sender lookup timed out (#925)");
            Vec::new()
        }
    }
}

/// A bounded TCP connect to `addr` (the sender's NDI port).
async fn tcp_probe(addr: SocketAddrV4) -> bool {
    matches!(
        tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

/// The route for `ndi_name`, given the address the finder advertised for it.
/// Never fails: anything it cannot decide keeps the name-only path.
pub async fn resolve(ndi_name: &str, advertised: Option<String>) -> NdiRoute {
    let net = HostNetwork::read();
    resolve_with(ndi_name, advertised, &net, lookup_machine, tcp_probe).await
}

/// [`resolve`] with the host network, the sender lookup and the TCP probe
/// injected, so the decision is testable without a network.
///
/// The advertised address is replaced when it is on a side network of this
/// host (SNV's Dante NIC) OR when it does not answer a TCP connect on the
/// sender's port (a sender advertising an address this host cannot route to,
/// e.g. SNV after its Dante NIC was removed, 2026-11-10). A reachable address
/// that is not on a side network keeps the name-only path, so a working site
/// (dev, PP) only pays one short TCP connect per pipeline start.
pub async fn resolve_with<L, LF, P, PF>(
    ndi_name: &str,
    advertised: Option<String>,
    net: &HostNetwork,
    lookup: L,
    probe: P,
) -> NdiRoute
where
    L: FnOnce(String, u16) -> LF,
    LF: Future<Output = Vec<Ipv4Addr>>,
    P: Fn(SocketAddrV4) -> PF,
    PF: Future<Output = bool>,
{
    let mut route = NdiRoute::name_only(ndi_name);
    route.advertised_address = advertised;
    let Some(advertised) = route
        .advertised_address
        .as_deref()
        .and_then(parse_url_address)
    else {
        return route;
    };
    let reason = if is_off_route(*advertised.ip(), net) {
        "on a side network"
    } else if probe(advertised).await {
        return route;
    } else {
        "not answering"
    };
    let Some(machine) = machine_name(ndi_name) else {
        tracing::debug!(%ndi_name, %advertised, reason, "advertised NDI address unusable; no machine name to look up (#925)");
        return route;
    };
    let candidates = lookup(machine.to_string(), advertised.port()).await;
    let mut alternatives = alternatives(advertised, &candidates, net);
    alternatives.truncate(MAX_PROBES);
    match first_reachable(&alternatives, probe).await {
        Some(chosen) => {
            tracing::debug!(%ndi_name, %advertised, %chosen, reason, "advertised NDI address unusable; connecting to the sender's reachable address (#925)");
            route.connect_address = Some(chosen.to_string());
        }
        None => {
            tracing::debug!(%ndi_name, %advertised, ?candidates, reason, "advertised NDI address unusable and no other address of the sender answers (#925)");
        }
    }
    route
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(name: &str, addr: [u8; 4], prefix_len: u8) -> InterfaceAddr {
        InterfaceAddr {
            name: name.to_string(),
            addr: Ipv4Addr::from(addr),
            prefix_len,
        }
    }

    /// SNV as read live 2026-11-09: LAN on the default route, Dante on a side NIC.
    fn snv() -> HostNetwork {
        HostNetwork {
            interfaces: vec![
                iface("lo", [127, 0, 0, 1], 8),
                iface("enp2s0", [10, 77, 9, 205], 23),
                iface("br-3213928f857b", [172, 18, 0, 1], 16),
                iface("docker0", [172, 17, 0, 1], 16),
                iface("enx6c1ff7c5df69", [10, 77, 7, 113], 24),
                iface("tailscale0", [100, 122, 204, 47], 32),
            ],
            default_interface: Some("enp2s0".to_string()),
        }
    }

    /// dev: one LAN NIC (+ docker, wireguard) — no side network for an NDI sender.
    fn dev() -> HostNetwork {
        HostNetwork {
            interfaces: vec![
                iface("lo", [127, 0, 0, 1], 8),
                iface("enp4s0", [10, 77, 8, 134], 23),
                iface("docker0", [172, 17, 0, 1], 16),
            ],
            default_interface: Some("enp4s0".to_string()),
        }
    }

    fn ip(a: [u8; 4]) -> Ipv4Addr {
        Ipv4Addr::from(a)
    }

    #[test]
    fn the_senders_dante_address_is_off_route_on_snv() {
        assert!(is_off_route(ip([10, 77, 7, 107]), &snv()));
    }

    #[test]
    fn a_lan_address_is_not_off_route() {
        assert!(!is_off_route(ip([10, 77, 9, 201]), &snv()));
        assert!(!is_off_route(ip([10, 77, 9, 201]), &dev()));
    }

    #[test]
    fn a_routed_address_outside_every_local_subnet_is_not_off_route() {
        // Reached through the default gateway — the normal case, name-only path.
        assert!(!is_off_route(ip([192, 168, 50, 10]), &snv()));
        // dev has no Dante NIC, so 10.77.7.x goes via the gateway there.
        assert!(!is_off_route(ip([10, 77, 7, 107]), &dev()));
    }

    /// A labelled alias of the default NIC (`enp2s0:1`) is the default NIC, not
    /// a side network.
    #[test]
    fn an_alias_of_the_default_interface_is_not_a_side_network() {
        let mut net = snv();
        net.interfaces
            .push(iface("enp2s0:1", [192, 168, 77, 2], 24));
        assert!(!is_off_route(ip([192, 168, 77, 50]), &net));
        assert_eq!(device_name("enp2s0:1"), "enp2s0");
        assert_eq!(device_name("enp2s0"), "enp2s0");
    }

    #[test]
    fn this_hosts_own_and_loopback_addresses_are_not_off_route() {
        assert!(!is_off_route(ip([10, 77, 7, 113]), &snv()));
        assert!(!is_off_route(ip([127, 0, 0, 1]), &snv()));
    }

    #[test]
    fn without_a_default_route_nothing_is_off_route() {
        let mut net = snv();
        net.default_interface = None;
        assert!(!is_off_route(ip([10, 77, 7, 107]), &net));
    }

    #[test]
    fn an_address_on_both_the_default_and_a_side_subnet_stays_on_route() {
        // Overlapping subnets: the default-route interface wins.
        let mut net = snv();
        net.interfaces.push(iface("enp2s0", [10, 77, 7, 1], 24));
        assert!(!is_off_route(ip([10, 77, 7, 107]), &net));
    }

    #[test]
    fn alternatives_keep_the_port_and_drop_unusable_addresses() {
        let advertised = SocketAddrV4::new(ip([10, 77, 7, 107]), 5961);
        let candidates = [
            ip([10, 77, 7, 107]), // the advertised one itself
            ip([127, 0, 0, 1]),   // loopback
            ip([0, 0, 0, 0]),     // unspecified
            ip([10, 77, 7, 108]), // another side-network address
            ip([10, 77, 9, 201]), // the LAN address — the one to use
            ip([10, 77, 9, 201]), // duplicate
        ];
        assert_eq!(
            alternatives(advertised, &candidates, &snv()),
            vec![SocketAddrV4::new(ip([10, 77, 9, 201]), 5961)]
        );
    }

    #[test]
    fn no_candidates_means_no_alternative() {
        let advertised = SocketAddrV4::new(ip([10, 77, 7, 107]), 5961);
        assert!(alternatives(advertised, &[], &snv()).is_empty());
    }

    /// The fake two-address sender: the first alternative does not answer, the
    /// second does — the receiver must take the second, never the first.
    #[tokio::test]
    async fn the_first_alternative_that_answers_is_chosen() {
        let a = SocketAddrV4::new(ip([10, 77, 9, 50]), 5961);
        let b = SocketAddrV4::new(ip([10, 77, 9, 201]), 5961);
        let chosen = first_reachable(&[a, b], |addr| async move { addr == b }).await;
        assert_eq!(chosen, Some(b));
        let none = first_reachable(&[a, b], |_| async { false }).await;
        assert_eq!(none, None);
    }

    /// The real probe against a listening and a closed local port.
    #[tokio::test]
    async fn the_tcp_probe_tells_a_listening_port_from_a_closed_one() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a local listener");
        let std::net::SocketAddr::V4(open) = listener.local_addr().expect("local addr") else {
            panic!("bound an IPv4 address");
        };
        assert!(tcp_probe(open).await);
        drop(listener);
        assert!(
            !tcp_probe(open).await,
            "a closed port must not count as reachable"
        );
    }

    #[test]
    fn url_address_parses_ip_and_port_only() {
        assert_eq!(
            parse_url_address("10.77.7.107:5961"),
            Some(SocketAddrV4::new(ip([10, 77, 7, 107]), 5961))
        );
        assert_eq!(parse_url_address("ndi://whatever"), None);
        assert_eq!(parse_url_address(""), None);
    }

    #[test]
    fn machine_name_is_the_part_before_the_source_name() {
        assert_eq!(
            machine_name("RESOLUME-SNV (SP-program)"),
            Some("RESOLUME-SNV")
        );
        assert_eq!(machine_name("STREAM-PP (stream (2))"), Some("STREAM-PP"));
        assert_eq!(machine_name("no-paren"), None);
        assert_eq!(machine_name(" (x)"), None);
    }

    #[test]
    fn default_interface_comes_from_the_lowest_metric_default_route() {
        let table =
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
            enp2s0\t00000000\t01084D0A\t0003\t0\t0\t102\t00000000\t0\t0\t0\n\
            enp2s0\t00084D0A\t00000000\t0001\t0\t0\t102\t00FEFFFF\t0\t0\t0\n\
            wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
            down0\t00000000\t0101A8C0\t0002\t0\t0\t1\t00000000\t0\t0\t0\n";
        assert_eq!(parse_default_interface(table), Some("enp2s0".to_string()));
        assert_eq!(parse_default_interface("Iface\tDestination\n"), None);
    }

    #[test]
    fn subnet_math_handles_the_edges() {
        let any = iface("x", [0, 0, 0, 0], 0);
        assert!(in_subnet(ip([8, 8, 8, 8]), &any));
        let host = iface("x", [10, 0, 0, 1], 32);
        assert!(in_subnet(ip([10, 0, 0, 1]), &host));
        assert!(!in_subnet(ip([10, 0, 0, 2]), &host));
    }

    #[test]
    fn a_route_with_a_connect_address_targets_the_url_only() {
        let mut route = NdiRoute::name_only("RESOLUME-SNV (SP-program)");
        assert_eq!(
            route.ndisrc_target(),
            NdisrcTarget::Name("RESOLUME-SNV (SP-program)")
        );
        route.connect_address = Some("10.77.9.201:5961".to_string());
        assert_eq!(
            route.ndisrc_target(),
            NdisrcTarget::UrlAddress("10.77.9.201:5961")
        );
    }

    /// `resolve` keeps the name-only route (no lookup, no probe) for an address
    /// that is not off-route — here loopback, which no host treats as a side
    /// network — and for a source without an advertised address. The dev / PP
    /// single-homed classification itself is pinned by the pure `dev()` tests.
    #[tokio::test]
    async fn an_advertised_address_that_is_not_off_route_keeps_the_name_only_path() {
        let route = resolve("SENDER (out)", Some("127.0.0.1:5961".to_string())).await;
        assert_eq!(route.connect_address, None);
        assert_eq!(route.advertised_address.as_deref(), Some("127.0.0.1:5961"));
        assert_eq!(route.ndisrc_target(), NdisrcTarget::Name("SENDER (out)"));
        let unknown = resolve("SENDER (out)", None).await;
        assert_eq!(unknown, NdiRoute::name_only("SENDER (out)"));
    }
    /// Probe that answers only for the listed addresses.
    fn answers(up: &'static [[u8; 4]]) -> impl Fn(SocketAddrV4) -> std::future::Ready<bool> {
        move |a: SocketAddrV4| std::future::ready(up.iter().any(|u| Ipv4Addr::from(*u) == *a.ip()))
    }

    /// SNV after the Dante NIC was pulled (2026-11-10): no side interface, the
    /// sender still advertises 10.77.7.107, which this host cannot reach.
    #[tokio::test]
    async fn an_unreachable_advertised_address_on_a_single_homed_host_uses_the_reachable_one() {
        let route = resolve_with(
            "RESOLUME-SNV (SP-program)",
            Some("10.77.7.107:5961".to_string()),
            &dev(),
            |machine, _port| {
                assert_eq!(machine, "RESOLUME-SNV");
                std::future::ready(vec![ip([10, 77, 7, 107]), ip([10, 77, 9, 201])])
            },
            answers(&[[10, 77, 9, 201]]),
        )
        .await;
        assert_eq!(route.connect_address.as_deref(), Some("10.77.9.201:5961"));
        assert_eq!(
            route.ndisrc_target(),
            NdisrcTarget::UrlAddress("10.77.9.201:5961")
        );
    }

    #[tokio::test]
    async fn a_reachable_advertised_address_keeps_the_name_only_path_without_a_lookup() {
        let route = resolve_with(
            "RESOLUME-SNV (SP-program)",
            Some("10.77.9.201:5961".to_string()),
            &dev(),
            |_machine, _port| -> std::future::Ready<Vec<Ipv4Addr>> {
                panic!("a reachable advertised address must not trigger a lookup")
            },
            answers(&[[10, 77, 9, 201]]),
        )
        .await;
        assert_eq!(route.connect_address, None);
        assert_eq!(
            route.ndisrc_target(),
            NdisrcTarget::Name("RESOLUME-SNV (SP-program)")
        );
    }

    /// A side-network address is replaced even when it answers (the Dante NIC
    /// may accept the SYN while the guard drops the stream).
    #[tokio::test]
    async fn a_side_network_address_is_replaced_even_when_it_answers() {
        let route = resolve_with(
            "RESOLUME-SNV (SP-program)",
            Some("10.77.7.107:5961".to_string()),
            &snv(),
            |_machine, _port| std::future::ready(vec![ip([10, 77, 9, 201])]),
            answers(&[[10, 77, 7, 107], [10, 77, 9, 201]]),
        )
        .await;
        assert_eq!(route.connect_address.as_deref(), Some("10.77.9.201:5961"));
    }

    #[tokio::test]
    async fn an_unreachable_address_with_no_answering_alternative_keeps_the_name() {
        let route = resolve_with(
            "RESOLUME-SNV (SP-program)",
            Some("10.77.7.107:5961".to_string()),
            &dev(),
            |_machine, _port| std::future::ready(vec![ip([10, 77, 9, 201])]),
            answers(&[]),
        )
        .await;
        assert_eq!(route.connect_address, None);
        assert_eq!(
            route.advertised_address.as_deref(),
            Some("10.77.7.107:5961")
        );
    }
}
