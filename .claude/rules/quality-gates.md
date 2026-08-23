---
paths:
  - "crates/**/*.rs"
---

# Local quality-gate gotchas (#407 file-size gate, function-length gate)

## `fn_length_check.py` counts EVERY line between a function's `fn` line and its closing
## `}` — INCLUDING a rustfmt-wrapped multi-line signature that carries no braces at all

Widening a function's return type (e.g. a tuple growing a new member) can push rustfmt to
wrap the return type across several lines:

```rust
) -> anyhow::Result<(
    String,
    Vec<ToolAction>,
    Vec<TurnMetadata>,
    Option<TokenUsage>,
)> {
```

None of those wrapped lines contain a `{`/`}`, so they don't affect the checker's BRACE
COUNTING — but the checker's `length = <closing-brace-line> - <fn-line> + 1` still counts
every line in between regardless of content. A 5-line return-type wrap silently costs the
function 5 lines of its budget. This is invisible to `cargo check`/`cargo fmt` — the ONLY
way to catch it before pushing is to actually re-run the checker, scoped to the file you
touched:

```bash
QC_TARGETS="crates/presenter-server/src/ai/agent.rs" python3 scripts/dev/fn_length_check.py .
```

(`QC_TARGETS` is newline-separated relative paths — a single path works fine as shown.) A
function already sitting in the 80-120 "warning" band before your change has NO headroom
left for this — a real incident (#687) pushed `run_agent` from 108 lines (warning) to 128
lines (>120 = HARD FAIL) purely from widening its return tuple + the accumulation logic,
with `cargo fmt --all --check` and `cargo check --workspace --tests` both staying clean the
whole time (neither of them enforces this cap at all).

**Fix pattern (cheap, low-risk, ~14 lines of margin gained in the #687 case):**
1. Extract the new logic into a small, separate, pure helper function — moves those lines
   entirely off the growing function's own budget.
2. Introduce a `pub type FooResult = anyhow::Result<(...)>;` alias for the widened tuple and
   use `-> FooResult` in the signature instead of the tuple written out inline. Collapses
   rustfmt's multi-line wrap down to a single `-> FooResult {` line at the actual function
   signature (the alias declaration itself may still wrap, but `fn_length_check.py`'s regex
   only matches `fn`, never `type`, so an alias's own line count never counts against
   anything).

Re-run `QC_TARGETS=<file> python3 scripts/dev/fn_length_check.py .` after either fix to
confirm the function dropped back under 120 (ideally with real margin, not landing exactly
at 119/120 — a function sitting on the edge has zero room for the next small change).

Same idea applies to the FILE-size gate (`scripts/dev/count_prod_lines.sh`, warn >800, fail
>1000) — check it too when a file was already close to a threshold before your change:

```bash
bash scripts/dev/count_prod_lines.sh crates/presenter-server/src/ai/agent.rs
```

## Adding a field to a struct under Tier-0: grep EVERY construction site (E0063), not just the obvious ones

`#[serde(default)]` on a new field makes the WIRE back-compatible, but a Rust struct LITERAL
(`Foo { a, b }`) must still name every field — a missing one is a hard `E0063` compile error that
Tier-0 (no local `cargo check`) surfaces ONLY at CI's Clippy job, ~5 min in, costing a whole cycle
(#732: 4 missed `StageClientSnapshot { .. }` literals in `contract_tests.rs`, found only after CI).

Before pushing a struct-field addition, grep the WHOLE tree for every literal AND the enum-variant it
may be nested in — production code AND `#[cfg(test)]`/`tests/`:

```bash
grep -rn 'StructName {' crates/ tests/   # every construction site, incl. tests
```

Do NOT grep only for the type in an enum variant you happened to change — the same struct is often
built directly in unrelated tests. Also watch `E0382` partial-move: `foo.field.expect(...)` MOVES a
non-`Copy` `Option` field, so a later `&foo` (e.g. re-serialize) fails to borrow — use
`foo.field.as_ref().expect(...)` when you still need `foo` afterward. Both are invisible to
`cargo fmt` and the size/fn gates; only a compiler (CI) catches them, so the grep is the Tier-0 stand-in.

## Shared `dev` under a multi-worker fleet: concurrency cancels in-progress pipelines

`pipeline.yml` has a concurrency group that CANCELS an in-progress `dev` run when a newer push lands.
During active fleet integration (several worktree workers merging different tickets into `dev` within
minutes), a given worker's run is repeatedly `cancelled` (NOT failed) before Test/E2E/deploy-dev run —
so its code never deploys to dev and its E2E never executes, through no fault of the code. Your commits
staying an ancestor of `origin/dev` (`git merge-base --is-ancestor <sha> origin/dev`) confirms the work
is safely integrated; the compile/lint/quality jobs (Clippy/Format/Quality/TypeScript) that DID run
green before the cancel are the real validation of your slice. Do NOT open a `dev→main` PR to force
your slice through — that promotes the whole fleet's in-flight work to prod; the `dev→main` release is
the supervisor's integration decision once `dev` quiesces.

## Run BOTH gates on EVERY touched file — not just the one you think grew (#735)

A change that adds a `match` arm, a loop body, or an `if/else` branch to an *existing* function
grows THAT enclosing function silently — even in a file you were not focused on. #735 restructured
`ableset.rs`'s `run_tracker` poll loop (added a backoff arm + recovery/failure logging) and pushed
it **108 → 142 lines** (> the 120 hard-fail), while attention was on `android_stage.rs`'s file-size
in the SAME batch. `cargo fmt`/`cargo check` never flag it, and it was caught only in code review —
one wasted round-trip that a local check would have prevented. So before pushing, feed **every**
`.rs` file the change touched to `fn_length_check.py` (newline-separated `QC_TARGETS`), not only the
biggest one:

```bash
QC_TARGETS="crates/presenter-server/src/ableset.rs" python3 scripts/dev/fn_length_check.py .
```

Fix by extracting the arm/loop body into a small helper (the same #687 fix pattern above) — e.g.
`run_tracker` dropped back to 89 lines by extracting `log_ableset_recovery` +
`update_active_song_status`.

## Over-cap FILE → split via a `foo.rs` + `foo/sub.rs` submodule (#742)

When a `.rs` file nears the >1000-line hard-fail (`count_prod_lines.sh`), extract a cohesive
submodule: keep `foo.rs` and add `mod sub;` + `use sub::*;`, with the moved items at
`foo/sub.rs` (Rust-2021 sibling-dir layout). Make the moved items `pub(super)`; the parent's
`use sub::*;` re-flattens them so EVERY existing call site AND the parent's `#[cfg(test)] mod
tests { use super::*; }` resolve unchanged — a pure relocation with zero behavior change and
no test edits (tests are cap-exempt, so leave them in the parent). `clippy::wildcard_imports`
is already crate-allowed (presenter-server `lib.rs`, `#![allow(...)]`), so `use sub::*;` is
fine. Two Tier-0 traps (no local compiler): (1) recompute the parent's top-level `use` block —
imports only the MOVED code used (e.g. `OsString`, `async_trait`, `Output`, `Command`) become
unused → `-D warnings`; move a test-only one INTO the `#[cfg(test)]` block. (2) a `super::CONST`
back-reference from the submodule works because a child module sees ancestor-private items.
Verify with `count_prod_lines.sh` (both files) + `fn_length_check.py` + `cargo fmt --all --check`;
CI is the compile gate. Real case #742: `android_stage.rs` 992 → 620 by extracting
`android_stage/adb.rs` (397).
