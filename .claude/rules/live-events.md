---
paths:
  - "crates/presenter-core/src/live.rs"
  - "crates/presenter-server/src/companion/variables.rs"
---

# Adding a `LiveEvent` variant — the one exhaustive match that breaks

`LiveEvent` (`crates/presenter-core/src/live.rs`) is consumed in many places, but only ONE match on
it is EXHAUSTIVE with no `_` catch-all: **`companion/variables.rs::apply_live_event`**. Adding a
variant without an arm there is a hard compile error (`E0004: non-exhaustive patterns`) that Tier-0
(no local `cargo check`) only surfaces at CI. **When you add a `LiveEvent` variant, add an arm to
`apply_live_event`** — return `false` when the event doesn't drive a Companion variable (that is how
NDI + stream events are handled there). This was the #706 review's 🔴: the `StreamState` /
`StreamConfigChanged` variants broke it.

Every OTHER consumer already has a catch-all and needs no change:
- WASM UI pages (`presenter-ui/src/pages/{camera,operator,stage,tablet}.rs`) — `_ => {}`.
- `presenter-ui/src/ws/stage.rs` — `Ok(event) => …` binds the rest.
- `companion/mod.rs` — matches the `live_rx.recv()` `Result`, delegates to `apply_live_event`.
- test matches (`state/tests.rs`, `companion/tests.rs`) — `other =>` / `_ => continue`.

Verify exhaustiveness by GREP, not the compiler (Tier-0): `grep -rn "LiveEvent::" crates/` and check
each match site for a catch-all. An exhaustive LiveEvent match always enumerates the common variants
(`Timers`/`Stage`/`Bible*`), so grepping for those variant ARMS finds every exhaustive consumer.
