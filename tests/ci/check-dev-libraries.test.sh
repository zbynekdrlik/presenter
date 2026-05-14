#!/usr/bin/env bash
# Regression tests for scripts/ci/check-dev-libraries.sh.
# Exercises the count-mismatch detection that issue #229 reported missing.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SCRIPT_DIR/../../scripts/ci/check-dev-libraries.sh"
WORK=$(mktemp -d)
trap "rm -rf $WORK" EXIT

fail=0

# Case 1: disk count matches DB count → script exits 0
mkdir -p "$WORK/case1/Library_A" "$WORK/case1/Library_B" "$WORK/case1/LibraryData"
echo '[{"name":"Library_A"},{"name":"Library_B"},{"name":"Bible"}]' > "$WORK/case1.json"
if ! "$SCRIPT" "$WORK/case1" "$WORK/case1.json" > /dev/null; then
    echo "FAIL case1: matching counts should succeed" >&2
    fail=1
fi

# Case 2 — THE BUG SCENARIO from #229: many on disk, only seed in DB → fail
mkdir -p "$WORK/case2/Library_A" "$WORK/case2/Library_B" "$WORK/case2/Library_C" "$WORK/case2/Library_D"
echo '[{"name":"Sample Library"},{"name":"Bible"}]' > "$WORK/case2.json"
if "$SCRIPT" "$WORK/case2" "$WORK/case2.json" > /dev/null 2>&1; then
    echo "FAIL case2: seed-only DB with 4 disk libs must fail (the #229 bug)" >&2
    fail=1
fi

# Case 3: DB has more rows than disk → fail
mkdir -p "$WORK/case3/Library_A"
echo '[{"name":"Library_A"},{"name":"Library_B"},{"name":"Bible"}]' > "$WORK/case3.json"
if "$SCRIPT" "$WORK/case3" "$WORK/case3.json" > /dev/null 2>&1; then
    echo "FAIL case3: extra DB rows must fail" >&2
    fail=1
fi

# Case 4: Bible is excluded from both sides → matching pure non-Bible counts pass
mkdir -p "$WORK/case4/Library_A" "$WORK/case4/LibraryData"
echo '[{"name":"Library_A"},{"name":"bible"},{"name":"BIBLE"}]' > "$WORK/case4.json"
if ! "$SCRIPT" "$WORK/case4" "$WORK/case4.json" > /dev/null; then
    echo "FAIL case4: Bible (any case) must be excluded from DB count" >&2
    fail=1
fi

# Case 5: missing disk path → script exits non-zero with clear message
if "$SCRIPT" "$WORK/does-not-exist" "$WORK/case1.json" > /dev/null 2>&1; then
    echo "FAIL case5: missing disk path must fail" >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "Shell tests for scripts/ci/check-dev-libraries.sh: FAILED" >&2
    exit 1
fi

echo "Shell tests for scripts/ci/check-dev-libraries.sh: all passed"
