#!/usr/bin/env bash
# Regression guard for #392: scripts/ops/bootstrap-host.sh MUST install adb.
#
# adb is a hard runtime dependency of the Android Stage Launcher
# (crates/presenter-server/src/android_stage.rs falls back to `adb` on PATH and
# spawns it in adb_connect/adb_launch/adb_foreground_package). When the
# provisioning script does not install it, an un-provisioned controller's stage
# launchers silently fail. The deploy-time installs (deploy.yml / pipeline.yml /
# release.yml) only cover the deploy hosts — a freshly bootstrapped controller
# must already have adb. This test FAILS if the package is dropped from the
# bootstrap script's APT_PACKAGES list again.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOOTSTRAP="$SCRIPT_DIR/../../scripts/ops/bootstrap-host.sh"

fail=0

if [ ! -f "$BOOTSTRAP" ]; then
    echo "FAIL: bootstrap script not found at $BOOTSTRAP" >&2
    exit 1
fi

# The Debian/Ubuntu adb package is `android-tools-adb` (provides /usr/bin/adb).
# Assert it appears inside the APT_PACKAGES=( … ) array literal, not merely
# anywhere in the file (so a stray comment can't make the gate pass).
pkg_block="$(awk '/^APT_PACKAGES=\(/{flag=1} flag{print} /^\)/{if(flag)exit}' "$BOOTSTRAP")"

if [ -z "$pkg_block" ]; then
    echo "FAIL: could not locate APT_PACKAGES=( … ) array in $BOOTSTRAP" >&2
    exit 1
fi

# Match the package as a standalone array element (its own line, allowing a
# trailing inline comment), so a substring elsewhere does not satisfy the gate.
if ! grep -Eq '^[[:space:]]*android-tools-adb([[:space:]]+#.*)?[[:space:]]*$' <<<"$pkg_block"; then
    echo "FAIL (#392): scripts/ops/bootstrap-host.sh APT_PACKAGES must include" >&2
    echo "             'android-tools-adb' — adb is a hard runtime dependency of" >&2
    echo "             the Android Stage Launcher. Un-provisioned controllers" >&2
    echo "             silently fail their stage launchers without it." >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "Shell tests for scripts/ops/bootstrap-host.sh (adb): FAILED" >&2
    exit 1
fi

echo "Shell tests for scripts/ops/bootstrap-host.sh (adb): all passed"
