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
