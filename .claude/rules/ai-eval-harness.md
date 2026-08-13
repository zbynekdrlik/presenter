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

## Local eval endpoint on dev2 (report §8 steps 1–3 — installed 2026-08-13)

CUDA Toolkit **13.2** (`cuda-toolkit-13-2` from NVIDIA's ubuntu2404 apt repo; matches the
595.71.05 driver's max supported CUDA, toolkit only — driver untouched) + **llama.cpp** built
from source with `-DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=120` (Blackwell `sm_120`).

- llama.cpp checkout + build: `/home/newlevel/tools/llama.cpp` (commit `8efbf65dbd55`,
  2026-08-13; binary `build/bin/llama-server`)
- Model: `/home/newlevel/models/Qwen3-8B-Q4_K_M.gguf` (unsloth/Qwen3-8B-GGUF, 4.7 GiB — the
  report §3 top pick)
- **Launcher — ALWAYS via the wrapper, never `llama-server` directly:**
  `/home/newlevel/tools/llama.cpp/run-eval-server.sh` (env-tunable: `MODEL`, `HOST`/`PORT`
  default `127.0.0.1:18790`, `CTX`, `NPREDICT`). It takes an exclusive `flock` on
  `/var/lock/dev2-gpu-eval.lock` for the server's lifetime and REFUSES to start (clear
  message, exit 1) when: the lock is held; a presenter CI job is live (`Runner.Worker` —
  covers the e2e-ndi GPU lane); GPU util is high on ≥2 of 5 one-second samples (bakerion's
  idle resident fires a sub-second burst every ~13 s, so single-sample checks false-refuse);
  any unexpected/fat GPU compute process exists; or free VRAM < 6200 MiB. All refusal paths
  plus lock release were verified live.
- **Never concurrent with bakerion active inference or the e2e-ndi CI lane** — two CUDA
  compute loads have wedged this GPU before (Xid 109; `recover-hung-gpu` skill). Bakerion's
  `backend-inference` ~0.8–1.1 GiB idle residency is the accepted baseline and does NOT block
  an eval run. NEVER set `nvidia-smi -c EXCLUSIVE_PROCESS` (breaks NVDEC → NDI decode).
- **Measured** (2026-08-13, `-c 8192`, q8_0 KV, `-ngl 99`): llama-server 5,276 MiB VRAM at
  load, 6,352 MiB total used during inference (~1.4 GiB free); prefill 1,229 tok/s, decode
  55.6 tok/s; Slovak completion clean, no `<think>` leakage (`--reasoning off` — the report
  D4 `enable_thinking` chat-template kwarg is deprecated in current llama.cpp).
- **Deviation from report §8 step 2:** default context is `-c 8192`, not 16384 — with
  bakerion resident only ~6.9 GiB VRAM is actually free, and the 16k peak (~7 GiB) would
  leave no margin; VRAM OOM is the documented wedge trigger. Raise `CTX` only after measuring.
  The systemd unit + `--sleep-idle-seconds 300` (report step 2) is NOT set up yet — the
  wrapper is the eval-run path; nothing GPU-resident is left running between eval runs.

## Layout

`crates/presenter-server/src/bin/ai_eval/` — `main.rs` (thin dispatch), `cli.rs` (hand-rolled
arg parsing, zero new deps), `corpus.rs` (fixture structs mirroring `corpus/SCHEMA.md`
field-for-field), `trace.rs` (trace = the production `Vec<ai::ChatMessage>` + a metadata
envelope), `seed.rs` (AppState seeding), `drive.rs` (the real `run_agent` loop), `scorer/`
(pure Layer-1 scorer — `bible_replay.rs` replays the real packer/validator, `turn_analysis.rs`
reads trace-recorded content directly, `tests.rs` the fixture suite), `report.rs`, `logging.rs`
(#688 — ai_eval's own `tracing_subscriber::EnvFilter`, demoting the per-case AppState-boot/
bible-ingest noise sources documented below, never touching presenter-server's own production log
levels). Behind the `ai-eval` Cargo feature (non-default, zero new dependency). `scripts/dev/
ai-eval/{corpus,golden,traces,report}/` unchanged from #662's original layout.

## Per-case boot/ingest WARN noise — demoted for ai_eval only, never for production (#688)

A 30-case corpus run built 30 fresh `AppState::in_memory()` instances and, for the 23/30
bible-authoring/adversarial cases, re-ingested the full 5-file default bible translation set from
scratch on EVERY case — firing the SAME three non-buggy `tracing::warn!` sources over and over
(~880 WARN lines in the #662 smoke-run, even at `RUST_LOG=warn`): `presenter_bible::parsers`'s
content-parsing quirks (dominant contributor — once per skipped/malformed row, times 5
translations, times 23 cases), `presenter_server::android_stage`'s "launcher URL unset" line (once
per boot — `PRESENTER_ANDROID_STAGE_URL` is never set for `ai_eval`), and `state/mod.rs`'s "NDI SDK
not found" line (once per boot — no NDI SDK on a headless eval box). None of these are bugs; they
are genuine WARN-level signals in a REAL deploy, just structurally noisy under this harness's
re-ingest-per-case design.

