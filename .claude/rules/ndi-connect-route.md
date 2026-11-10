---
paths:
  - "crates/presenter-ndi/src/connect_route.rs"
  - "crates/presenter-ndi/src/discovery.rs"
  - "crates/presenter-ndi/src/pipeline/build.rs"
  - "crates/presenter-ndi/src/manager/lifecycle.rs"
  - "crates/presenter-ndi/src/manager/supervisor.rs"
  - "crates/presenter-ndi/src/manager/silent_streak.rs"
  - "crates/presenter-server/src/state/integrations.rs"
---

# Which address the NDI receiver connects to — multi-homed hosts (#925)

SNV 2026-11-09: SP-program was silent for hours. SNV has a second NIC on the isolated Dante
network (10.77.7.113/24), the SongPlayer PC advertises its Dante address 10.77.7.107 too, and the
NDI SDK — given only `ndi-name` — connected to that one (`SYN-SENT … → 10.77.7.107:5961`, never
answered). `~/.ndi/ndi-config.v1.json` `adapters.allowed` did NOT change the choice (tested live).

- **`connect_route.rs` decides, before the `active` lock** (`NdiManager::route_for` in
  `start_pipeline` and `rebuild_pipeline`). The finder's `p_url_address` is suspect only when it
  lies in the subnet of a local interface that is NOT the default-route interface
  (getifaddrs + `/proc/net/route`). Use interface SUBNETS, not the routing table: SNV's Dante link
  route lives only in policy table 100, so `ip route get 10.77.7.107` says "via the LAN gateway".
- **Alternatives come from mDNS** (`<machine>.local`, nss-mdns via `lookup_host`, 2 s bound) — the
  SDK reports one url per source. The first one on the default-route side that answers a 1 s TCP
  connect on the sender's port wins.
- **`DANTE_SUBNET` (CIDR, dstack two-networks.md §4, one name for all projects)** is ALWAYS
  off-route, even on a host without a Dante leg (dev, PP); SNV + dev get `10.77.7.0/24`
  (deploy-site `dante-subnet.conf` drop-in, dev unit). Malformed → WARN, ignored.
- **A chosen address = `url-address` ALONE.** `Processing.NDI.structs.h`: the url is used when the
  name is NULL/empty; with a name set the SDK may resolve the name through its finder again.
- **A verified address is dialled DIRECTLY (`url-address`), never by name** (SNV 2026-11-10
  10:45: the finder reported 10.77.9.201, the check passed, but the SDK connecting by NAME dialled
  the sender's new Dante VLAN address 10.77.7.115 — SP-program black). The finder's
  `p_url_address` is only ONE of the sender's addresses; name resolution may pick another. The
  name-only path remains only for a source without an advertised address.
- **An advertised address that does not answer is replaced too** (2026-11-10: SNV's Dante NIC was
  pulled, no side interface left, the sender still advertised 10.77.7.107). Off a side network the
  advertised address gets ONE 1 s TCP probe; it answers → dialled as `url-address` (one short
  connect per start); silent → the same mDNS + probe alternatives. A side-network
  address is replaced even when it answers (the guard may accept the SYN, then drop the stream).
- **A dead connection must end in a pipeline EOS** (dev 2026-11-10: sender restarted on another
  port, receiver black > 120 s). `ndisrc` posts EOS after 5 s without frames (`timeout`), and only
  `Stopped` makes the supervisor rebuild and call `route_for` again. A bin posts EOS only when EVERY
  sink got one, so no sink may wait for a pad that may never come: the audio fakesink is created in
  `pad-added` (`add_demux_branches`), never up front. The NDI SDK keeps a sender's port per source
  name across restarts, so a port move is rare — but without the EOS it was permanent (by name too,
  once an outage lasted > 5 s).
- **`resolve_with`** takes the host network, the lookup and the probe as arguments — test the
  decision through it, never through real sockets.
- **Detection:** `SourceSilent { advertised_address }` carries the finder's address; the server
  logs ONE WARN per silent streak (attempt 2, `should_warn_silent_but_advertised`) for a source the
  finder still sees — "on the network but sends no frames". `/healthz ndi_pipelines[]` has
  `advertisedAddress` + `connectAddress` (null = by name); the `NDI pipeline streaming` INFO line
  has `connect_address` (`-` = by name).
- **Verify on SNV:** journal `NDI pipeline streaming … connect_address=10.77.9.201:5961`, `ss -tan`
  `ESTAB … 10.77.9.201:5961`. The temporary `nft table ip presenter_ndi_fix` DNAT must be deleted
  only after that, then verify again.
