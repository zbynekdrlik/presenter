---
paths:
  - "crates/presenter-server/src/resolume/port_drift_integration_tests.rs"
  - "crates/presenter-server/src/resolume/port_drift.rs"
---

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
