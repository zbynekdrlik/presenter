#!/usr/bin/env bash
# Regression guard for #433 (+ the #392 release.yml twin): the
# "Ensure ADB is installed" remote heredoc in BOTH deploy.yml and release.yml
# MUST start with `set -e`.
#
# Without `set -e`, a failed `apt-get install android-tools-adb` is swallowed:
# the step's trailing `echo "ADB version: …"` returns 0 regardless, so the deploy
# passes silently with adb still absent and the Android Stage Launcher fails on
# the live host. release.yml was fixed in 47d466a (#392 review); deploy.yml is
# its twin (#433). This test FAILS if either ADB-install heredoc loses `set -e`.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/../.."

fail=0

# Assert that, in <workflow>, the line immediately after the ADB-install
# heredoc opener (`ssh … << 'REMOTE_SCRIPT'`) following the
# "Ensure ADB is installed" step is exactly `set -e` (allowing surrounding
# whitespace). Scanning the step block (not the whole file) prevents an
# unrelated `set -e` elsewhere from satisfying the gate.
assert_set_e() {
    local workflow="$1"
    local path="$ROOT/.github/workflows/$workflow"

    if [ ! -f "$path" ]; then
        echo "FAIL: workflow not found at $path" >&2
        fail=1
        return
    fi

    # Pull the ADB-install step block: from the step name to the heredoc
    # terminator (REMOTE_SCRIPT on its own line).
    local block
    block="$(awk '
        /- name: Ensure ADB is installed/ {flag=1}
        flag {print}
        flag && /^[[:space:]]*REMOTE_SCRIPT[[:space:]]*$/ {exit}
    ' "$path")"

    if [ -z "$block" ]; then
        echo "FAIL ($workflow): could not locate the 'Ensure ADB is installed' step" >&2
        fail=1
        return
    fi

    # The first line after the heredoc opener must be `set -e`.
    local after_opener
    after_opener="$(awk '
        /<< '\''REMOTE_SCRIPT'\''/ {found=1; next}
        found {print; exit}
    ' <<<"$block")"

    if ! grep -Eq '^[[:space:]]*set -e[[:space:]]*$' <<<"$after_opener"; then
        echo "FAIL (#433): .github/workflows/$workflow 'Ensure ADB is installed'" >&2
        echo "             heredoc must start with 'set -e' so a failed adb install" >&2
        echo "             fails the deploy loudly. Got first line: '$after_opener'" >&2
        fail=1
    fi
}

assert_set_e deploy.yml
assert_set_e release.yml

if [ "$fail" -ne 0 ]; then
    echo "Shell tests for ADB-install set -e gate: FAILED" >&2
    exit 1
fi

echo "Shell tests for ADB-install set -e gate (deploy.yml + release.yml): all passed"
