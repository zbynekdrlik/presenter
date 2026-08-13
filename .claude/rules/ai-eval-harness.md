---
paths:
  - "crates/presenter-server/src/bin/ai_eval/**"
  - "crates/presenter-server/src/lib.rs"
  - "crates/presenter-server/src/main.rs"
  - "scripts/dev/ai-eval/**"
---

# AI-eval harness (#662/#680) — gotchas for anyone touching it next

## `presenter-server` is now lib.rs + thin main.rs, not binary-only

Before #680, `presenter-server` had no `lib.rs` — everything lived under `main.rs`'s private
module tree. That's why `ai_eval` (a second `[[bin]]` target under `src/bin/`) needed the split:
in Cargo's model, EVERY binary target in a package (including a second one in `src/bin/`) is a
wholly separate crate root with zero access to another binary's private modules, no matter how
"pub(crate)" something looks inside the crate that owns `main.rs`. `lib.rs` now holds
`pub mod ai; pub mod state; pub mod config; pub mod router;` (+ `mock_integrations` behind its own
feature) — both `main` and `ai_eval` link against this SAME compiled lib crate for free (Cargo
auto-wires it), so there's no code duplication. If a THIRD binary is ever added here, it gets the
same free linkage — just widen whatever NEW `pub(crate)`→`pub` items it specifically needs (see
"minimal widening" below), don't touch anything already exposed.

## Cargo's `src/bin/*` autodiscovery fires the MOMENT the file exists — `required-features` and the file must land in the SAME commit

Confirmed empirically while building this: `cargo check --workspace --tests` (the DEFAULT build,
`ai-eval` feature off) FAILED to compile because `src/bin/ai_eval/main.rs` existed on disk while
the `Cargo.toml` `[[bin]] ... required-features = ["ai-eval"]` entry was temporarily absent —
Cargo's autobins picked the file up as an ordinary, unconditional bin target regardless. This
means: introducing a new feature-gated binary is NOT safely splittable across two commits (file
first, gate later, or vice versa) — whichever commit adds the file must ALSO add the `[[bin]]`
entry, or every intermediate state breaks the "default build unaffected by a non-default feature"
guarantee. If you ever split a large driver's commits (e.g. for a RED→GREEN test-first split),
keep the Cargo.toml gate + a minimally-compiling file tree together in the FIRST commit, and only
split the LOGIC inside already-existing files across later commits.

## Minimal `pub(crate)`→`pub` widening — don't ride extra items on the same re-export line

`ai_eval` only needs: `ai::agent` (`run_agent`), `ai::bible_validator` (module only — its contents
were already fully `pub`, pure, no `AppState`), `ai::tools` (`execute_tool`),
`ai::tools::bible_presentation::parse_bible_items`, and `state::slides`'s
`BibleItem`/`ComposedBibleSlide`/`compose_bible_items_into_slides` (pure, no DB). A first pass
widened two MORE items (`compose_bible_slides`, `PasteSlidesError`) on the same re-export line
"for consistency" — a review caught this immediately since neither is used from
`src/bin/ai_eval/` (grep before widening, not after). Keep re-export lines split by actual need,
not by "it's the same statement anyway" convenience — `pub use compose::{A, B};` +
`pub(crate) use compose::C;` on separate lines is correct when only A/B are genuinely external.

## `AppState::in_memory()` — cfg-widened, not duplicated

`#[cfg(test)]` → `#[cfg(any(test, feature = "ai-eval"))]` (`state/mod.rs`), same widening applied
to the `OscConfig` import it needs (`in_memory()`'s body uses `OscConfig::default()` — both cfg
gates must move together or the non-test/ai-eval-only build fails on a missing import). This lets
`ai_eval` build a fresh isolated `AppState` per corpus case the exact same way the existing test
suite already proves works — never add a SECOND constructor; widen the cfg on the existing one.

## Layout

`crates/presenter-server/src/bin/ai_eval/` — `main.rs` (thin dispatch), `cli.rs` (hand-rolled
arg parsing, zero new deps), `corpus.rs` (fixture structs mirroring `corpus/SCHEMA.md`
field-for-field), `trace.rs` (trace = the production `Vec<ai::ChatMessage>` + a metadata
envelope), `seed.rs` (AppState seeding), `drive.rs` (the real `run_agent` loop), `scorer/`
(pure Layer-1 scorer — `bible_replay.rs` replays the real packer/validator, `turn_analysis.rs`
reads trace-recorded content directly, `tests.rs` the fixture suite), `report.rs`. Behind the
`ai-eval` Cargo feature (non-default, zero new dependency). `scripts/dev/ai-eval/{corpus,golden,
traces,report}/` unchanged from #662's original layout.
