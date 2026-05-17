# Settings Audit + Startup Read-Only Guarantee — Design Spec

**Closes:** #309 — "multiple times happened that settings on production devices was changed, for example ableset control, and ableton midi follow"

**Goal:** Make prod settings (ableset, osc, resolume hosts, android stage displays, video sources) NEVER change due to automated paths (deploy, restart, migration, read-side rewrites). When changes DO happen, log who/what/when in a queryable audit table.

---

## Problem

On 2026-05-07, integrations on prod (`10.77.9.205`) were found disabled during a production event. User did not disable them. Cause unknown. The user's mandate: NO automated path should touch settings.

Investigation identified one concrete risk path:

- `crates/presenter-persistence/src/repository/mod.rs:223-256` — `get_ableset_settings()` performs a **read-side write** when:
  - `model.http_port == 5950`, OR
  - `model.osc_port == 5950`, OR
  - `model.library_name` case-insensitively equals `"NEWLEVEL"`

  Triggers a silent UPDATE on the DB row. Migration-via-getter pattern. Not the cause of the 2026-05-07 incident (prod values don't match triggers) but violates the user's intent.

Other paths that COULD write settings without a UI click:

- Singleton auto-create when row missing: `insert_osc_settings` / `insert_ableset_settings` with `enabled=false` defaults.
- Companion WebSocket setters (if Companion plugin is misconfigured or a stale command lands).
- Frontend Settings form posting stale state on save.

Currently NO mechanism logs which path mutated a settings row. We cannot tell post-hoc whether the disable came from the operator UI, a Companion command, startup, or a read-side rewrite.

---

## Architecture

Three components:

1. **`settings_audit` append-only log table** — captures every write to any settings table, with the source attribution.
2. **Single chokepoint repo method `upsert_setting_with_audit`** — all settings writes route through it.
3. **Removal of the read-side mutation in `get_ableset_settings`** — port/library-name migration moves to a proper sea-orm migration that runs once on schema upgrade. The getter becomes pure read.

### Audit table

Schema (sea-orm migration `m20260517_000001_create_settings_audit`):

```
CREATE TABLE settings_audit (
    id TEXT NOT NULL PRIMARY KEY,           -- UUIDv4
    setting_table TEXT NOT NULL,            -- "ableset_settings" | "osc_settings" | "resolume_host" | ...
    setting_id TEXT NOT NULL,               -- singleton id or row UUID
    source TEXT NOT NULL,                   -- enum (see below)
    actor TEXT NOT NULL DEFAULT 'unknown',  -- HTTP user (IP/header) or "system"
    before_json TEXT,                       -- nullable: NULL on first insert
    after_json TEXT NOT NULL,               -- JSON of the row AFTER the write
    changed_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_settings_audit_table_id_time ON settings_audit (setting_table, setting_id, changed_at DESC);
CREATE INDEX idx_settings_audit_source_time ON settings_audit (source, changed_at DESC);
```

Append-only. No update, no delete. Old entries beyond N days can be pruned by a future cron — out of scope for this spec.

### `source` enum (Rust + string serialised to DB)

```rust
pub enum SettingsAuditSource {
    HttpSetter,        // user clicked Save in the operator UI
    CompanionSetter,   // Companion WebSocket command (broadcast.set_live etc.)
    StartupDefault,    // singleton row was missing and got auto-created with defaults
    SchemaMigration,   // a proper sea-orm migration backfilled a column
}
```

`StartupDefault` is the ONLY source allowed during normal startup. Any other source firing on startup is a bug.

`SchemaMigration` runs only when sea-orm migration steps execute (i.e. once per schema version on a given DB).

The read-side migration in `get_ableset_settings` is REMOVED — it does not get a source because it no longer exists.

### Repository chokepoint

New method:

```rust
impl Repository {
    pub async fn upsert_setting_with_audit<T, F, Fut>(
        &self,
        setting_table: &'static str,
        setting_id: &str,
        source: SettingsAuditSource,
        actor: &str,
        write: F,
    ) -> anyhow::Result<T>
    where
        F: FnOnce(&DatabaseConnection) -> Fut,
        Fut: Future<Output = anyhow::Result<(T, serde_json::Value, Option<serde_json::Value>)>>,
        // Returns (domain_value, after_json, before_json_or_none)
    { ... }
}
```

Existing settings write methods (`insert_osc_settings`, `insert_ableset_settings`, `upsert_resolume_host`, `set_resolume_host_enabled`, `update_android_stage_display`, etc.) are refactored to call `upsert_setting_with_audit` internally. Existing callers don't need to change their signatures — `source` and `actor` are threaded through as new required parameters.

Tests assert: every settings-write path produces exactly one audit row. Specifically, starting an `AppState::in_memory` twice in a row produces audit rows with `source=StartupDefault` on the FIRST startup and ZERO rows on the SECOND startup (because singletons already exist).

### HTTP setter wiring

The integration router handlers (`PUT /integrations/osc/settings`, `PUT /integrations/ableset/settings`, `POST /integrations/resolume/hosts`, etc.) extract a best-effort actor string from the request:

- Prefer `X-Forwarded-For` header (for reverse-proxy IP)
- Fall back to peer socket address from axum `ConnectInfo`
- Default `"anonymous"` if neither available

Source is always `SettingsAuditSource::HttpSetter` from these handlers.

Companion WS setters use `SettingsAuditSource::CompanionSetter` and pass `"companion"` as the actor.

### Removal of read-side mutation

`get_ableset_settings` at `repository/mod.rs:223-256` becomes a pure read:

```rust
pub async fn get_ableset_settings(&self) -> anyhow::Result<AbleSetSettings> {
    self.ensure_ableset_settings_table().await?;
    if let Some(model) = ableset_settings::Entity::find_by_id(...).one(&self.db).await? {
        return Ok(ableset_model_to_domain(model)?);
    }
    self.insert_ableset_settings_with_audit(
        AbleSetSettingsDraft::default(),
        SettingsAuditSource::StartupDefault,
        "system",
    ).await
}
```

If the legacy port=5950 / library="NEWLEVEL" backfill is still needed for any prod or dev DB:

- Add a one-shot sea-orm migration `m20260517_000002_fix_legacy_ableset_defaults.rs` that runs the same UPDATE under transaction with audit row `source=SchemaMigration`.
- Add an idempotent guard inside the migration so it can be replayed safely.

### Read endpoint for audit log

`GET /integrations/audit?table=<name>&setting_id=<id>&since=<rfc3339>&limit=<n>` returns a paged JSON list of audit rows for forensics. Implemented under `crates/presenter-server/src/router/integrations/audit.rs`. No write endpoint — pure read.

Auth: same as other integration endpoints (which currently means no auth on this repo; if/when auth is added, this endpoint adopts the same convention).

---

## Test strategy

1. **Migration test** — `m20260517_000001_create_settings_audit` up + down creates and drops the table cleanly.
2. **Audit-row-per-write test** — each settings setter (osc, ableset, resolume add/update, android stage update, video source toggle) produces exactly one audit row with the expected source.
3. **Startup-read-only invariant test** (regression test for #309) — `AppState::in_memory()` followed by reading all settings produces exactly the `StartupDefault` audit rows for missing singletons. A SECOND identical `AppState` startup on the same DB produces ZERO new audit rows. Asserts: the second startup writes nothing.
4. **Removal-of-read-side-mutation test** — seed a row with `http_port=5950` (the old legacy value). Call `get_ableset_settings`. Assert the row is RETURNED unchanged AND no audit row was added. (Migration moves the rewrite into a one-shot.)
5. **HTTP setter actor test** — POST with `X-Forwarded-For: 10.0.0.5` produces an audit row with `actor: "10.0.0.5"`.
6. **End-to-end Playwright** — start dev server, GET /integrations/audit (empty or only-startup rows), toggle a setting via the operator UI, GET /integrations/audit again and assert a new row with `source=http_setter` and the matching new value.

---

## Out of scope

- Authentication / authorisation on settings endpoints. Untouched.
- Companion plugin changes. Companion WS setters get `source=CompanionSetter` but the plugin itself is not changed.
- Audit-log retention / pruning. Future cron job.
- UI surface for browsing audit history. Read endpoint exists; building a UI tab is a follow-up.

---

## Estimated scope

- New migration: ~50 LoC
- Audit row entity + repo helpers: ~120 LoC
- Refactor existing 4-5 setting setters through the chokepoint: ~100 LoC
- Read endpoint: ~60 LoC
- Tests (unit + integration + regression): ~250 LoC
- Documentation in CLAUDE.md `## Database Policy` mentioning audit log: ~10 lines

Total: ~600 LoC. Solo PR per autonomous-batch-issue-development.md "Solo-PR" gate (schema change + cross-cutting).

---

## Rollout

- One PR (`dev` → `main`)
- Migration runs auto on prod startup (incremental, idempotent)
- No data deletion, no destructive operations
- After deploy: verify on prod by reading `/integrations/audit` — should show one row per singleton from the startup migration creating the new table, plus zero recent rows. Subsequent operator UI clicks should produce new rows immediately.

## Success criteria

1. Prod settings cannot change without an audit row naming the source.
2. The `get_ableset_settings` read-side rewrite is gone.
3. Startup of the server on an unchanged DB produces zero new audit rows.
4. If integrations are found disabled again, `GET /integrations/audit?table=ableset_settings` immediately identifies the source and the timestamp.
