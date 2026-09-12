---
paths:
  - "crates/presenter-server/src/router/ai.rs"
  - "crates/presenter-server/src/router/ai_env.rs"
  - "crates/presenter-server/src/ai/mod.rs"
  - "crates/presenter-server/src/ai/client.rs"
  - ".github/workflows/deploy.yml"
  - ".github/workflows/pipeline.yml"
  - ".github/workflows/release.yml"
  - "scripts/deploy/presenter.service"
  - "scripts/deploy/presenter-dev.service"
---

# AI backend config — env precedence + `/etc/presenter/ai.env` (#761, bundled proxy removed #762)

**Why this exists:** the AI assistant was switched from the on-device CLIProxyAPI
proxy + Claude OAuth to **OpenRouter** (API key + model via env). The switch
would have been silently inert without the precedence fix below. #762 then
REMOVED the bundled proxy + Claude OAuth entirely — the effective `apiUrl` is now
simply env → DB → default `https://openrouter.ai/api/v1`, with no bundled-proxy
classification and no `requiresClaudeAuth`.

## Rules

- **`PRESENTER_AI_{API_URL,API_KEY,MODEL}` WIN over the persisted DB `ai-settings`
  row — but ONLY on the EFFECTIVE call/status path.** The override is applied in
  `resolve_effective_settings` (`router/ai.rs`) via
  `router::ai_env::apply_env_overrides`, NEVER in `get_settings_internal`. The
  persist/display path (`get_settings`/`update_settings`) stays RAW so env never
  leaks into a saved or displayed DB row — leaking it would let the next ordinary
  "open Settings → Save" rewrite the stored `apiUrl` to the env value (the
  #679/#683 data-loss class). The root cause the precedence fixes:
  `get_settings_internal` reads the DB row first and only falls back to the
  env-aware `AiSettings::default()` when NO row exists, and every prod DB
  (SNV/PP/dev) still holds a pre-migration row pinning `apiUrl` to the old
  bundled proxy (`http://127.0.0.1:18787/v1`), so env was ignored.

- **Precedence helpers are PARAMETERIZED on the override values, not reading env
  themselves** (`apply_settings_overrides(settings, url, key, model)`), so they
  are unit-testable WITHOUT mutating process-global env — a mutated env var races
  every other test in this binary reading the same key (same rationale as
  `parse_idle_clear_minutes`). `apply_env_overrides` is the thin env-reading
  production wrapper. An empty env value (`…=`) maps to `None` (no override), so a
  deploy that clears the key does not send an empty key.

- **Deploy writes `/etc/presenter/ai.env` (0600 root-only), never the committed
  unit.** All three deploy workflows (`deploy.yml` SNV, `pipeline.yml` dev,
  `release.yml` PP) carry a "Configure AI backend" step modeled on "Configure TURN
  credentials": the API key is passed via the `printf` builtin over ssh stdin
  (never argv) and written under `umask 077`. An empty `OPENROUTER_API_KEY` secret
  REMOVES the file (AI cleanly off / DB fallback), never half-configured. The
  committed units reference it as `EnvironmentFile=-/etc/presenter/ai.env` (leading
  `-` = optional). `release.yml` has no TURN step, so its AI step does its own
  `install -d /etc/presenter`. The step runs BEFORE the service (re)start so the
  started service reads the fresh file. Model changes in prod = edit the `AI_MODEL`
  GH Actions variable + redeploy, no code change.

- **Model slug must exist in the backend's `/models` catalog** or the post-deploy
  `modelValid` gate (#661) hard-fails. Verify a new slug against the public
  `https://openrouter.ai/api/v1/models` before committing. `DEFAULT_AI_MODEL`
  (`ai/mod.rs`) is the fallback when `PRESENTER_AI_MODEL` is unset — since #761 an
  OpenRouter slug (`google/gemini-3.8-flash`), pinned by a regression test.

- **OpenRouter attribution headers** (`HTTP-Referer`, `X-Title: Presenter`) are
  sent unconditionally on every outbound AI request builder in `ai/client.rs`
  (chat + `list_models`) — harmless on any other OpenAI-compatible backend.
