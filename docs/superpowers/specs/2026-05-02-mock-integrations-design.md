# Mock Integrations on Dev — Design

**Date:** 2026-05-02
**Status:** Proposed
**Scope:** `presenter-server` (new feature-gated mock listeners) + dev deploy pipeline (DB rewrite + feature-flagged build)
**Issue:** [#279](https://github.com/zbynekdrlik/presenter/issues/279) — `dev presenter connects to live resolume arena and interferes on production!!!`

## Goal

Stop the dev server from sending live commands to production hardware. The dev binary embeds in-process mocks for the 3 push-style integrations (Resolume HTTP, REAPER OSC, AbleSet) and the dev pipeline rewrites integration tables to point at those mocks instead of production endpoints. Production is unaffected.

## Why

`pipeline.yml:786-810` "Replace dev database with production snapshot" copies prod's `presenter.db` to dev on every deploy so dev mirrors prod data for realistic testing. The snapshot also carries production's `resolume_hosts` rows pointing at prod's Resolume Arena. After dev restarts, it reads those rows, sees `is_enabled=true`, and starts pushing clip triggers, text updates, and timer pings to PROD's Resolume during live worship services.

**Scope analysis of all integration tables** (verified by reading `crates/presenter-migration/src/m20250927_000001_create_core_tables.rs`):

- **`ResolumeHosts`** — has `host` + `port` + `is_enabled`. Pushes outbound HTTP. **Bleeds via DB.**
- **`AbleSetSettings`** — has `host` + `port` + `enabled`. Pushes outbound HTTP/WS. **Bleeds via DB.**
- **`OscSettings`** — has `enabled` + `listen_port` + `address_pattern` + `velocity_mode`. NO `host` column. Outbound OSC is configured via env var `PRESENTER_OSC_HOST_PORT` on each environment's systemd unit, so prod's REAPER target is set per-deployment, not in the DB. **Does NOT bleed.**
- **`AndroidStageDisplays`** — clients connect to `presenter-server`'s WS, not vice versa. **Pull-style; does not push outbound.**
- **`VideoSources`** — NDI sources, broadcaster (presenter is the source). **Pull-style; does not push outbound.**

So 2 integrations need active mocks (Resolume, AbleSet) and 2 tables get DELETE-cleared (`android_stage_displays`, `video_sources`) to remove stale prod-side identifiers from dev's DB. OSC needs no change.

The user explicitly wants **virtual integration targets** rather than disabling — so dev can test integration code paths against realistic mock endpoints without touching prod hardware.

## Approach

### Two distinct binaries via Cargo feature

Add a `mock-integrations` Cargo feature on the `presenter-server` crate. When enabled at compile time, `main.rs` spawns two additional in-process listeners alongside the main HTTP server. Each listener speaks the protocol of one external integration and accepts the calls real `presenter-server` makes.

- Dev pipeline (`pipeline.yml` build job): `cargo build --release --features mock-integrations`
- Prod pipeline (`deploy.yml` build job): `cargo build --release` (no feature; default)
- Prod release pipeline (`release.yml` for companion-pp): same, no feature

Two binaries, two different sets of compiled code. Prod can never accidentally enable mocks even if the dev DB is somehow restored on prod.

### Per-integration mocks

| Integration | Protocol | Mock port | Mock behavior |
|---|---|---|---|
| Resolume Arena | HTTP REST | `127.0.0.1:8091` | Accepts `GET /api/v1/composition` (returns minimal valid composition JSON with empty layers), `PUT /api/v1/composition/.../value` (returns 200), clip-trigger PUTs (returns 200). Logs each request to a 1024-entry in-memory ring buffer accessible via `GET /__mock/log` for operator inspection. |
| AbleSet | HTTP / WS | `127.0.0.1:39042` | Serves `GET /api/setlist` returning a static-but-valid setlist JSON shape. Accepts WebSocket upgrade and emits no events. Mirrors only the parts of AbleSet's API that presenter-server actually calls. |

The mocks **reuse the same protocol structs** (e.g. `presenter-server`'s existing `Composition` deserializer, `OscMessage` parser) — if the real protocol shape changes, the mocks fail to compile until updated. This keeps them in lockstep with the integration code.

### Pipeline DB rewrite

Add a new step in `pipeline.yml` immediately AFTER the "Replace dev database with production snapshot" step (line 810) and BEFORE service restart:

```yaml
- name: Redirect integrations to mock endpoints
  run: |
    ssh deploy-target << 'REMOTE_SCRIPT'
    set -e
    sqlite3 /opt/presenter-dev/presenter.db <<'SQL'
    UPDATE resolume_hosts SET host='127.0.0.1', port=8091, label=label || ' (mock)';
    UPDATE ableset_settings SET host='127.0.0.1', port=39042;
    DELETE FROM android_stage_displays;
    DELETE FROM video_sources;
    SQL
    REMOTE_SCRIPT
```

The two `DELETE` statements cover the pull-style integrations: `android_stage_displays` (prod tablet identifiers — irrelevant on dev's network) and `video_sources` (prod NDI source names — won't resolve on dev). Operator re-enters dev-specific values via the settings UI when local testing is needed.

The rewrite is idempotent — running it twice produces the same end state.

### CI guard against accidental drift

Add a job to the prod release pipeline (`deploy.yml` and `release.yml`) that greps the built artifact for the mock-listener strings:

```yaml
- name: Verify prod artifact has no mock integrations
  run: |
    if strings target/release/presenter-server | grep -E "mock_integrations::|MockResolume|MockAbleSet" >/dev/null; then
      echo "::error::Prod build contains mock-integrations symbols. Rebuild without --features mock-integrations."
      exit 1
    fi
```

Defense in depth — even if someone accidentally adds `--features mock-integrations` to a prod workflow, this catches it before deployment.

## File structure

### Created

```
crates/presenter-server/src/
  mock_integrations/
    mod.rs              # feature-gated module entry, exposes start_all() called from main.rs
    resolume.rs         # axum router mocking Resolume HTTP API
    ableset.rs          # axum router (HTTP + WS) mocking AbleSet
    request_log.rs      # 1024-entry ring buffer + GET /__mock/log handler
```

### Modified

- `crates/presenter-server/Cargo.toml` — add `mock-integrations = []` feature
- `crates/presenter-server/src/main.rs` — `#[cfg(feature = "mock-integrations")] mod mock_integrations;` and conditional `mock_integrations::start_all().await?;` after main listener binding
- `.github/workflows/pipeline.yml` — pass `--features mock-integrations` to the dev build job + new "Redirect integrations" step after the prod-snapshot copy
- `.github/workflows/deploy.yml` — add prod-no-mocks-symbols guard
- `.github/workflows/release.yml` — same guard
- `Cargo.toml` workspace + `crates/presenter-ui/Cargo.toml` — version bump 0.4.52 → 0.4.53 / 0.1.21 → 0.1.22

## Behavior after this change

| Scenario | Before | After |
|---|---|---|
| Dev deploy with prod-snapshot DB | Dev connects to PROD Resolume, sends commands during service | Dev connects to local mock on 127.0.0.1:8091, mock acks silently |
| Dev clip trigger | PROD Resolume changes clip on the live wall | Mock logs request, returns 200, no real-world effect |
| Operator views dev's Resolume status page | Shows "connected" to prod's IP | Shows "connected" to "(mock)" with localhost host |
| Operator wants to test against real Resolume on dev | Already happening (the bug) | Operator manually edits the host back to a real Resolume IP via the settings UI; dev settings UI is the explicit gate |
| Prod deploy | Prod connects to real Resolume | Same |
| Prod artifact accidentally built with feature | (no guard) | CI fails on `verify-no-mocks` step |

## Testing

### Unit / integration tests (Rust)

Per mock module under `#[cfg(all(test, feature = "mock-integrations"))]`:

1. `mock_integrations::resolume::tests::accepts_composition_get` — bind mock on a random port, GET `/api/v1/composition`, assert 200 + valid Composition JSON.
2. `mock_integrations::resolume::tests::accepts_clip_trigger` — PUT `/api/v1/composition/layers/1/clips/1/connect`, assert 200, assert request appears in log.
3. `mock_integrations::ableset::tests::serves_setlist_shape` — GET `/api/setlist`, assert response deserializes via presenter-server's existing `AbleSetSetlist` struct.

Existing wiremock-based resolume tests remain unchanged — they test against ad-hoc wiremock instances, not the embedded mock.

### Pipeline E2E

Add to `tests/e2e/`:

- `mock-resolume-roundtrip.spec.ts` — opens `/ui/operator`, performs an action that triggers a Resolume call (e.g. clip mapping change), confirms `/__mock/log` shows the PUT. Runs in dev pipeline only (skipped if `__mock/log` returns 404, which it will on prod).

### Manual verification on dev

1. SSH to dev (10.77.8.134), `curl http://localhost:8091/api/v1/composition` — must return mock JSON.
2. `curl http://localhost:8091/__mock/log` — must return `[]` (or recent entries).
3. From dev's operator UI, change a Resolume clip text. Hit `/__mock/log` again — must show the PUT.
4. Watch prod's Resolume — must NOT change (the actual fix verification).
5. `sqlite3 /opt/presenter-dev/presenter.db "SELECT host, port FROM resolume_hosts;"` — all rows show `127.0.0.1:8091`.

## Production data treatment

**Untouched.** Production's database, binary, and runtime are not modified. The CI guard ensures prod can never receive a mocked binary.

## Closes

- Issue #279 — `dev presenter connects to live resolume arena and interferes on production!!!`

## Risks / unknowns

- **AbleSet protocol coverage.** AbleSet's API has more endpoints than presenter-server uses. The mock implements only what presenter-server calls — verified by reading `crates/presenter-server/src/ableset/`. If presenter-server later adds a new AbleSet call, the mock needs to grow with it. Acceptable cost — same coupling exists for Resolume.
- **Mock startup ordering.** The three mocks must bind their ports BEFORE the main `presenter-server` startup tries to connect to them. Solution: `start_all()` is awaited before the integration registries' first sync. Verify via integration test.
- **Operator confusion.** "(mock)" suffix on Resolume host labels and the settings UI showing `127.0.0.1` is the only signal that integrations are virtualized. If operators expect dev to behave like prod, the differing latency and missing "actually changed something on the wall" feedback may surprise. Mitigation: dev's `/healthz` already shows `channel: dev` — combined with the label suffix, the signals are clear enough.
- **Future integration additions.** Adding a 6th integration to presenter-server (e.g. another video router) requires adding a new mock + DB rewrite line. Not automated; documented in CLAUDE.md after this PR ships.

## Out of scope

- **Rich mock UIs / inspectors.** A web page showing recent mock requests with diffs against expected — useful but separate.
- **Persistent mock state.** Mocks are stateless in-memory; restart loses the request log. Sufficient for the bug at hand.
- **Mocking PULL-style integrations** (Android stage WebSocket, NDI broadcaster). Not bleeders; pipeline DELETEs their rows.
- **Per-mock enable/disable toggles** at runtime. The feature flag is compile-time. If an operator wants real Resolume on dev temporarily, they edit the DB host directly — same workflow as today.
- **Test-environment isolation for the existing wiremock-based unit tests.** Those work fine; no change.
