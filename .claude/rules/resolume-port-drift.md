---
paths:
  - "crates/presenter-server/src/resolume/port_drift_integration_tests.rs"
  - "crates/presenter-server/src/resolume/port_drift_sibling_tests.rs"
  - "crates/presenter-server/src/resolume/port_drift_claim_tests.rs"
  - "crates/presenter-server/src/resolume/port_drift.rs"
  - "crates/presenter-server/src/resolume/port_claims.rs"
  - "crates/presenter-server/src/resolume/mod.rs"
  - "crates/presenter-server/src/resolume/driver.rs"
---

# Resolume port drift — never onto a port another host on the same machine owns (#813); tests use verified-free consecutive ports

## A drift never lands on a port another host on the same machine owns (#813)

PP runs two Arenas on one PC: Arena-Bridge on 8090 (host `arena bridge`) and
the songs Arena on 8091 (host `arena songs`). With Arena-Bridge down, the
bridge host's probe found the songs Arena answering `/product` on 8091,
adopted it and persisted it (2026-10-06 21:25:59Z). Both hosts then drove
Songs PP: lyrics twice, Bible pushes into a composition without `#bible`
clips. Arena has no instance id (every Arena answers `{"name": "Arena"}`), so
"is this MY Arena?" can only come from the configuration.

- **`port_claims.rs` owns the answer.** One `PortClaims` table per registry:
  per host its address (`host_key`: trimmed, lowercase, plus the resolved
  IP), configured port, active port, enabled flag. `ResolumeRegistry::set_hosts`
  rebuilds it BEFORE any worker spawns or gets `RefreshConfig`; every worker
  holds a clone.
- **Same machine** = the same `host_key`, OR both claims carry a resolved IP
  and the IPs are equal (`resolume-pp.lan` vs `10.77.8.201`, `localhost` vs
  `127.0.0.1`, `::1` vs `0:0:0:0:0:0:0:1`). An IP literal's IP is parsed at
  rebuild. A hostname's IP is recorded by its worker in `resolve_endpoint`
  (`record_resolved_ip`, ignored when the table already holds a newer host
  string for that id). `apply_dial_config` resolves once before the start-up
  check, so the IP is known before it. A refreshed host keeps its IP while its
  host string is unchanged. A hostname that has not resolved yet (DNS down)
  matches by string only until it does. Another address (even another
  loopback IP) is never the same machine.
- **Sibling** = another host on the same machine, ENABLED OR NOT. It owns its
  configured port: disabling a host in presenter does not stop its Arena, and
  PP disables idle hosts. Only an ENABLED sibling also owns its active port. A
  disabled host never probes, so its persisted value is unverified. A host
  configured on the SAME port as this one targets the same Arena by intent and
  is not a sibling.
- **A persisted `active_port` is a seed, a runtime claim is live.** `rebuild`
  keeps the active port a running, unchanged worker recorded (live). Every
  other host starts from its persisted value (a seed). An enabled host's seed
  is kept only if no other host on the machine owns that port: its configured
  port, its kept runtime claim, or its own seed (`colliding_seeds`). A seed on
  the host's OWN configured port is never dropped. `rebuild` returns the
  dropped seeds, and `set_hosts` (`start_dropped_seeds_on_configured_port`)
  sets that host's `active_port` to None BEFORE `RefreshConfig`/`spawn_host`.
  It also logs a WARN and persists the clear through the port-drift writer.
  The driver seeds itself from that config, so a seed never outranks a live
  claim, two seeds on one port both drop, and nothing depends on worker order.
  Dropping only the table entry would not do this: each worker's start check
  is a `try_claim`, so whichever ran first would win. This also heals the
  pre-#813 PP value (bridge persisted on 8091) at the first `set_hosts` after
  the deploy.
- **The probe** (`probe_port_drift`) scans `drift_candidates(configured,
  siblings)`: the #564 window minus sibling-owned ports. The host's own
  configured port is never dropped (heal-back must stay possible). The skip
  logs a WARN when `sibling_skip_is_warn(consecutive_failures)` (the #484
  power-of-two gate, `should_log_error`) is true, and at DEBUG otherwise.
- **Adoption is check-and-claim under one lock** (`PortClaims::try_claim` via
  `claim_dial_port`), so two workers probing at once cannot take the same port.
  The loser gets `false`, the table stays unchanged, and the probe moves on.
  A heal-back (`claim_dial_port(None)`) calls `release`. A worker records every
  port it adopts, so a sibling's RUNTIME drift is excluded, not only the
  persisted one.
- **A running host's port that a sibling now owns is dropped and persisted**
  (`HostDriver::drop_sibling_port` -> `adopt_active_port(None)` -> the
  port-drift writer). This runs on `HostCommand::SiblingsChanged`, which
  `set_hosts` sends to every host whose own dial target did not change (a
  sibling was added, re-pointed, enabled or removed). `RefreshConfig` alone
  would never reach those hosts, and it drops the mapping. The same check also
  runs at worker start and after `RefreshConfig`; there it catches a seed that
  becomes a sibling's port only once the host's IP has resolved.
  `notify_siblings_changed` uses `try_send` and falls back to a spawned
  `send` on a full queue: `set_hosts` holds the hosts lock every push needs,
  so it must never wait on a busy worker for this.
