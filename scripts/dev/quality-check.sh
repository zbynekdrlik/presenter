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

# File size exemptions per CLAUDE.md:
# - Migration files are declarative schema definitions, exempt from limits
# - Test files are exempt
EXEMPT_FILESIZE_PATTERNS=(
  "*/presenter-migration/src/*.rs"
  "**/tests.rs"
  "**/tests/*.rs"
)

note() { printf '%s\n' "$*"; }
warn() { warnings+=("$*"); }
fail() { failures+=("$*"); }

# 1) Router feature modules present
for f in crates/presenter-server/src/router/{bible,libraries,playlists,presentations}.rs; do
  [[ -f "$f" ]] || fail "Missing feature router: $f"
done

# 2) router.rs should not contain legacy presentation handlers
if command -v rg >/dev/null 2>&1 && rg -n "(insert_slide_handler|duplicate_slide_handler|delete_slide_handler|reorder_slides_handler|update_slide_content_handler)" crates/presenter-server/src/router.rs >/dev/null; then
  fail "router.rs still contains legacy presentation handler impls; move to router/presentations.rs"
fi

# 3) UI pages present
for f in crates/presenter-server/src/ui/{operator,tablet,bible,settings,home,timer_overlay}.rs; do
  [[ -f "$f" ]] || fail "Missing UI page: $f"
done

# 4) No direct playwright show-report usages (policy requires helper script)
if command -v rg >/dev/null 2>&1 && rg -n "playwright show-report" \
      -g 'scripts/**' -g 'tests/**' -g 'crates/**' -g 'package*.json' \
      -g '!scripts/dev/show-playwright-report.sh' \
      -g '!scripts/dev/quality-check.sh' \
      -g '!docs/**' -g '!prompts/**' -g '!node_modules/**' >/dev/null; then
  fail "Direct 'playwright show-report' usage found; use scripts/dev/show-playwright-report.sh"
fi

# 5) No focused/skipped Playwright tests
if command -v rg >/dev/null 2>&1 && rg -n "\\.(only|skip)\\(" tests/e2e >/dev/null 2>&1; then
  fail "Found focused/skipped E2E tests ('.only' or '.skip')."
fi

# 6) No continue-on-error in CI workflows (strict mode)
if command -v rg >/dev/null 2>&1 && rg -n "continue-on-error" .github/workflows/*.yml >/dev/null 2>&1; then
  fail "Found continue-on-error in workflows - remove for strict CI"
fi

# Determine changed files vs base
git fetch -q origin || true
if command -v rg >/dev/null 2>&1; then
  mapfile -t changed < <(git diff --name-only "$BASE_REF"...HEAD 2>/dev/null | rg '^crates/.+\.rs$' || true)
else
  mapfile -t changed < <(git diff --name-only "$BASE_REF"...HEAD 2>/dev/null | grep -E '^crates/.+\.rs$' || true)
fi

# If no changed list (e.g., shallow), fall back to all files but only warn
target_files=("${changed[@]}")
if (( ${#target_files[@]} == 0 )); then
  if command -v rg >/dev/null 2>&1; then
    mapfile -t target_files < <(rg --files -g 'crates/**/*.rs')
  else
    mapfile -t target_files < <(find crates -type f -name '*.rs')
  fi
  warn "No diff against $BASE_REF; checking all Rust files in advisory mode"
fi

# Helper: check if file matches any exempt pattern
is_exempt_file() {
  local file="$1"
  for pattern in "${EXEMPT_FILESIZE_PATTERNS[@]}"; do
    case "$file" in
      $pattern) return 0 ;;
    esac
  done
  return 1
}

# 6) File size limits (only for target files)
for file in "${target_files[@]}"; do
  [[ -f "$file" ]] || continue

  # Skip exempt files (migrations, tests)
  if is_exempt_file "$file"; then
    continue
  fi

  is_changed=0
  for cf in "${changed[@]:-}"; do
    if [[ "$cf" == "$file" ]]; then is_changed=1; break; fi
  done
  # Count production lines only (exclude inline #[cfg(test)] module and below)
  test_line=$(awk '/^\s*#\[cfg\(test\)\]/{print NR; exit}' "$file")
  if [[ -n "${test_line}" ]]; then
    prod_lines=$(( test_line - 1 ))
  else
    prod_lines=$(wc -l < "$file" | tr -d ' ')
  fi
  lines=$prod_lines
  if (( lines > 1000 )); then
    if (( is_changed )); then
      fail "${file} exceeds hard cap (1000 lines): ${lines}"
    else
      warn "${file} exceeds hard cap (1000 lines): ${lines}"
    fi
  elif (( lines > 800 )); then
    warn "${file} exceeds target size (800 lines): ${lines}"
  fi
done

