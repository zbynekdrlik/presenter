---
paths:
  - "crates/presenter-server/src/router/ai.rs"
  - "crates/presenter-server/src/router/ai_health.rs"
  - "crates/presenter-server/src/router.rs"
---

# AI status on `/healthz` + `/ai/status` — one shared computation (#760)

**Why this exists:** the production AI assistant sat dead 14 days on SNV because no
signal about a dead AI backend ever left the box (journal WARN + operator chip +
deploy `::warning::` — none externally pollable). `/healthz` now carries an `ai`
object so an EXTERNAL watchdog can detect it in minutes.

## Rules

- **`/ai/status` and `/healthz.ai` share ONE computation — `router::ai::evaluate_ai_status`.**
  Never duplicate the connectivity/model-validity logic. The `/ai/status` handler
  (`check_status`) is a thin wrapper over it; `/healthz` renders a subset via
  `router::ai_health::render_ai_health`. If you change the AI status verdict, change
  it in `evaluate_ai_status` only.

- **`/healthz.ai` is backend-agnostic: `{connected, error, model}`.** No OAuth-specific
  fields (`claudeAuthenticated`/`tokenExpiresAt`/`expiryWarning` were deliberately NOT
  used — #662 migrates the backend from Claude OAuth/CLIProxyAPI to OpenRouter API-key).
  `connected`/`error` come from `compute_ai_connected`/`compute_ai_status_error`, whose
  OAuth input (`claude_authenticated`) participates ONLY when `requires_claude_auth`
  (bundled proxy). Keep any new field backend-neutral.

- **`/healthz.ai` is served through a per-`AppState` stale-while-revalidate CACHE
  (`crate::ai::health_cache::AiHealthCache`, 30s TTL), NOT a live probe per request.**
  WHY: `/healthz` is polled by EVERY open operator tab (the header version poll in
  `presenter-ui`), the stage NDI reload guard, `version_label` on mount, AND the deploy
  gates — a live `/models` probe per hit would scale the readiness probe's latency and
  external-request rate with the number of open tabs and couple it to the AI backend's
  response time (the #760 rework finding). The cache (`get_ai_health`): fresh → return
  with ZERO awaits (no network on the hot path); stale → return the stale value
  immediately and refresh in the BACKGROUND via `tokio::spawn` (at most ONE in-flight
  refresh, guarded by an `AtomicBool` so N concurrent hits never spawn N probes); empty
  (cold) → one bounded inline probe. A failed refresh (`produce` returns `None`) KEEPS
  the previous value AND resets its freshness timestamp (`at = now`) so the next probe
  waits a FULL TTL — otherwise a persistently-failing producer leaves the entry stale and
  every hit re-triggers a probe back-to-back, breaking "at most once per TTL" during an
  outage. The cold-start failure is likewise cached (`cold_failure()` with `at = now`) so a
  dead backend at startup also probes once per TTL, not once per hit. Recovery is detected
  within one TTL either way.
  The producer wraps the shared `evaluate_ai_status` (still one bounded 3s `list_models`
  round trip, run at most once per TTL). Scope the cache PER `AppState` (an `Arc` field),
  NEVER a module-level `static` — the test suite builds many `AppState`s in one process
  and a global would cross-contaminate tests. `/ai/status` stays LIVE (the operator chip
  wants freshness) — only `/healthz` reads through the cache.

- **Clear the single-flight `refreshing` flag from an RAII `Drop` guard, never a manual
  tail `store(false)`.** If the producer ever panicked, a manual reset would be skipped and
  the flag would wedge `true` forever — silently freezing this always-polled readiness
  endpoint's verdict (no more probes ever). `RefreshGuard` resets on drop, so an unwind on
  either the cold-inline path or the swallowed spawned-task path still clears it. This
  removes the "correctness depends on the producer never panicking" coupling.

- **Put new AI-status render/format code in `router/ai_health.rs`, not `router/ai.rs`.**
  `router/ai.rs` is already past the 800-line warning cap (under the 1000 hard-fail);
  grow the small sibling module instead (project file-line gate, `quality-gates.md`).

- **Deploy gates read `/ai/status` (not `/healthz`), right after deploy.** `modelValid:false`
  is a HARD-fail (`exit 1`); `connected:false` is an owner-visible `::error::` that does
  NOT fail the job (a dead AI login must not block unrelated deploys) — never a
  `::warning::` nobody reads. The three workflows (deploy/pipeline/release) must stay
  consistent. The `::error::`-without-`exit 1` is intentionally non-blocking and is NOT a
  `continue-on-error` violation (the else-branch is a bare `echo`, exit 0 under `set -e`).