- **Keep the order in `set_hosts`:** rebuild the table FIRST, then stop /
  refresh / notify / spawn. A worker spawned or refreshed before the rebuild
  checks against the old sibling set. `debug_assert!(port_claims.contains(id))`
  in `spawn_host` and before the `RefreshConfig` send fails every debug test
  run if this order is ever broken.
- **Residual risk (config-only protection, no Arena identity exists):** an
  Arena that lands on a port in the window that NO host has claimed yet (the
  songs Arena cannot bind 8091 and takes 8092 before the songs host probed)
  goes to whichever host probes first. An Arena that drifts onto ANOTHER
  host's configured port is not excluded for that host (a configured port is
  never refused), so the two hosts can end up swapped. Neither is a
  regression from #564. A tie-breaker (e.g. prefer the host whose own
  configured port is refused) would be a separate decision.
- **Operational rule — two Arenas on one PC: keep their webserver ports MORE
  than `PORT_DRIFT_PROBE_RANGE` (5) apart** (PP since 2026-10-07: Arena-Bridge
  8090, Songs Arena 8100). Then neither drift window reaches the other Arena
  and the residual risk above cannot occur. The port lives in
  `<user>\Documents\Resolume Arena\Preferences\server.xml`
  (`<ServerController ... port="...">`), per Windows user, i.e. per Arena
  instance. Edit it only with that Arena closed, keep a `.bak-<ts>` copy, then
  re-point the presenter host (`PUT /integrations/resolume/hosts/{id}`).
- **Clearing a wrong persisted drift by hand** (pre-#813 builds): a host/port
  edit clears `active_port` (`repository/resolume.rs`, #564), so PUT the host
  to a throw-away port and straight back.
- Tests:
  - `port_claims.rs` unit-tests the table rules: disabled siblings, seed
    collisions, resolved-IP matching, `contains`.
  - `port_drift_claim_tests.rs` drives `claim_dial_port` on a `HostDriver`
    that shares a table (lost race, heal-back).
  - `port_drift_sibling_tests.rs` drives the real registry + workers
    (`set_hosts`, `snapshot_for`, an in-memory `Repository` for the persisted
    clear). Passing `repo.list_resolume_hosts()` to `set_hosts` is safe in
    parallel tests, because each `connect_in_memory()` is its own DB.
  - A negative "never adopts" check first waits for the host's
    `ConnectRefused` (the probe runs right after it), then watches the
    snapshot for 3 s.
  - After a clear, assert the host is OFF the sibling's port (`!=
    Some(sibling_port)`, persisted and in memory), never `is_none()`. The
    cleared host re-probes its own window and may adopt a parallel test's
    mock Arena on +2..+5.
  - Three-port layouts use `free_port_triple()` there, which verifies all
    three ports like `free_port_pair()` does.

# Resolume port-drift tests — allocate a VERIFIED-FREE CONSECUTIVE port pair

## The contract (#564): the drift target is exactly `configured_port + 1`

The port-drift subsystem exists because Resolume Arena can silently answer on a
port ONE above its configured value (real field incident: `resolume-pp`
configured on 8090, Arena actually on 8091). The production probe
(`port_drift.rs::probe_candidate_ports`) scans `configured..=configured + 5` and
adopts the first genuine Resolume hit. So the integration tests MUST drive the
drift target at exactly `configured_port + 1`, and it MUST stay inside that
5-port probe window — the two ports have to be **contiguous**.

## The flake this caused (#744) — never allocate with `free_port() + 1`

The old fixture grabbed ONE `:0` ephemeral port as `configured_port`, then
ASSUMED `configured_port + 1` was free and bound it explicitly for the drifted
wiremock server, without ever checking it. Under parallel `cargo test` load
another process routinely held `+1`, so
`bind(("127.0.0.1", drifted_port)).expect("bind drifted port")` panicked and
red-ed the whole `Test` job for whichever PR happened to run (~40-min waste).

## The pattern — use `free_port_pair() -> (u16, u16)`

`free_port_pair()` binds `base` via `:0`, then — while STILL HOLDING
`base_listener` — actually tries to bind `base + 1`; a success proves BOTH ports
were simultaneously free. It retries with a fresh `base` if `base + 1` is taken
(or `base == u16::MAX`, via `checked_add`), bounded ~100 attempts then a clear
`panic!`. Every port-drift test does
`let (configured_port, drifted_port) = free_port_pair();`.

- ANY new port-drift test (or a "non-Resolume server on a nearby port" test)
  must use `free_port_pair()`, NOT `free_port() + N` — the `+ N` blind-bind is
  exactly the #744 flake.
- A residual release-then-rebind TOCTOU window remains (inherent to "bind `:0`,
  then rebind a KNOWN port for wiremock"), but it is the same negligible
  on-loopback window the single-port helper already accepted — `base + 1` is now
  VERIFIED free at allocation instead of blindly assumed.
- This is a TEST-FIXTURE robustness concern only; the production probe is
  correct and race-free (it scans, it does not assume).