`bin/ai_eval/logging.rs`'s `build_eval_log_filter` demotes exactly these three to `error`, on top
of the caller's own base `RUST_LOG` level — verified NOT to hide a genuine per-case failure (seed/
ingest errors already surface structurally via `Trace.seed_failed`/`error` and `main.rs`'s
`eprintln!`, both independent of tracing level, per #662 defect 1) and NOT to swallow any OTHER
warning reachable during an actual corpus-case run (grep-verified: no `ai/tools/*` path ever
touches `AndroidStageRegistry`; `presenter_bible::parsers` has no purpose besides bible-content
parsing). The `state/mod.rs` NDI line specifically got a single `target: "presenter_server::state::
ndi_probe"` rename (not a module-wide directive) — `state` is this app's biggest catch-all module,
and a module-wide directive would risk silently swallowing an unrelated warn! added there later for
a totally different feature. **If you add a new `tracing::warn!` anywhere under `presenter_bible::
parsers` or `presenter_server::android_stage`, know that it will NOT show up during an `ai_eval`
run** (module-wide demotion) — everywhere else in the codebase, including the rest of `state/*.rs`,
is unaffected.

## Running the harness on dev2 — CI builds it, dev2 only runs it (Tier-0)

dev2 is Tier-0 (CI-only builds, see project `CLAUDE.md`), but it's where the harness must RUN —
against a local llama.cpp endpoint or the bundled CLIProxyAPI baseline. `ai_eval` is never built
locally there. `.github/workflows/ai-eval-build.yml` (workflow_dispatch only — never wired into
the push-triggered `dev`/`main` pipelines, since the whole point of the `ai-eval` feature gate is
that normal CI never pays for it) compiles the release binary on a GitHub-hosted runner and
publishes it as an artifact:

```bash
# 1. Kick off the build (any ref — usually dev)
gh workflow run ai-eval-build.yml --ref dev

# 2. Find the run + wait for it (bounded poll, per ci-monitoring.md — never gh run watch)
run_id=$(gh run list --workflow=ai-eval-build.yml --limit 1 --json databaseId -q '.[0].databaseId')
gh run view "$run_id" --json status,conclusion

# 3. Download the artifact (name is ai-eval-<short-sha>, 12-char SHA)
gh run download "$run_id" -n "ai-eval-$(git rev-parse --short=12 origin/dev)" -D /tmp/ai-eval-bin
chmod +x /tmp/ai-eval-bin/ai_eval

# 4. Run it against a candidate endpoint (bundled proxy baseline, or a local llama.cpp server) —
#    --corpus-dir/--traces-dir/--report are REQUIRED (no built-in default, #662 defect 3) since
#    this binary runs from /tmp/ai-eval-bin/, nowhere near the repo checkout it was compiled from.
#    bible-authoring/adversarial cases ALSO need the 5 env vars below set FIRST (LOCAL files,
#    never network — crates/presenter-bible/src/lib.rs default_translation_specs; unset ⇒ drive
#    exits NON-ZERO naming the unseeded cases, see "Smoke-run defect fixes" below).
export PRESENTER_BIBLE_KJV=$(pwd)/data/bibles/kjv.usfm.zip
export PRESENTER_BIBLE_SEB=$(pwd)/data/bibles/seb.bbl.mybible.zip
export PRESENTER_BIBLE_ROHACEK=$(pwd)/data/bibles/rohacek.bbl.mybible.zip
export PRESENTER_BIBLE_SEVP=$(pwd)/data/bibles/sevp.obohu.mybible.zip
export PRESENTER_BIBLE_MILOST=$(pwd)/data/bibles/milost.bbl.mybible.zip

/tmp/ai-eval-bin/ai_eval drive --candidate-url http://127.0.0.1:8787/v1 --model claude-opus-4-6 \
  --corpus-dir scripts/dev/ai-eval/corpus --traces-dir scripts/dev/ai-eval/traces
/tmp/ai-eval-bin/ai_eval score-l1 --corpus-dir scripts/dev/ai-eval/corpus \
  --traces-dir scripts/dev/ai-eval/traces --report scripts/dev/ai-eval/report/results.json
```

Artifact retention is 14 days — re-run the workflow for a fresh binary after a source change
instead of trusting a stale download. The workflow needs the same gstreamer/protobuf/cmake/nasm
system packages as `pipeline.yml`'s `build` job: `ai_eval` links against `presenter-server`'s
`lib.rs`, which pulls in `presenter-ndi` (gstreamer bindings) as a normal path dependency
regardless of which bin target is selected — it does NOT need the wasm32 target, the nightly
toolchain, or the trunk-tools cache, since it never touches the WASM `presenter-ui` crate.

