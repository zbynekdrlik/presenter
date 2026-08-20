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

# A Companion variable that needs ASYNC data can't be set in `apply_live_event`

`apply_live_event` is SYNC (`fn(&mut self, event) -> bool`) with no `AppState` handle — it can only
set variables from data already inside the event. When a new event carries IDs but the variable needs
NAMES (or any repository / DB read), resolve it ASYNC in the `companion/mod.rs` live-loop BEFORE
storing — NOT in `apply_live_event`. Pattern (#711: `StreamState` → `stream_scene`/`stream_overlays`):
the loop does `if matches!(ev, LiveEvent::StreamState { .. })` → `stream::apply_stream_state_event(&state,
&mut vars, &ev).await` (async id→name via `repository().load_output_def`) → the sync `apply_stream_state`
setter; every other event keeps the `variables.apply_live_event(ev)` path. The event's own arm in
`apply_live_event` then stays a documented no-op kept ONLY for exhaustiveness (a genuinely variable-less
event like `StreamConfigChanged` legitimately returns `false` there). The `matches!`-then-branch shape
(borrow in the if-arm, move in the else-arm) compiles under NLL because the branches are mutually
exclusive and the event is unused afterwards.