# 7) Function length (naive) — warn > 80, fail > 120 lines
# Provide targets to the checker to scope analysis
if (( ${#target_files[@]} )); then
  QC_TARGETS=$(printf '%s\n' "${target_files[@]}")
else
  QC_TARGETS=""
fi
export QC_TARGETS

fn_check=$(python3 - "$ROOT_DIR" <<'PY'
import os, re, sys, json
root = sys.argv[1]
fn_start = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)\s*\(")

# Exempt patterns per CLAUDE.md:
# - Migration up() functions (declarative schema)
# - render_*_ui functions (Leptos HTML-like DSL)
# - build_router functions (route declarations)
EXEMPT_FN_NAMES = {'up', 'down', 'build_router'}
EXEMPT_FN_PREFIXES = ('render_', )

def is_exempt_function(fn_name, filepath):
    # Migration files - exempt all functions
    if '/presenter-migration/' in filepath:
        return True
    # Specific exempt function names
    if fn_name in EXEMPT_FN_NAMES:
        return True
    # UI render functions
    for prefix in EXEMPT_FN_PREFIXES:
        if fn_name.startswith(prefix):
            return True
    return False

violations = []  # > 120 lines (hard fail)
warnings = []    # > 80 lines (warning)
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
            match = fn_start.match(lines[i])
            if match:
                fn_name = match.group(1)
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
                        if not is_exempt_function(fn_name, path):
                            entry = {'file': path, 'start': i+1, 'length': length, 'fn': fn_name}
                            if length > 120:
                                violations.append(entry)
                            elif length > 80:
                                warnings.append(entry)
                        i = j
                        break
                    j += 1
            i += 1
violations.sort(key=lambda v: (-v['length'], v['file'], v['start']))
warnings.sort(key=lambda v: (-v['length'], v['file'], v['start']))
print(json.dumps({'violations': violations, 'warnings': warnings}))
PY
) || { fail "Function length checker crashed"; fn_check='{"violations":[],"warnings":[]}'; }

# Report hard failures (>120 lines)
viols=$(echo "$fn_check" | jq -c '.violations')
if [[ -n "${viols:-}" && "${viols}" != "[]" ]]; then
  while IFS= read -r row; do
    file=$(echo "$row" | jq -r '.file')
    start=$(echo "$row" | jq -r '.start')
    length=$(echo "$row" | jq -r '.length')
    fn_name=$(echo "$row" | jq -r '.fn')
    fail "Function too long (>120): ${file}:${start} fn ${fn_name} (${length} lines)"
  done < <(echo "$viols" | jq -c '.[]')
fi

# Report warnings (>80 lines)
fn_warns=$(echo "$fn_check" | jq -c '.warnings')
if [[ -n "${fn_warns:-}" && "${fn_warns}" != "[]" ]]; then
  while IFS= read -r row; do
    file=$(echo "$row" | jq -r '.file')
    start=$(echo "$row" | jq -r '.start')
    length=$(echo "$row" | jq -r '.length')
    fn_name=$(echo "$row" | jq -r '.fn')
    warn "Function long (>80): ${file}:${start} fn ${fn_name} (${length} lines)"
  done < <(echo "$fn_warns" | jq -c '.[]')
fi

# 8) Format & Lint (strict)
if ! cargo fmt --all -- --check >/dev/null 2>&1; then
  fail "cargo fmt reported formatting changes (run: cargo fmt --all)"
fi

if ! cargo clippy -p presenter-server --tests --no-deps --quiet -- -D warnings >/dev/null 2>&1; then
  warn "cargo clippy (presenter-server) reported warnings (advisory for this branch; will be enforced next)"
fi

# 9) Dependency and security checks
if command -v cargo-deny >/dev/null 2>&1; then
  if ! cargo deny check >/dev/null 2>&1; then
    fail "cargo-deny policy violations (licenses/bans/duplicates)"
  fi
else
  warn "cargo-deny not installed; skipping (install: cargo install cargo-deny)"
fi

if command -v cargo-audit >/dev/null 2>&1; then
  if ! cargo audit -q >/dev/null 2>&1; then
    fail "cargo-audit found vulnerabilities (update dependencies)"
  fi
else
  warn "cargo-audit not installed; skipping (install: cargo install cargo-audit)"
fi

# 10) Production code hygiene (no unwrap/expect/panic)
if command -v rg >/dev/null 2>&1 && rg -n "\\b(panic!)\\(|\\.unwrap\\(|\\.expect\\\(" \
    -g 'crates/**/src/**/*.rs' \
    -g '!**/tests/**' -g '!**/benches/**' -g '!**/examples/**' \
    >/tmp/qc-nounwrap.txt 2>/dev/null; then
  # Filter out false positives (doc comments / allow lines) – minimal heuristic
  if [[ -s /tmp/qc-nounwrap.txt ]]; then
    fail "Found unwrap/expect/panic in production code (see /tmp/qc-nounwrap.txt)"
  fi
fi

# 11) Async anti-patterns in server (blocking in async paths)
if command -v rg >/dev/null 2>&1 && rg -n "std::thread::sleep|block_on\\(" \
    crates/presenter-server/src \
    -g '!**/tests/**' >/tmp/qc-async-blocking.txt 2>/dev/null; then
  if [[ -s /tmp/qc-async-blocking.txt ]]; then
    fail "Found blocking calls in async paths (see /tmp/qc-async-blocking.txt)"
  fi
fi

# 12) Toolchain pin
if [[ ! -f rust-toolchain.toml ]]; then
  warn "rust-toolchain.toml is missing; pin toolchain for reproducible builds"
fi

# 13) cargo check warnings (advisory)
check_out=$(cargo check 2>&1 || true)
if echo "$check_out" | grep -q "warning:"; then
  warn "cargo check reported warnings; run clippy/fixes when feasible."
fi

# Emit results
if (( EMIT_JSON )); then
  # Serialize bash arrays to JSON arrays correctly (empty arrays => [])
  to_json_array() {
    local -n _arr=$1
    if (( ${#_arr[@]} == 0 )); then
      echo '[]'
    else
      # NUL-delimit to preserve content; drop trailing empty element introduced by printf
      printf '%s\0' "${_arr[@]}" | jq -Rs 'split("\u0000")[:-1]'
    fi
  }
  fail_json=$(to_json_array failures)
  warn_json=$(to_json_array warnings)
  jq -n --argjson failures "$fail_json" --argjson warnings "$warn_json" \
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
