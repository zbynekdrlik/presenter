#!/usr/bin/env bash
# Regression guard for #393: scripts/ops/bootstrap-host.sh MUST generate adb's
# RSA keypair at $HOME/.android/adbkey when it is absent.
#
# presenter.service runs hardened with ProtectHome=read-only and
# ReadWritePaths=/opt/presenter* only (no ~/.android), so adb's first
# `start-server` cannot mkdir ~/.android to generate its keypair and the server
# never ACKs ("ADB server didn't ACK"). A freshly-provisioned controller (or one
# whose ~/.android was wiped) therefore cannot bootstrap adb from the hardened
# service. Generating the key during bootstrap — when HOME is still writable —
# fixes the cold-start race without widening the service's writable surface.
#
# `adb keygen FILE` is NOT idempotent: it overwrites FILE with a fresh key on
# every invocation, which would clobber an already-keyed host's established
# device authorizations. So the bootstrap step MUST be guarded to run keygen
# only when ~/.android/adbkey does not already exist — leaving prod/dev (which
# already have a key) untouched.
#
# This test FAILS if the keygen step is removed, if it stops targeting
# ~/.android/adbkey, or if the absent-only guard is dropped.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOOTSTRAP="$SCRIPT_DIR/../../scripts/ops/bootstrap-host.sh"

fail=0

if [ ! -f "$BOOTSTRAP" ]; then
    echo "FAIL: bootstrap script not found at $BOOTSTRAP" >&2
    exit 1
fi

# 1. The script must invoke `adb keygen` targeting the .android/adbkey path.
if ! grep -Eq 'adb[[:space:]]+keygen[[:space:]]+.*\.android/adbkey' "$BOOTSTRAP"; then
    echo "FAIL (#393): scripts/ops/bootstrap-host.sh must run" >&2
    echo "             'adb keygen \"\$HOME/.android/adbkey\"' so a freshly" >&2
    echo "             provisioned controller can bootstrap adb under the" >&2
    echo "             hardened (ProtectHome=read-only) presenter.service." >&2
    fail=1
fi

# 2. The keygen MUST be guarded so it only runs when the key is absent (adb
#    keygen overwrites an existing key, which would break a keyed host). Assert
#    the script tests for the absence of the adbkey file near the keygen call.
if ! grep -Eq '! *-(f|e) .*\.android/adbkey|-(f|e) .*\.android/adbkey.*\|\||\[\[ *! *-(f|e)' "$BOOTSTRAP"; then
    echo "FAIL (#393): the adb keygen step must be guarded to run only when" >&2
    echo "             \$HOME/.android/adbkey is ABSENT (e.g. 'if [ ! -f ... ]')." >&2
    echo "             'adb keygen' overwrites an existing key, which would" >&2
    echo "             clobber an already-keyed host's device authorizations." >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "Shell tests for scripts/ops/bootstrap-host.sh (adb keygen): FAILED" >&2
    exit 1
fi

echo "Shell tests for scripts/ops/bootstrap-host.sh (adb keygen): all passed"
