#!/usr/bin/env bash
set -euo pipefail

# Quality & Architecture checks aligned with Issue #41 (2025 Baseline)
# Usage: scripts/dev/quality-check.sh [--strict] [--json]

STRICT=0
EMIT_JSON=0
BASE_REF="origin/main"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --strict) STRICT=1; shift ;;
    --json) EMIT_JSON=1; shift ;;
    --against) BASE_REF="$2"; shift 2 ;;
    -h|--help)
      cat <<'USAGE'
Usage: scripts/dev/quality-check.sh [--strict] [--json]

Runs repository policy checks used by Issue #41 (Recurring Quality & Architecture Review).
Fails the build when --strict is provided and any required check does not pass.
USAGE
      exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

failures=()
warnings=()

note() { printf '%s\n' "$*"; }
warn() { warnings+=("$*"); }
fail() { failures+=("$*"); }

# 1) Router feature modules present
for f in crates/presenter-server/src/router/{bible,libraries,playlists,presentations}.rs; do
  [[ -f "$f" ]] || fail "Missing feature router: $f"
done

# 2) router.rs should not contain legacy presentation handlers
if rg -n "(insert_slide_handler|duplicate_slide_handler|delete_slide_handler|reorder_slides_handler|update_slide_content_handler)" crates/presenter-server/src/router.rs >/dev/null; then
  fail "router.rs still contains legacy presentation handler impls; move to router/presentations.rs"
fi

# 3) UI pages present
for f in crates/presenter-server/src/ui/{operator,tablet,bible,settings,home,timer_overlay}.rs; do
  [[ -f "$f" ]] || fail "Missing UI page: $f"
done

# 4) No direct playwright show-report usages (policy requires helper script)
if rg -n "playwright show-report" \
      -g 'scripts/**' -g 'tests/**' -g 'crates/**' -g 'package*.json' \
      -g '!scripts/dev/show-playwright-report.sh' \
      -g '!scripts/dev/quality-check.sh' \
      -g '!docs/**' -g '!prompts/**' -g '!node_modules/**' >/dev/null; then
  fail "Direct 'playwright show-report' usage found; use scripts/dev/show-playwright-report.sh"
fi

# 5) No focused/skipped Playwright tests
if rg -n "\\.(only|skip)\\(" tests/e2e >/dev/null 2>&1; then
  fail "Found focused/skipped E2E tests ('.only' or '.skip')."
fi

# Determine changed files vs base
git fetch -q origin || true
mapfile -t changed < <(git diff --name-only "$BASE_REF"...HEAD 2>/dev/null | rg '^crates/.+\.rs$' || true)

# If no changed list (e.g., shallow), fall back to all files but only warn
target_files=("${changed[@]}")
if (( ${#target_files[@]} == 0 )); then
  mapfile -t target_files < <(rg --files -g 'crates/**/*.rs')
  warn "No diff against $BASE_REF; checking all Rust files in advisory mode"
fi

# 6) File size limits (only for target files)
for file in "${target_files[@]}"; do
  [[ -f "$file" ]] || continue
  # Count production lines only (exclude inline #[cfg(test)] module and below)
  test_line=$(awk '/^\s*#\[cfg\(test\)\]/{print NR; exit}' "$file")
  if [[ -n "${test_line}" ]]; then
    prod_lines=$(( test_line - 1 ))
  else
    prod_lines=$(wc -l < "$file" | tr -d ' ')
  fi
  lines=$prod_lines
  if (( lines > 1000 )); then
    fail "${file} exceeds hard cap (1000 lines): ${lines}"
  elif (( lines > 800 )); then
    warn "${file} exceeds target size (800 lines): ${lines}"
  fi
done

# 7) Function length (naive) — fail > 60 lines
python3 - "$ROOT_DIR" <<'PY' || exit 3
import os, re, sys, json
root = sys.argv[1]
fn_start = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+[A-Za-z0-9_]+\s*\(")
violations = []
targets_env = os.environ.get('QC_TARGETS', '')
targets = [t for t in targets_env.split('\n') if t.strip()]
for dirpath, _, filenames in os.walk(os.path.join(root, 'crates')):
    for name in filenames:
        if not name.endswith('.rs'): continue
        path = os.path.join(dirpath, name)
        if targets and path not in targets:
            continue
        with open(path, 'r', encoding='utf-8') as f:
            lines = f.readlines()
        i = 0
        while i < len(lines):
            if fn_start.match(lines[i]):
                # find first '{'
                j = i
                brace = 0
                started = False
                while j < len(lines):
                    brace += lines[j].count('{')
                    brace -= lines[j].count('}')
                    if '{' in lines[j]:
                        started = True
                    if started and brace == 0:
                        length = j - i + 1
                        if length > 60:
                            violations.append({'file': path, 'start': i+1, 'length': length})
                        i = j
                        break
                    j += 1
            i += 1
violations.sort(key=lambda v: (-v['length'], v['file'], v['start']))
print(json.dumps(violations))
PY
fn_json=$?
if [[ $fn_json -eq 3 ]]; then
  fail "Function length checker crashed"
else
  viols=$(python3 - <<'PY'
import json, sys
data = sys.stdin.read()
print(data)
PY
  )
fi

if [[ -n "${viols:-}" && "${viols}" != "[]" ]]; then
  while IFS= read -r row; do
    file=$(echo "$row" | jq -r '.file')
    start=$(echo "$row" | jq -r '.start')
    length=$(echo "$row" | jq -r '.length')
    fail "Function too long (>60): ${file}:${start} (${length} lines)"
  done < <(echo "$viols" | jq -c '.[]')
fi

# 8) cargo check warnings (non-fatal unless strict policy chosen later)
check_out=$(cargo check 2>&1 || true)
if echo "$check_out" | grep -q "warning:"; then
  warn "cargo check reported warnings; run clippy/fixes when feasible."
fi

# Emit results
if (( EMIT_JSON )); then
  jq -n --argjson failures "$(printf '%s\n' "${failures[@]:-}" | jq -R . | jq -s .)" \
        --argjson warnings "$(printf '%s\n' "${warnings[@]:-}" | jq -R . | jq -s .)" \
        '{failures: $failures, warnings: $warnings}'
else
  if ((${#failures[@]})); then
    echo "[quality] Failures:"; printf '  - %s\n' "${failures[@]}"
  fi
  if ((${#warnings[@]})); then
    echo "[quality] Warnings:"; printf '  - %s\n' "${warnings[@]}"
  fi
fi

if ((${#failures[@]})); then
  if (( STRICT )); then
    exit 1
  fi
fi
exit 0