## Smoke-run defect fixes (#662, first live run 2026-08-13 — issuecomment-5279674449)

The first live smoke-run against a real candidate (Qwen3-8B via llama.cpp) found 4 harness
defects, all fixed (this doc section is the corrected recipe):

- **The 5 bible env vars above are MANDATORY for bible-authoring/adversarial slices, and
  `drive` now enforces this loudly.** `PRESENTER_BIBLE_KJV/SEB/ROHACEK/SEVP/MILOST` must each
  point at the matching zip under `data/bibles/` — `AppState::refresh_default_bible_translations`
  reads these LOCAL files (never the network — the "needs network access" wording in an earlier
  version of this harness was simply wrong). **Unset a var and `drive` now exits NON-ZERO**,
  printing exactly which case(s) could not be seeded and why, and marks each such trace
  `seedFailed: true` — `score-l1` reports it as "seed failed — harness/environment issue, NOT a
  model result", never folded into the model's pass rate. Before this fix, an unset var silently
  produced a healthy-looking "Wrote 30 trace(s)" exit-0, and the loss only surfaced as a
  suspiciously bad Layer-1 score.
- **`--corpus-dir`/`--traces-dir` are required for every mode; `--report` is required for
  score-l1/all.** There is no built-in default any more — the binary always runs as a standalone
  artifact copied out of the repo (step 3/4 above), where a path baked in at compile time would
  silently point at the wrong machine.
- Traces now carry `durationMs` (wall-clock per case) and a `usage` field (was always `null` at
  the time of this smoke-run — real token counts landed in #687, see the section below).
  `score-l1`'s printed summary shows total/avg drive time and, when nonzero, a distinct
  seed-failed count.

## Real token-usage capture (#687)

`client::ChatCompletionResponse.usage: Option<client::Usage>` parses the OpenAI-compatible
`usage` object per call (`prompt_tokens`/`completion_tokens`/`total_tokens`, each independently
`Option<u32>` — a provider may omit the whole object, or just one field, and a missing count is
never read as `0`). `agent::run_agent` SUMS it across every `call_chat_completions` call the turn
makes (a turn can call the candidate more than once via the tool-call loop) into
`agent::TokenUsage` — its OWN type, deliberately reused directly by `ai_eval` (`trace.rs` imports
`presenter_server::ai::agent::TokenUsage` for `Trace::usage`, exactly like it already does for
`TurnMetadata` — never a parallel schema). `run_agent`'s return tuple widened to 4 elements again
(`(String, Vec<ToolAction>, Vec<TurnMetadata>, Option<TokenUsage>)`, same tuple-widening pattern
the #662 `turn_metadata` addition established) — `router/ai.rs`'s live SSE endpoint discards the
4th element same as the 3rd, browser-facing JSON unaffected. `report::build_report` folds every
case's `Trace::usage` into `Report::total_usage` via `TokenUsage::accumulate` (same missing-field
discipline), surfaced in `score-l1`'s printed summary when nonzero.

## Reasoning-on rerun defect fixes (#662, issuecomment-5280071954)

A second smoke-run (same corpus, Qwen3-8B reasoning ENABLED) found 2 more defects, both fixed:

- **Traces now carry `turns[]`** — one entry per LLM call `run_agent` made this turn
  (`finishReason` + `reasoningContentLen`, never the full reasoning text). `run_agent`'s Ok
  return type widened to `(String, Vec<ToolAction>, Vec<TurnMetadata>)`
  (`presenter_server::ai::agent::TurnMetadata`) — `router/ai.rs`'s live SSE chat endpoint
  discards the 3rd element (byte-identical JSON sent to the browser; this never touches the
  `ProgressEvent` SSE channel or the WASM frontend at all). `score-l1`'s report surfaces a
  `finishReasonLength` count — how many LLM calls hit the provider's context/token ceiling
  (`finishReason == "length"`), previously diagnosable only by cross-referencing the candidate
  server's own private log against trace timestamps.
- **A stalled, unproductive retry loop is now detected and classified distinctly.** When the
  candidate retries an identical failing tool call (same tool, same argument key set, same
  error/rule class) 3+ times in a row — `adv-10`'s real case did this 8 times before the
  accumulated context crashed the request with a malformed-JSON HTTP 500 — `drive::
  detect_stalled_retry_loop` marks the trace `stalledRetryLoop: "<description>"`. `score-l1`
  reports it as "candidate stalled in an unproductive retry loop", checked BEFORE (and instead
  of) the generic "run_agent returned an error" classification, and the report surfaces a
  distinct `stalledRetryLoopTotal` count. Before this fix, that crash scored identically to a
  genuine infra/network failure.
