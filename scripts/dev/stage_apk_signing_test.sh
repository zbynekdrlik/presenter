#!/usr/bin/env bash
set -euo pipefail
#
# #740 regression guard — the Presenter Stage APK must be signed with a STABLE,
# committed debug keystore so `adb install -r` succeeds across CI builds.
#
# Before #740 the debug build had no `signingConfig`, so each ephemeral CI runner
# auto-generated a FRESH debug keystore per run → every APK was signed with a
# different signer certificate → `adb install -r` failed
# INSTALL_FAILED_UPDATE_INCOMPATIBLE on every upgrade, tearing the running stage
# app down. This guard asserts the build is wired to a single committed key and
# (optionally) that a built APK is actually signed with it.
#
# Usage:
#   scripts/dev/stage_apk_signing_test.sh              # source-config check (local + CI pre-build)
#   scripts/dev/stage_apk_signing_test.sh <apk-path>   # also verify the BUILT apk's signer cert (CI post-build)
#

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
KEYSTORE="$REPO_ROOT/android/presenter-stage/debug.keystore"
GRADLE="$REPO_ROOT/android/presenter-stage/app/build.gradle.kts"
STOREPASS="android"
ALIAS="androiddebugkey"

# Pinned SHA-256 of the committed debug keystore's signing certificate. The bug
# #740 fixes is that this fingerprint used to change on every CI build, so pinning
# the exact stable key is the on-point regression assertion. If the key is ever
# legitimately rotated (e.g. near its 2054 expiry), regenerate the keystore and
# update this pin in the SAME commit.
EXPECTED_SHA256="AF:84:96:84:32:54:8D:E5:55:A0:27:FC:4A:86:FA:06:5A:F1:23:03:73:6C:8F:01:1D:95:6A:9F:64:98:69:78"

fail() {
    echo "FAIL (#740 APK-signing guard): $1" >&2
    exit 1
}

command -v keytool >/dev/null 2>&1 || fail "keytool not found (needs a JDK)"

want="$(printf '%s' "$EXPECTED_SHA256" | tr -d '[:space:]')"

# (a) the committed keystore must exist — otherwise the build falls back to a
#     fresh per-run auto-generated debug key (the bug).
[ -f "$KEYSTORE" ] || fail "committed keystore missing at android/presenter-stage/debug.keystore"

# (a) and its signing certificate must be the pinned stable key.
ks_out="$(keytool -list -v -keystore "$KEYSTORE" -storepass "$STOREPASS" -alias "$ALIAS" 2>/dev/null || true)"
ks_fp="$(printf '%s\n' "$ks_out" | sed -n 's/.*SHA256: //p' | tr -d '[:space:]')"
[ -n "$ks_fp" ] || fail "could not read the committed keystore's SHA-256 (wrong password/alias?)"
[ "$ks_fp" = "$want" ] || fail "committed keystore fingerprint drifted: got $ks_fp, expected $want"

# (b) the build must point the debug signingConfig at that committed keystore.
grep -q 'rootProject.file("debug.keystore")' "$GRADLE" \
    || fail "build.gradle.kts does not point the debug signingConfig at the committed debug.keystore"
grep -q 'getByName("debug")' "$GRADLE" \
    || fail "build.gradle.kts has no debug signingConfig"

echo "OK: committed debug keystore present, fingerprint pinned, build.gradle references it"

# (c) optional — verify the BUILT apk is actually signed with the same stable key.
# `keytool -printcert -jarfile` reads the v1 (JAR/META-INF) signature. AGP keeps
# v1 signing on by default here because minSdk=22 (<24); if minSdk is ever raised
# to >=24, AGP may drop v1 by default and this check would false-FAIL (a fail-safe
# direction — never a false pass) — switch to `apksigner verify --print-certs` then.
if [ "${1:-}" != "" ]; then
    APK="$1"
    [ -f "$APK" ] || fail "apk not found at $APK"
    apk_out="$(keytool -printcert -jarfile "$APK" 2>/dev/null || true)"
    apk_fps="$(printf '%s\n' "$apk_out" | sed -n 's/.*SHA256: //p' | tr -d '[:space:]')"
    case $'\n'"$apk_fps"$'\n' in
    *$'\n'"$want"$'\n'*)
        echo "OK: built APK $APK is signed with the pinned stable key"
        ;;
    *)
        fail "built APK $APK is NOT signed with the pinned stable key (got: ${apk_fps:-<none>})"
        ;;
    esac
fi
