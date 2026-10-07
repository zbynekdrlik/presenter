---
paths:
  - "crates/presenter-server/src/resolume/port_drift_integration_tests.rs"
  - "crates/presenter-server/src/resolume/port_drift_sibling_tests.rs"
  - "crates/presenter-server/src/resolume/port_drift.rs"
  - "crates/presenter-server/src/resolume/port_claims.rs"
  - "crates/presenter-server/src/resolume/mod.rs"
  - "crates/presenter-server/src/resolume/driver.rs"
---

# Resolume port drift — never onto a port another host on the same address owns (#813); tests use verified-free consecutive ports

## A drift never lands on a port another host on the same address owns (#813)

PP runs two Arenas on one PC: Arena-Bridge on 8090 (host `arena bridge`) and
the songs Arena on 8091 (host `arena songs`). With Arena-Bridge down, the
bridge host's probe found the songs Arena answering `/product` on 8091,
adopted it and persisted it (2026-10-06 21:25:59Z). Both hosts then drove
Songs PP: lyrics twice, Bible pushes into a composition without `#bible`
clips. Arena has no instance id (every Arena answers `{"name": "Arena"}`), so
"is this MY Arena?" can only come from the configuration.

- **`port_claims.rs` owns the answer.** One `PortClaims` table per registry:
  per host its address key (`host_key`: trimmed, lowercase), configured port,
  active port, enabled flag. `ResolumeRegistry::set_hosts` rebuilds it BEFORE
  any worker spawns or gets `RefreshConfig`; every worker holds a clone.
- **Sibling** = another ENABLED host with the same `host_key`. It owns its
  configured port and its active port. A host configured on the SAME port as
  this one targets the same Arena by intent and is not a sibling. Another
  address (even another loopback IP) never is. Hostname aliases (`localhost`
  vs `127.0.0.1`) are NOT resolved: they count as different machines.
- **The probe** (`probe_port_drift`) scans `drift_candidates(configured,
  siblings)`: the #564 window minus sibling-owned ports. The host's own
  configured port is never dropped (heal-back must stay possible). The skip
  logs a WARN on the #484 power-of-two gate of the host's failure streak
  (`should_log_error(consecutive_failures)`), DEBUG otherwise.
- **Adoption is check-and-claim under one lock** (`PortClaims::try_claim`), so
  two workers probing at once cannot take the same port. A heal-back calls
  `release`. A worker records every port it adopts, so a sibling's RUNTIME
  drift is excluded, not only the persisted one. `rebuild` keeps that runtime
  claim for a host whose worker keeps running (no host/port/enabled change)
  and resets it to the persisted value for a host that gets `RefreshConfig`.
- **A sibling-owned `active_port` is dropped and persisted**
  (`HostDriver::drop_sibling_port` -> `adopt_active_port(None)` -> the
  port-drift writer): at worker start (`apply_dial_config`, the pre-#813 PP
  value heals by itself after the deploy), after `RefreshConfig`, and on
  `HostCommand::SiblingsChanged`, which `set_hosts` sends to every host whose
  own dial target did not change (a sibling was added, re-pointed, enabled or
  removed). `RefreshConfig` alone would never reach those hosts, and it drops
  the mapping. `notify_siblings_changed` uses `try_send` and falls back to a
  spawned `send` on a full queue: `set_hosts` holds the hosts lock every push
  needs, so it must never wait on a busy worker for this.
- **Keep the order in `set_hosts`:** rebuild the table FIRST, then stop /
  refresh / notify / spawn. A worker spawned or refreshed before the rebuild
  checks against the old sibling set.
- **Residual risk (config-only protection, no Arena identity exists):** an
  Arena that lands on a port in the window that NO host has claimed yet (the
  songs Arena cannot bind 8091 and takes 8092 before the songs host probed)
  goes to whichever host probes first. An Arena that drifts onto ANOTHER
  host's configured port is not excluded for that host (a configured port is
  never refused), so the two hosts can end up swapped. Neither is a
  regression from #564. A tie-breaker (e.g. prefer the host whose own
  configured port is refused) would be a separate decision.
- Tests: `port_drift_sibling_tests.rs` drives the real registry + workers
  (`set_hosts`, `snapshot_for`, an in-memory `Repository` for the persisted
  clear). Passing `repo.list_resolume_hosts()` to `set_hosts` is safe in
  parallel tests: each `connect_in_memory()` is its own DB. A negative "never adopts" check first waits for the host's
  `ConnectRefused` (the probe runs right after it), then watches the snapshot
  for 3 s. Three-port layouts use `free_port_triple()` there, which verifies
  all three ports like `free_port_pair()` does.

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
