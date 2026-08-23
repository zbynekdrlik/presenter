---
paths:
  - "android/presenter-stage/**"
  - "scripts/dev/stage_apk_signing_test.sh"
---

# Presenter Stage APK is signed with a COMMITTED stable debug key (#740)

`android/presenter-stage/app/build.gradle.kts` points the `debug` `signingConfig` at a
COMMITTED keystore, `android/presenter-stage/debug.keystore` (standard Android debug creds:
alias `androiddebugkey`, store/key password `android`, valid to 2054). This is deliberate and
required: without an explicit `storeFile`, Gradle auto-generates a FRESH debug keystore per
run on the ephemeral CI runners, so every APK is signed with a different certificate →
`adb install -r` fails `INSTALL_FAILED_UPDATE_INCOMPATIBLE` on the stage TVs and the watchdog
tears the running app down (the #740 root cause; #734 fixed the tear-down harm).

## Do NOT

- Do NOT delete `debug.keystore` or revert the `signingConfigs { getByName("debug") { storeFile = … } }`
  block to "fix" a keystore-commit warning. `hooks/block-sensitive-staging.sh` blocks
  `git add` of a `*.keystore` — that block is EXPECTED here; a debug keystore is not a secret
  (universal creds, LAN-only WebView shell, never store-distributed). Re-stage it with the
  logged bypass `# airuleset:secret-ok <reason>` on the `git add` AND the `git commit`.
- Do NOT raise `minSdk` to ≥24 without switching the guard's APK check from
  `keytool -printcert -jarfile` (v1/JAR sig) to `apksigner verify --print-certs` — AGP may
  drop v1 signing by default then and the guard would false-FAIL (fail-safe, but noisy).

## The regression guard

`scripts/dev/stage_apk_signing_test.sh` asserts the committed keystore exists, its cert
SHA-256 matches the pinned value, and `build.gradle.kts` references it; given a built APK path
it also asserts the APK's signer cert == the pin. It runs in `pipeline.yml`'s build-apk job
(JDK 17 → `keytool` present) before + after `gradle assembleDebug`. If the key is ever
legitimately rotated (near the 2054 expiry), regenerate the keystore and update the pinned
`EXPECTED_SHA256` in the SAME commit. Only `pipeline.yml` runs the guard — `deploy.yml`/`release.yml`
build the same pipeline-gated config, so signing is deterministic there via the explicit `storeFile`.

## First deploy after a key change

TVs holding an APK signed with the old key still won't match once → one final uninstall+reinstall
(handled by #734's watchdog; and with `EXPECTED_STAGE_APK_VERSION_CODE` unbumped the watchdog
treats the installed copy as up-to-date and won't even attempt it until a real versionCode bump).
From then on all builds share the committed key → clean `adb install -r` upgrades forever.
