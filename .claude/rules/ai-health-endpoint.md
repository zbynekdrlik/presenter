---
paths:
  - "crates/presenter-server/src/router/ai.rs"
  - "crates/presenter-server/src/router/ai_health.rs"
  - "crates/presenter-server/src/router.rs"
  - "crates/presenter-server/src/ai/last_error.rs"
  - "crates/presenter-server/src/ai/agent.rs"
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
  fields (#662 migrated the backend from Claude OAuth/CLIProxyAPI to OpenRouter API-key;
  #762 removed the bundled proxy + OAuth entirely). `connected`/`error` come from
  `compute_ai_connected(connectivity_ok, model_valid)` / `compute_ai_status_error` — pure
  connectivity + model-validity signals, no auth input. Keep any new field backend-neutral.

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

- **`list_models` (`GET /models`) is NOT a liveness proof for a METERED backend (#764).**
  A metered API-key backend (OpenRouter, #761) serves `/models` 200 even when the workspace
  budget is exhausted, the key is revoked, or completions 402/429 — so a probe-only verdict
  reports a false `connected:true` while every real `POST /ai/chat` 403s ("Workspace daily
  budget … exceeded"). The `connected` verdict therefore ALSO folds in the most recent REAL
  completion outcome: `ai::agent::run_agent` records each completion's success/failure into
  the per-`AppState` `ai::last_error::AiCallHealth` (redacted, ≤200-char excerpt + `Instant`),
  and `evaluate_ai_status` applies `router::ai_health::apply_last_completion_failure` AFTER the
  probe — a failure within `AI_LAST_FAILURE_WINDOW` (15 min) flips a falsely-green verdict to
  `connected:false` with the backend message; a later success clears it; the window expires so
  a transient outage the operator stops exercising doesn't pin a permanent false `false`. The
  fold only ever flips `connected:true → false`, never overwrites an already-`false` probe's
  more specific error (invalid model / connectivity). Redact any completion excerpt with the
  shared `ai::redact::redact_proxy_output_line` (covers OpenRouter `sk-or-v1-` keys, #764;
  relocated from the deleted `proxy_output_relay` in #762) BEFORE it reaches
  `/healthz`/`/ai/status`. The three deploy gates fire ONE cheap real completion (`POST
  /ai/chat` + `POST /ai/clear`) before the `/ai/status` read so a budget/credit/key outage is
  caught at deploy time, not only at first operator use — reusing the existing non-blocking
  `connected:false` `::error::`, no SSE parsing needed. The operator chip
  (`presenter-ui/components/ai_status.rs`) reads the flat `connected`/`error` fields directly
  (since #762 there is no nested `proxy.*` — an `unavailable` state, label "AI: nedostupné",
  surfaces any `connected:false` outage).
