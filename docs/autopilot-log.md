# Autopilot Log

Terse per-issue record of autonomous cycles (issue #, commits, tests, decisions).

---

## 2026-06-16 — #374 CI: function-length gate self-test (regression guard for abs/rel no-op)

- **Scope (rescoped by ticket-validator):** only the remaining-open item — add a self-test so the function-length gate's abs/rel no-op bug can't silently return. Gate fix (`5f5fe9f`) + the 4 over-cap NDI/WebRTC fns were already resolved (PR #368); not re-touched.
- **Version:** 0.4.131 → **0.4.132** (commit `18849e2`), after merging main into dev (FF to `58dbe9b`).
- **Implementation:**
  - Extracted the embedded checker out of `quality-check.sh` §7 into standalone `scripts/dev/fn_length_check.py` (identical behavior) so the test exercises the exact CI code (no drift). Commit `f7960d8`.
  - Added the self-test, then aligned it to the repo's `tests/ci/*.test.sh` convention → **`tests/ci/fn-length-gate.test.sh`**, run via the Test job's existing "Run CI shell tests" step. Commit `4b11f37`.
- **RED→GREEN proof:** test passes against the real (fixed) checker; reintroducing the abs/rel bug (`rel`→`path` in the relpath compare) makes the test FAIL (exit 1); restoring it passes again. Asserts: gate fires on a relative-path over-cap fixture, buggy abs-path variant no-ops, absolute target no-ops, unscoped scan fires.
- **PR #399** dev→main, merged `2cf145a`. Dev pipeline all green (incl. self-test step); deploy-dev verified `/healthz` = 0.4.132. Main Deploy → prod.
- **Side findings (filed):** `#404` — prod android stage relaunch fails (Fully Kiosk `launchComponent` activity missing on sd1-sd4; TVs still show prod `/stage` via TCL browser). Verified dev presenter has ZERO android-stage config (no prod/dev watchdog conflict); cleaned up stray dev2 adb client connections to sd1-sd4.
- **NDI audit (user concern):** current `dev` confirmed intact — every NDI stutter-debug experiment (VP8/480p/sw-encode/silent-audio/playout-delay/RTCP-SR/lite/diag) fully reverted, zero residue; Resolume/audio/HW-H264/30s-reaper/watchdogs all live (verified by grep + clean cargo check/clippy).
