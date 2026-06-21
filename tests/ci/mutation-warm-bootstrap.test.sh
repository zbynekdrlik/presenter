#!/usr/bin/env bash
# Regression guard for #439: the diff-scoped mutation PR gate must bootstrap its
# `mutants`-profile build cache via a dedicated `mutation-warm` job that the
# sharded `mutation` jobs depend on.
#
# Without it, every shard cold-builds the heavy dep graph (gstreamer/NDI/leptos/
# seaorm, ~15-19 min) and is cancelled by the 20-min JOB-level timeout BEFORE
# rust-cache's post-step can save the cache (a job-level timeout skips
# post-actions). So the `mutants` cache never warms and every run cold-builds
# forever — a permanent deadlock (#430 "fixed" but ineffective; reopened as
# #439). The dedicated warm job builds the deps ONCE under a generous cap,
# succeeds, and saves the cache; the shards then restore it warm and fit the cap.
#
# This test FAILS if the warm job is removed, stops building the mutants profile,
# loses its longer-than-shard cap, or the shards stop depending on it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WF="$ROOT/.github/workflows/pipeline.yml"

if [ ! -f "$WF" ]; then
  echo "FAIL: $WF not found" >&2
  exit 1
fi

python3 - "$WF" <<'PY'
import sys, yaml

wf = yaml.safe_load(open(sys.argv[1]))
jobs = wf.get("jobs", {})
errs = []

# 1. The bootstrap job must exist.
warm = jobs.get("mutation-warm")
if warm is None:
    errs.append("missing 'mutation-warm' job (mutants-cache bootstrap)")
else:
    # 2. It must build the mutants profile.
    runs = " ".join(s.get("run", "") for s in warm.get("steps", []))
    if "--profile mutants" not in runs:
        errs.append("'mutation-warm' must build the mutants profile "
                    "(cargo build --profile mutants ...)")
    # 3. Its cap must exceed the 20-min shard cap (cold build is ~15-19 min).
    try:
        cap = int(warm.get("timeout-minutes", 0))
    except (TypeError, ValueError):
        cap = 0
    if cap <= 20:
        errs.append("'mutation-warm' timeout-minutes must exceed the 20-min "
                    f"shard cap (got {cap!r})")

# 4. The sharded mutation job must depend on the warm job.
mut = jobs.get("mutation")
if mut is None:
    errs.append("missing 'mutation' job")
else:
    needs = mut.get("needs", [])
    if isinstance(needs, str):
        needs = [needs]
    if "mutation-warm" not in needs:
        errs.append("'mutation' shards must `needs: mutation-warm` to restore "
                    f"the warm cache (needs={needs!r})")

if errs:
    print("FAIL: mutation-warm bootstrap not wired correctly:", file=sys.stderr)
    for e in errs:
        print(f"  - {e}", file=sys.stderr)
    sys.exit(1)

print("OK: mutation-warm bootstrap wired correctly")
PY
