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

- **The AI verdict is a LIVE bounded probe, not an on-disk cache.** `evaluate_ai_status`
  does one `list_models` round trip bounded to 3s by `ai::client::connectivity_client`.
  An API-key backend has no on-disk freshness signal to read, so a cache is not an
  option; the 3s bound is what keeps `/healthz` from hanging. `render_ai_health` is
  best-effort — a settings/DB error folds into `connected:false` + a generic string,
  never a failed readiness probe. If the watchdog polls very often (OpenRouter `/models`
  has rate limits / cost), add a short-TTL cache in `evaluate_ai_status` rather than
  dropping the probe.

- **Put new AI-status render/format code in `router/ai_health.rs`, not `router/ai.rs`.**
  `router/ai.rs` is already past the 800-line warning cap (under the 1000 hard-fail);
  grow the small sibling module instead (project file-line gate, `quality-gates.md`).

- **Deploy gates read `/ai/status` (not `/healthz`), right after deploy.** `modelValid:false`
  is a HARD-fail (`exit 1`); `connected:false` is an owner-visible `::error::` that does
  NOT fail the job (a dead AI login must not block unrelated deploys) — never a
  `::warning::` nobody reads. The three workflows (deploy/pipeline/release) must stay
  consistent. The `::error::`-without-`exit 1` is intentionally non-blocking and is NOT a
  `continue-on-error` violation (the else-branch is a bare `echo`, exit 0 under `set -e`).
