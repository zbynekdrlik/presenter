#!/usr/bin/env bash
set -euo pipefail

# scripts/dev/ai-eval/run.sh
#
# Entry point for the presenter AI-eval verification harness (#662, report §6.2/§6.6).
#
# Stages: drive -> score-l1 -> judge -> gate (report §6.2's directory layout / §6.6 CI wiring).
#   drive    — run the candidate model through every corpus case via the real run_agent loop,
#              write one trace JSON per case under traces/. Shells out to
#              `cargo run --bin ai_eval --features ai-eval -- drive ...`
#              (crates/presenter-server/src/bin/ai_eval/, report §8 step 8 — #680). Needs the
#              candidate model endpoint (--candidate-url) reachable over the network. For
#              bible-authoring/adversarial cases it ALSO needs the PRESENTER_BIBLE_KJV/SEB/
#              ROHACEK/SEVP/MILOST env vars pointing at the local data/bibles/*.zip archives —
#              this ingestion reads local files, never the network (#662 defect 2). A case that
#              cannot even be seeded (e.g. one of those env vars unset) makes `drive` exit
#              non-zero and print which case + why — see `.claude/rules/ai-eval-harness.md`.
#   score-l1 — deterministic structural scoring of the traces just driven (real tool structs,
#              real create_bible_presentation packer, real bible_validator, verse-text diff,
#              delete-gate check, sequencing sanity — report §6.4 Layer-1). Shells out to
#              `cargo run --bin ai_eval --features ai-eval -- score-l1 ...`. Pure — no model,
#              no network; can score already-committed golden/ traces just as well as
#              freshly-driven ones.
#   judge    — Layer-2 LLM-as-judge pass over the SAME traces via judge/promptfooconfig.yaml.
#              The config exists (skeleton PR) but has never been run end-to-end against real
#              traces — report §8 step 10, still a later ticket.
#   gate     — apply the §6.5 pass/fail bar to report/results.json and exit non-zero on any
#              failing tier — report §8 step 11, still a later ticket. score-l1 above already
#              writes report/results.json with real per-case/per-slice data; gate just needs
#              to apply the bar to it.
#
# `ai_eval` is behind the non-default `ai-eval` Cargo feature (crates/presenter-server/
# Cargo.toml) — a default `cargo build`/`cargo check`/`cargo test` never compiles it. This
# script's own drive/score-l1 stages are the intended CI/local entrypoint that DOES pass
# --features ai-eval, same as the acceptance-criterion invocation in #680.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORPUS_DIR="$SCRIPT_DIR/corpus"
GOLDEN_DIR="$SCRIPT_DIR/golden"
TRACES_DIR="$SCRIPT_DIR/traces"
REPORT_DIR="$SCRIPT_DIR/report"
JUDGE_DIR="$SCRIPT_DIR/judge"

CANDIDATE_URL=""
CANDIDATE_MODEL=""
SLICE="all"   # all | worship-crud | bible-authoring | adversarial
STAGE="all"   # all | drive | score-l1 | judge | gate

usage() {
  cat <<'EOF'
Usage: run.sh [--candidate-url URL] [--model NAME] [--slice SLICE] [--stage STAGE]

  --candidate-url URL   OpenAI-compatible /v1 base URL of the candidate model server
                         (e.g. http://10.77.8.134:8787/v1 for the recommended dev2
                         llama-server per the report §4). Required for the "drive" stage.
  --model NAME           Model name to request from --candidate-url (e.g. qwen3-8b).
                         Required for the "drive" stage.
  --slice SLICE          Restrict to one corpus slice: all (default) | worship-crud |
                         bible-authoring | adversarial.
  --stage STAGE          Which stage to run: all (default) | drive | score-l1 | judge | gate.
  -h, --help              Show this help.

Every stage that needs the not-yet-built ai_eval.rs driver (report §8 step 8) fails loudly
with a clear "not yet implemented" message instead of silently doing nothing. This is
intentional — see README.md "Status".
EOF
}

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --candidate-url)
      [[ $# -ge 2 ]] || fail "--candidate-url requires a value"
      CANDIDATE_URL="$2"
      shift 2
      ;;
    --model)
      [[ $# -ge 2 ]] || fail "--model requires a value"
      CANDIDATE_MODEL="$2"
      shift 2
      ;;
    --slice)
      [[ $# -ge 2 ]] || fail "--slice requires a value"
      SLICE="$2"
      shift 2
      ;;
    --stage)
      [[ $# -ge 2 ]] || fail "--stage requires a value"
      STAGE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

case "$SLICE" in
  all|worship-crud|bible-authoring|adversarial) ;;
  *) fail "unknown --slice '$SLICE' (expected: all, worship-crud, bible-authoring, adversarial)" ;;
esac

count_cases() {
  local slice="$1"
  if [[ "$slice" == "all" ]]; then
    find "$CORPUS_DIR" -name '*.case.json' | wc -l | tr -d ' '
  else
    find "$CORPUS_DIR/$slice" -name '*.case.json' 2>/dev/null | wc -l | tr -d ' '
  fi
}

stage_drive() {
  [[ -n "$CANDIDATE_URL" ]] || fail "drive stage requires --candidate-url (see --help)"
  [[ -n "$CANDIDATE_MODEL" ]] || fail "drive stage requires --model (see --help)"

  local n
  n="$(count_cases "$SLICE")"
  [[ "$n" -gt 0 ]] || fail "no corpus cases found for slice '$SLICE' under $CORPUS_DIR"

  echo "drive: $n case(s), slice '$SLICE', candidate $CANDIDATE_URL (model: $CANDIDATE_MODEL)"
  local slice_args=()
  [[ "$SLICE" == "all" ]] || slice_args=(--slice "$SLICE")
  (
    cd "$SCRIPT_DIR/../../.." && \
    cargo run --bin ai_eval --features ai-eval -- drive \
      --candidate-url "$CANDIDATE_URL" \
      --model "$CANDIDATE_MODEL" \
      --corpus-dir "$CORPUS_DIR" \
      --traces-dir "$TRACES_DIR" \
      "${slice_args[@]}"
  ) || fail "drive stage failed — see cargo output above"
}

stage_score_l1() {
  if [[ ! -d "$TRACES_DIR" ]] || [[ -z "$(find "$TRACES_DIR" -name '*.json' -print -quit 2>/dev/null)" ]]; then
    fail "no traces under $TRACES_DIR — run the drive stage first"
  fi

  echo "score-l1: scoring traces under $TRACES_DIR (slice '$SLICE') — pure, no model/network"
  local slice_args=()
  [[ "$SLICE" == "all" ]] || slice_args=(--slice "$SLICE")
  (
    cd "$SCRIPT_DIR/../../.." && \
    cargo run --bin ai_eval --features ai-eval -- score-l1 \
      --corpus-dir "$CORPUS_DIR" \
      --traces-dir "$TRACES_DIR" \
      --report "$REPORT_DIR/results.json" \
      "${slice_args[@]}"
  ) || fail "score-l1 stage failed — see cargo output above"
}

stage_judge() {
  if [[ ! -d "$TRACES_DIR" ]] || [[ -z "$(find "$TRACES_DIR" -name '*.json' -print -quit 2>/dev/null)" ]]; then
    fail "no traces under $TRACES_DIR — run the drive stage first (not yet implemented)"
  fi
  command -v npx >/dev/null 2>&1 || fail "npx not found — the judge stage runs via" \
"'npx promptfoo eval -c judge/promptfooconfig.yaml'. Node 22 is already provisioned on the" \
"dev2 self-hosted runner (CLAUDE.md Runner Architecture) for Playwright; nothing new to" \
"install there. Do NOT run 'npm install' on this box outside that provisioned toolchain."
  fail "judge stage has never been run end-to-end — judge/promptfooconfig.yaml," \
"judge/trace_provider.js and judge/tests_from_traces.js exist (this PR) but are unvalidated" \
"against real traces, since none exist yet (report §8 step 10 wires this for real once" \
"steps 8-9 land)."
}

stage_gate() {
  local results="$REPORT_DIR/results.json"
  [[ -f "$results" ]] || fail "no $results — run the score-l1 and judge stages first (neither" \
"is implemented yet; nothing to gate). The pass/fail bar this stage will apply (report §6.5):" \
"1) >=98% Layer-1 on worship-crud, 2) >=90% Layer-1 on bible-authoring (zero verse-text" \
"corruption on verbatim quotes; self-correction within 3 retries), 3) >=85% Layer-2 judge pass" \
"on the Bible rubric majority-of-3 with no single criterion below 75%, 4) no worse than 10" \
"points below the claude-opus-4-6 reference on the combined score. Any tier failing must name" \
"which case category failed, per §6.5's 'not \"73% overall\"' requirement."
}

case "$STAGE" in
  drive) stage_drive ;;
  score-l1) stage_score_l1 ;;
  judge) stage_judge ;;
  gate) stage_gate ;;
  all)
    stage_drive
    stage_score_l1
    stage_judge
    stage_gate
    ;;
  *) fail "unknown --stage '$STAGE' (expected: all, drive, score-l1, judge, gate)" ;;
esac
