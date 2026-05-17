# Settings Audit + Startup Read-Only Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make prod presenter settings (ableset, osc, resolume hosts, android stage displays, video sources) untouchable by automated paths. Every settings write goes through one audited chokepoint that records source + actor + before/after.

**Architecture:** New append-only `settings_audit` table. New `SettingsAuditSource` enum (`HttpSetter | CompanionSetter | StartupDefault | SchemaMigration`). All existing settings-write repository methods refactored through a single `upsert_setting_with_audit` chokepoint. Read-side rewrite in `get_ableset_settings` removed and moved to a proper sea-orm migration.

**Tech stack:** Rust workspace + sea-orm migrations + axum router + Leptos WASM client. SQLite backend. Existing patterns followed (see `m20260414_000002_seed_android_stage_displays.rs` for migration shape).

**Spec:** `docs/superpowers/specs/2026-05-17-settings-audit-design.md` (commit `c9d75ec`).

**Closes:** #309

---

## File Structure

**Modify:**
- `Cargo.toml` — version bump 0.4.81 → 0.4.82
- `Cargo.lock` + `crates/presenter-ui/Cargo.lock` — propagate
- `crates/presenter-persistence/src/entities.rs` — new `settings_audit` entity
- `crates/presenter-persistence/src/lib.rs` — re-export audit types
- `crates/presenter-persistence/src/repository/mod.rs` — chokepoint + refactor existing setters
- `crates/presenter-migration/src/lib.rs` — register two new migrations
- `crates/presenter-server/src/router/integrations/osc.rs` — extract actor + pass source
- `crates/presenter-server/src/router/integrations/ableset.rs` — same
- `crates/presenter-server/src/router/integrations/resolume.rs` — same
- `crates/presenter-server/src/router/integrations/android_stage.rs` — same
- `crates/presenter-server/src/router/integrations/video_source.rs` — same
- `crates/presenter-server/src/router/integrations.rs` (or `mod.rs`) — add `audit` submodule
- `crates/presenter-server/src/router.rs` — wire `/integrations/audit` route
- `crates/presenter-server/src/companion/*.rs` — pass CompanionSetter source on setter commands
- `crates/presenter-server/src/state/mod.rs` — pass StartupDefault source to auto-create paths
- `CLAUDE.md` — add audit log mention to `## Database Policy`

**Create:**
- `crates/presenter-migration/src/m20260517_000001_create_settings_audit.rs`
- `crates/presenter-migration/src/m20260517_000002_fix_legacy_ableset_defaults.rs`
- `crates/presenter-server/src/router/integrations/audit.rs`
- `tests/e2e/settings-audit.spec.ts`

---

## Tasks

### Task 1: Bump workspace version

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/presenter-ui/Cargo.lock`

- [ ] **Step 1: Edit `Cargo.toml`**

Change line `version = "0.4.81"` → `version = "0.4.82"` in the `[workspace.package]` section.

- [ ] **Step 2: Refresh lock files**

Run:
```bash
cargo update --workspace -p presenter-core -p presenter-server -p presenter-persistence -p presenter-migration -p presenter-importer -p presenter-bible -p presenter-ndi
cd crates/presenter-ui && cargo update --workspace -p presenter-ui && cd -
```

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.lock
git commit -m "chore: bump workspace version to 0.4.82 for #309"
```

---

### Task 2: Add `SettingsAuditSource` enum + audit row types

**Files:**
- Modify: `crates/presenter-persistence/src/lib.rs`
- Create: `crates/presenter-persistence/src/audit.rs`

- [ ] **Step 1: Create `crates/presenter-persistence/src/audit.rs`**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsAuditSource {
    HttpSetter,
    CompanionSetter,
    StartupDefault,
    SchemaMigration,
}

impl SettingsAuditSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HttpSetter => "http_setter",
            Self::CompanionSetter => "companion_setter",
            Self::StartupDefault => "startup_default",
            Self::SchemaMigration => "schema_migration",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsAuditEntry {
    pub id: String,
    pub setting_table: String,
    pub setting_id: String,
    pub source: SettingsAuditSource,
    pub actor: String,
    pub before_json: Option<serde_json::Value>,
    pub after_json: serde_json::Value,
    pub changed_at: chrono::DateTime<chrono::Utc>,
}
```

- [ ] **Step 2: Register module in `lib.rs`**

Add to `crates/presenter-persistence/src/lib.rs`:
```rust
pub mod audit;
pub use audit::{SettingsAuditEntry, SettingsAuditSource};
```

- [ ] **Step 3: Compile check**

Run: `cargo check -p presenter-persistence`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-persistence/src/audit.rs crates/presenter-persistence/src/lib.rs
git commit -m "feat(persistence): SettingsAuditSource + SettingsAuditEntry types (#309)"
```

---

### Task 3: Add `settings_audit` migration + entity

**Files:**
- Create: `crates/presenter-migration/src/m20260517_000001_create_settings_audit.rs`
- Modify: `crates/presenter-migration/src/lib.rs`
- Modify: `crates/presenter-persistence/src/entities.rs`

- [ ] **Step 1: Create migration file**

Path: `crates/presenter-migration/src/m20260517_000001_create_settings_audit.rs`

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(Iden)]
pub enum SettingsAudit {
    Table,
    Id,
    SettingTable,
    SettingId,
    Source,
    Actor,
    BeforeJson,
    AfterJson,
    ChangedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SettingsAudit::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(SettingsAudit::Id).text().not_null().primary_key())
                    .col(ColumnDef::new(SettingsAudit::SettingTable).text().not_null())
                    .col(ColumnDef::new(SettingsAudit::SettingId).text().not_null())
                    .col(ColumnDef::new(SettingsAudit::Source).text().not_null())
                    .col(ColumnDef::new(SettingsAudit::Actor).text().not_null().default("unknown"))
                    .col(ColumnDef::new(SettingsAudit::BeforeJson).text().null())
                    .col(ColumnDef::new(SettingsAudit::AfterJson).text().not_null())
                    .col(ColumnDef::new(SettingsAudit::ChangedAt).timestamp_with_time_zone().not_null())
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_settings_audit_table_id_time")
                    .table(SettingsAudit::Table)
                    .col(SettingsAudit::SettingTable)
                    .col(SettingsAudit::SettingId)
                    .col(SettingsAudit::ChangedAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_settings_audit_source_time")
                    .table(SettingsAudit::Table)
                    .col(SettingsAudit::Source)
                    .col(SettingsAudit::ChangedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SettingsAudit::Table).if_exists().to_owned())
            .await
    }
}
```

- [ ] **Step 2: Register migration in `crates/presenter-migration/src/lib.rs`**

Add `mod m20260517_000001_create_settings_audit;` and append `Box::new(m20260517_000001_create_settings_audit::Migration)` to the `migrations()` Vec.

- [ ] **Step 3: Add entity in `crates/presenter-persistence/src/entities.rs`**

Append:

```rust
pub mod settings_audit {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "settings_audit")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub setting_table: String,
        pub setting_id: String,
        pub source: String,
        pub actor: String,
        pub before_json: Option<String>,
        pub after_json: String,
        pub changed_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
```

- [ ] **Step 4: Compile check**

Run: `cargo check -p presenter-migration -p presenter-persistence`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-migration/src/m20260517_000001_create_settings_audit.rs crates/presenter-migration/src/lib.rs crates/presenter-persistence/src/entities.rs
git commit -m "feat(persistence): settings_audit table migration + entity (#309)"
```

---

### Task 4: Add repository chokepoint method

**Files:**
- Modify: `crates/presenter-persistence/src/repository/mod.rs`

- [ ] **Step 1: Add chokepoint method**

In `impl Repository`, add:

```rust
#[instrument(skip(self, before, after))]
pub async fn record_settings_audit(
    &self,
    setting_table: &'static str,
    setting_id: &str,
    source: SettingsAuditSource,
    actor: &str,
    before: Option<serde_json::Value>,
    after: serde_json::Value,
) -> anyhow::Result<()> {
    use crate::entities::settings_audit;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let active = settings_audit::ActiveModel {
        id: sea_orm::ActiveValue::set(id),
        setting_table: sea_orm::ActiveValue::set(setting_table.to_string()),
        setting_id: sea_orm::ActiveValue::set(setting_id.to_string()),
        source: sea_orm::ActiveValue::set(source.as_str().to_string()),
        actor: sea_orm::ActiveValue::set(actor.to_string()),
        before_json: sea_orm::ActiveValue::set(before.map(|v| v.to_string())),
        after_json: sea_orm::ActiveValue::set(after.to_string()),
        changed_at: sea_orm::ActiveValue::set(now.into()),
    };
    settings_audit::Entity::insert(active).exec(&self.db).await?;
    Ok(())
}

#[instrument(skip(self))]
pub async fn list_settings_audit(
    &self,
    setting_table: Option<&str>,
    setting_id: Option<&str>,
    since: Option<chrono::DateTime<chrono::Utc>>,
    limit: u64,
) -> anyhow::Result<Vec<crate::audit::SettingsAuditEntry>> {
    use crate::entities::settings_audit;
    use sea_orm::{ColumnTrait, QueryFilter, QueryOrder, QuerySelect};
    let mut q = settings_audit::Entity::find()
        .order_by_desc(settings_audit::Column::ChangedAt)
        .limit(limit);
    if let Some(t) = setting_table {
        q = q.filter(settings_audit::Column::SettingTable.eq(t));
    }
    if let Some(id) = setting_id {
        q = q.filter(settings_audit::Column::SettingId.eq(id));
    }
    if let Some(t) = since {
        let stamp: chrono::DateTime<chrono::FixedOffset> = t.into();
        q = q.filter(settings_audit::Column::ChangedAt.gte(stamp));
    }
    let rows = q.all(&self.db).await?;
    rows.into_iter()
        .map(|m| {
            let source = match m.source.as_str() {
                "http_setter" => SettingsAuditSource::HttpSetter,
                "companion_setter" => SettingsAuditSource::CompanionSetter,
                "startup_default" => SettingsAuditSource::StartupDefault,
                "schema_migration" => SettingsAuditSource::SchemaMigration,
                other => anyhow::bail!("unknown source: {other}"),
            };
            Ok(crate::audit::SettingsAuditEntry {
                id: m.id,
                setting_table: m.setting_table,
                setting_id: m.setting_id,
                source,
                actor: m.actor,
                before_json: m
                    .before_json
                    .map(|s| serde_json::from_str(&s))
                    .transpose()?,
                after_json: serde_json::from_str(&m.after_json)?,
                changed_at: m.changed_at.into(),
            })
        })
        .collect()
}
```

Add `use crate::audit::SettingsAuditSource;` near top.

- [ ] **Step 2: Compile check**

Run: `cargo check -p presenter-persistence`
Expected: PASS

- [ ] **Step 3: Add unit test**

In the `#[cfg(test)] mod tests` of `repository/mod.rs` (or create one if missing):

```rust
#[tokio::test]
async fn record_and_list_settings_audit_roundtrip() {
    let repo = Repository::connect_in_memory().await.unwrap();
    repo.record_settings_audit(
        "ableset_settings",
        "singleton",
        crate::audit::SettingsAuditSource::HttpSetter,
        "10.0.0.5",
        Some(serde_json::json!({"enabled": false})),
        serde_json::json!({"enabled": true}),
    )
    .await
    .unwrap();

    let rows = repo
        .list_settings_audit(Some("ableset_settings"), None, None, 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor, "10.0.0.5");
    assert_eq!(rows[0].source, crate::audit::SettingsAuditSource::HttpSetter);
    assert_eq!(rows[0].after_json["enabled"], true);
}
```

- [ ] **Step 4: Run test**

Run: `cargo test -p presenter-persistence record_and_list_settings_audit_roundtrip`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-persistence/src/repository/mod.rs
git commit -m "feat(persistence): record_settings_audit + list_settings_audit chokepoint (#309)"
```

---

### Task 5: Wire audit into OSC settings setter

**Files:**
- Modify: `crates/presenter-persistence/src/repository/mod.rs`

- [ ] **Step 1: Refactor `insert_osc_settings`**

Change signature to:
```rust
async fn insert_osc_settings(
    &self,
    draft: OscSettingsDraft,
    source: SettingsAuditSource,
    actor: &str,
) -> anyhow::Result<OscSettings>
```

Before the insert, read the current row (if any) and serialise it to JSON. After the insert, serialise the new state and call `record_settings_audit`. Use `setting_table = "osc_settings"`, `setting_id = OSC_SETTINGS_SINGLETON_ID`.

- [ ] **Step 2: Update callers**

`get_osc_settings` calls `insert_osc_settings(OscSettingsDraft::default(), SettingsAuditSource::StartupDefault, "system")`.

`upsert_osc_settings` signature also takes `source` + `actor` and forwards.

- [ ] **Step 3: Update state callers**

Find every caller of `upsert_osc_settings` outside the repo (likely `crates/presenter-server/src/state/mod.rs` in OSC settings methods). Thread `source` + `actor` through.

- [ ] **Step 4: Compile check**

Run: `cargo check -p presenter-persistence -p presenter-server`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-persistence/src/repository/mod.rs crates/presenter-server/src/state/mod.rs
git commit -m "feat(persistence): audit OSC settings writes (#309)"
```

---

### Task 6: Wire audit into AbleSet settings setter

**Files:**
- Modify: `crates/presenter-persistence/src/repository/mod.rs`

- [ ] **Step 1: Refactor `insert_ableset_settings`**

Same pattern as Task 5. Add `source` + `actor` parameters. Read before-state, write, record audit row. `setting_table = "ableset_settings"`, `setting_id = ABLESET_SETTINGS_SINGLETON_ID`.

- [ ] **Step 2: Update `get_ableset_settings`**

Pass `SettingsAuditSource::StartupDefault, "system"` to the auto-create path. DO NOT remove the read-side rewrite yet — Task 10 does that.

- [ ] **Step 3: Update callers**

`upsert_ableset_settings` takes `source` + `actor` and forwards. Update all callers in `crates/presenter-server/src/state/mod.rs`.

- [ ] **Step 4: Compile check**

Run: `cargo check -p presenter-persistence -p presenter-server`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-persistence/src/repository/mod.rs crates/presenter-server/src/state/mod.rs
git commit -m "feat(persistence): audit AbleSet settings writes (#309)"
```

---

### Task 7: Wire audit into Resolume hosts setters

**Files:**
- Modify: `crates/presenter-persistence/src/repository/mod.rs`

- [ ] **Step 1: Refactor every setter on `resolume_host`**

Methods include `upsert_resolume_host`, `delete_resolume_host`, `set_resolume_host_enabled`, `update_resolume_host`. Find them via `grep -n "resolume_host" crates/presenter-persistence/src/repository/mod.rs`.

Each gets `source: SettingsAuditSource, actor: &str` parameters. Before write: serialise current row to JSON (None if insert). After write: serialise new state and record audit row. `setting_table = "resolume_host"`, `setting_id = <row uuid string>`.

For deletes: record audit with `after_json = null-equivalent or {"deleted": true, ...row}`. Easiest: pass the deleted row's pre-state in `before_json`, and `after_json = serde_json::json!({"deleted": true, "id": id})`.

- [ ] **Step 2: Update state + router callers**

Thread `source` + `actor` through all callers.

- [ ] **Step 3: Compile check**

Run: `cargo check -p presenter-persistence -p presenter-server`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-persistence/src/repository/mod.rs crates/presenter-server/
git commit -m "feat(persistence): audit Resolume hosts writes (#309)"
```

---

### Task 8: Wire audit into Android stage display setters

**Files:**
- Modify: `crates/presenter-persistence/src/repository/mod.rs`

- [ ] **Step 1: Refactor every setter on `android_stage_display`**

Same pattern as Task 7. Methods include any `upsert_android_stage_display`, `set_android_stage_display_enabled`, `delete_android_stage_display`. `setting_table = "android_stage_display"`.

- [ ] **Step 2: Update state + router callers**

- [ ] **Step 3: Compile check**

Run: `cargo check -p presenter-persistence -p presenter-server`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-persistence/src/repository/mod.rs crates/presenter-server/
git commit -m "feat(persistence): audit Android stage display writes (#309)"
```

---

### Task 9: Wire audit into Video source setters

**Files:**
- Modify: `crates/presenter-persistence/src/repository/mod.rs`

- [ ] **Step 1: Refactor every setter on `video_source`**

Same pattern. Methods likely include `upsert_video_source`, `set_video_source_active`, `delete_video_source`. `setting_table = "video_source"`.

- [ ] **Step 2: Update callers**

- [ ] **Step 3: Compile check**

Run: `cargo check -p presenter-persistence -p presenter-server`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-persistence/src/repository/mod.rs crates/presenter-server/
git commit -m "feat(persistence): audit Video source writes (#309)"
```

---

### Task 10: Remove read-side rewrite + add one-shot legacy migration

**Files:**
- Modify: `crates/presenter-persistence/src/repository/mod.rs`
- Create: `crates/presenter-migration/src/m20260517_000002_fix_legacy_ableset_defaults.rs`
- Modify: `crates/presenter-migration/src/lib.rs`

- [ ] **Step 1: Remove read-side rewrite in `get_ableset_settings`**

Replace the block at `repository/mod.rs:223-256` so the function becomes pure read + auto-create-if-missing:

```rust
#[instrument(skip_all)]
pub async fn get_ableset_settings(&self) -> anyhow::Result<AbleSetSettings> {
    self.ensure_ableset_settings_table().await?;
    if let Some(model) =
        ableset_settings::Entity::find_by_id(ABLESET_SETTINGS_SINGLETON_ID.to_string())
            .one(&self.db)
            .await?
    {
        return Ok(ableset_model_to_domain(model)?);
    }
    self.insert_ableset_settings(
        AbleSetSettingsDraft::default(),
        SettingsAuditSource::StartupDefault,
        "system",
    )
    .await
}
```

- [ ] **Step 2: Create migration `m20260517_000002_fix_legacy_ableset_defaults.rs`**

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let backend = conn.get_database_backend();

        // Replace any legacy port=5950 and library="NEWLEVEL" with the new defaults.
        // Idempotent: only touches rows that still match the legacy values.
        let new_http_port: i32 = 80;
        let new_osc_port: i32 = 39051;
        let new_library = "NEW LEVEL";

        conn.execute(sea_orm::Statement::from_sql_and_values(
            backend,
            "UPDATE ableset_settings SET http_port = ?1 WHERE http_port = 5950",
            [new_http_port.into()],
        ))
        .await?;
        conn.execute(sea_orm::Statement::from_sql_and_values(
            backend,
            "UPDATE ableset_settings SET osc_port = ?1 WHERE osc_port = 5950",
            [new_osc_port.into()],
        ))
        .await?;
        conn.execute(sea_orm::Statement::from_sql_and_values(
            backend,
            "UPDATE ableset_settings SET library_name = ?1 WHERE UPPER(library_name) = 'NEWLEVEL'",
            [new_library.into()],
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // No down — legacy values cannot be safely restored.
        Ok(())
    }
}
```

- [ ] **Step 3: Register migration in `crates/presenter-migration/src/lib.rs`**

Append `Box::new(m20260517_000002_fix_legacy_ableset_defaults::Migration)` AFTER the audit table migration so the audit table exists first.

- [ ] **Step 4: Write regression test for removal**

In `crates/presenter-persistence/src/repository/mod.rs` test module:

```rust
#[tokio::test]
async fn get_ableset_settings_does_not_rewrite_legacy_values() {
    use crate::audit::SettingsAuditSource;
    let repo = Repository::connect_in_memory().await.unwrap();

    // Seed a row with legacy values
    repo.insert_ableset_settings(
        AbleSetSettingsDraft {
            enabled: true,
            host: "fohabl.lan".into(),
            osc_port: 5950,
            http_port: 5950,
            library_name: "NEWLEVEL".into(),
            song_prefix_length: 3,
        },
        SettingsAuditSource::HttpSetter,
        "test",
    )
    .await
    .unwrap();

    // Capture audit count
    let audit_before = repo
        .list_settings_audit(Some("ableset_settings"), None, None, 100)
        .await
        .unwrap()
        .len();

    // Read should NOT mutate
    let settings = repo.get_ableset_settings().await.unwrap();
    assert_eq!(settings.http_port, 5950);
    assert_eq!(settings.osc_port, 5950);
    assert_eq!(settings.library_name, "NEWLEVEL");

    let audit_after = repo
        .list_settings_audit(Some("ableset_settings"), None, None, 100)
        .await
        .unwrap()
        .len();
    assert_eq!(audit_before, audit_after, "get_ableset_settings must not write");
}
```

- [ ] **Step 5: Run test**

Run: `cargo test -p presenter-persistence get_ableset_settings_does_not_rewrite_legacy_values`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-persistence/src/repository/mod.rs crates/presenter-migration/src/m20260517_000002_fix_legacy_ableset_defaults.rs crates/presenter-migration/src/lib.rs
git commit -m "fix(persistence): remove read-side mutation in get_ableset_settings, migrate legacy values once (#309)"
```

---

### Task 11: Wire HTTP setters to extract actor + pass HttpSetter source

**Files:**
- Modify: `crates/presenter-server/src/router/integrations/osc.rs`
- Modify: `crates/presenter-server/src/router/integrations/ableset.rs`
- Modify: `crates/presenter-server/src/router/integrations/resolume.rs`
- Modify: `crates/presenter-server/src/router/integrations/android_stage.rs`
- Modify: `crates/presenter-server/src/router/integrations/video_source.rs`
- Modify: `crates/presenter-server/src/router.rs`

- [ ] **Step 1: Add actor extractor helper**

Create `crates/presenter-server/src/router/integrations/actor.rs` (or inline in `mod.rs`):

```rust
use axum::extract::ConnectInfo;
use axum::http::HeaderMap;
use std::net::SocketAddr;

pub fn extract_actor(headers: &HeaderMap, peer: Option<&SocketAddr>) -> String {
    if let Some(v) = headers.get("x-forwarded-for").and_then(|h| h.to_str().ok()) {
        let first = v.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }
    peer.map(|s| s.ip().to_string()).unwrap_or_else(|| "anonymous".to_string())
}
```

- [ ] **Step 2: Update each setter handler signature**

Every setter handler adds:
```rust
headers: HeaderMap,
ConnectInfo(peer): ConnectInfo<SocketAddr>,
```

And passes `SettingsAuditSource::HttpSetter` + `&extract_actor(&headers, Some(&peer))` to the state/repo method.

- [ ] **Step 3: Wire `into_make_service_with_connect_info` if not already**

Check `crates/presenter-server/src/main.rs` or wherever the server starts. `axum` requires `into_make_service_with_connect_info::<SocketAddr>()` for `ConnectInfo<SocketAddr>` to be available. If currently using `into_make_service()`, switch.

- [ ] **Step 4: Compile check**

Run: `cargo check -p presenter-server`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-server/
git commit -m "feat(server): HTTP setters pass HttpSetter source + actor IP (#309)"
```

---

### Task 12: Wire Companion WS setters to pass CompanionSetter source

**Files:**
- Modify: `crates/presenter-server/src/companion/*.rs` (find via grep)

- [ ] **Step 1: Locate companion setter dispatch**

```bash
grep -rn "set_broadcast_live\|set_companion_settings\|upsert_ableset\|upsert_osc" crates/presenter-server/src/companion/ | head -20
```

- [ ] **Step 2: Pass `SettingsAuditSource::CompanionSetter, "companion"` to each settings write**

Any companion command that ends up writing settings: thread source + actor through.

- [ ] **Step 3: Compile check**

Run: `cargo check -p presenter-server`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-server/
git commit -m "feat(server): Companion setters pass CompanionSetter source (#309)"
```

---

### Task 13: Add `GET /integrations/audit` endpoint

**Files:**
- Create: `crates/presenter-server/src/router/integrations/audit.rs`
- Modify: `crates/presenter-server/src/router/integrations.rs` (or `mod.rs`)
- Modify: `crates/presenter-server/src/router.rs`

- [ ] **Step 1: Create `audit.rs` handler**

```rust
use axum::{extract::{Query, State}, Json};
use serde::Deserialize;
use crate::state::AppState;
use crate::router::AppError;
use presenter_persistence::SettingsAuditEntry;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditQuery {
    pub table: Option<String>,
    pub setting_id: Option<String>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<u64>,
}

pub(crate) async fn list_settings_audit(
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Vec<SettingsAuditEntry>>, AppError> {
    let limit = q.limit.unwrap_or(100).min(1000);
    let rows = state
        .repository
        .list_settings_audit(q.table.as_deref(), q.setting_id.as_deref(), q.since, limit)
        .await
        .map_err(AppError::internal_error_chain)?;
    Ok(Json(rows))
}
```

- [ ] **Step 2: Register submodule + route**

In `crates/presenter-server/src/router/integrations.rs` (or `mod.rs`): `pub(crate) mod audit;`

In `crates/presenter-server/src/router.rs`: `.route("/integrations/audit", get(integrations::audit::list_settings_audit))`

- [ ] **Step 3: Add unit test**

In `audit.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;

    #[tokio::test]
    async fn list_settings_audit_returns_empty_on_fresh_state() {
        let state = crate::state::AppState::in_memory().await.unwrap();
        let res = list_settings_audit(
            State(state),
            Query(AuditQuery { table: None, setting_id: None, since: None, limit: None }),
        )
        .await;
        let Ok(Json(rows)) = res else { panic!("expected Ok"); };
        // NOTE: fresh state may contain StartupDefault rows; assert no HttpSetter rows
        assert!(rows.iter().all(|r| r.source != presenter_persistence::SettingsAuditSource::HttpSetter));
    }
}
```

- [ ] **Step 4: Compile + test**

Run: `cargo test -p presenter-server list_settings_audit_returns_empty_on_fresh_state`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-server/src/router/integrations/audit.rs crates/presenter-server/src/router/integrations.rs crates/presenter-server/src/router.rs
git commit -m "feat(server): GET /integrations/audit endpoint (#309)"
```

---

### Task 14: Startup read-only regression test

**Files:**
- Modify: `crates/presenter-persistence/src/repository/mod.rs` test module (or a new file)

- [ ] **Step 1: Write regression test**

```rust
#[tokio::test]
async fn second_startup_writes_no_audit_rows() {
    use crate::audit::SettingsAuditSource;

    // Connect, trigger settings reads (force singleton creation)
    let repo = Repository::connect_in_memory().await.unwrap();
    let _ = repo.get_osc_settings().await.unwrap();
    let _ = repo.get_ableset_settings().await.unwrap();

    let first_count = repo
        .list_settings_audit(None, None, None, 10_000)
        .await
        .unwrap()
        .len();
    assert!(first_count >= 2, "expected at least 2 startup default rows, got {first_count}");

    // Second "startup" — same DB, same reads
    let _ = repo.get_osc_settings().await.unwrap();
    let _ = repo.get_ableset_settings().await.unwrap();

    let second_count = repo
        .list_settings_audit(None, None, None, 10_000)
        .await
        .unwrap()
        .len();
    assert_eq!(first_count, second_count, "second startup must not write any audit rows");

    // Sanity: every existing row's source is StartupDefault
    for row in repo
        .list_settings_audit(None, None, None, 10_000)
        .await
        .unwrap()
    {
        assert_eq!(row.source, SettingsAuditSource::StartupDefault);
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p presenter-persistence second_startup_writes_no_audit_rows`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-persistence/src/repository/mod.rs
git commit -m "test(persistence): regression for #309 — second startup writes nothing"
```

---

### Task 15: E2E test — toggle setting, verify audit row

**Files:**
- Create: `tests/e2e/settings-audit.spec.ts`

- [ ] **Step 1: Write E2E spec**

Use `support.ts` `startTestServer` pattern from other specs. Scenario:

```typescript
import { test, expect } from "@playwright/test";
import { deriveTestConfig, refreshDevData, startTestServer, stopServer, type ServerHandle } from "./support";

test.describe.configure({ timeout: 120_000 });

let serverHandle: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("toggling ableset settings via HTTP creates an http_setter audit row", async ({ request }) => {
  const beforeResp = await request.get(new URL("/integrations/audit?table=ableset_settings&limit=100", baseURL).toString());
  expect(beforeResp.ok()).toBeTruthy();
  const beforeRows = (await beforeResp.json()) as Array<{ source: string }>;
  const beforeHttp = beforeRows.filter((r) => r.source === "http_setter").length;

  const current = await (await request.get(new URL("/integrations/ableset/settings", baseURL).toString())).json();
  const updated = { ...current, enabled: !current.enabled };
  const putResp = await request.put(
    new URL("/integrations/ableset/settings", baseURL).toString(),
    { data: updated },
  );
  expect(putResp.ok()).toBeTruthy();

  const afterResp = await request.get(new URL("/integrations/audit?table=ableset_settings&limit=100", baseURL).toString());
  const afterRows = (await afterResp.json()) as Array<{ source: string; afterJson: { enabled: boolean } }>;
  const afterHttp = afterRows.filter((r) => r.source === "http_setter").length;
  expect(afterHttp).toBe(beforeHttp + 1);
  expect(afterRows[0].afterJson.enabled).toBe(updated.enabled);

  // Restore
  await request.put(new URL("/integrations/ableset/settings", baseURL).toString(), { data: current });
});
```

- [ ] **Step 2: Run E2E locally**

Run: `npm run test:playwright -- settings-audit`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/settings-audit.spec.ts
git commit -m "test(e2e): settings audit roundtrip via HTTP setter (#309)"
```

---

### Task 16: Update CLAUDE.md `## Database Policy` section

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add audit log mention**

Under `## Database Policy`, add a new subsection:

```markdown
### Settings Audit Log

All settings writes (ableset, osc, resolume hosts, android stage displays, video sources) are recorded in `settings_audit` (append-only). Each entry captures:

- `setting_table`, `setting_id` — which row changed
- `source` — `http_setter` | `companion_setter` | `startup_default` | `schema_migration`
- `actor` — caller IP or `"system"` / `"companion"`
- `before_json`, `after_json` — full row state before and after

Query: `GET /integrations/audit?table=<name>&setting_id=<id>&since=<rfc3339>&limit=<n>`.

Startup MUST be read-only against settings tables. The only allowed startup write is creating a singleton row if missing (with `source=startup_default`). A second startup on an unchanged DB produces zero new audit rows — enforced by the regression test in `crates/presenter-persistence/src/repository/mod.rs::second_startup_writes_no_audit_rows`.
```

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: settings audit log + startup read-only invariant (#309)"
```

---

### Task 17: Push + monitor CI + open PR (CONTROLLER-HANDLED)

This task is handled by the controlling agent — not a subagent task.

- [ ] **Step 1: Run full local lint + test gate**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test --workspace
```

All must pass.

- [ ] **Step 2: Push**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI**

Per `ci-monitoring.md` — single `sleep N && gh run view` background pattern. Watch Pipeline run until ALL jobs green (incl. Mutation, 3× Playwright shards, Deploy to Dev, Deploy Companion).

- [ ] **Step 4: Verify on dev**

```bash
curl -s http://10.77.8.134:8080/healthz
# version: 0.4.82

curl -s "http://10.77.8.134:8080/integrations/audit?limit=10"
# Should return JSON array (may have startup_default rows from migration)

# Toggle a setting via curl + verify audit row appears
curl -s -X PUT http://10.77.8.134:8080/integrations/ableset/settings \
  -H "Content-Type: application/json" \
  -d '{"enabled":false,"host":"fohabl.lan","oscPort":39051,"httpPort":80,"libraryName":"NEW LEVEL","songPrefixLength":3}'
curl -s "http://10.77.8.134:8080/integrations/audit?table=ableset_settings&limit=3"
# Should show one http_setter row with the toggled value
# Restore: PUT enabled=true again
```

Plus Playwright DOM read to confirm the operator UI settings page still works.

- [ ] **Step 5: Open PR**

```bash
gh pr create --base main --head dev --title "feat(persistence): settings audit log + startup read-only guarantee (#309)" --body "..."
```

Wait for explicit user "merge it" per `pr-merge-policy.md`.

---

## Test commands

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`
- `cargo test --workspace`
- `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd -`
- `npm run test:playwright -- settings-audit`

## Critical airuleset rules

- `core/version-bumping.md` — Task 1 FIRST
- `core/ci-monitoring.md` — single sleep + gh run view, no /loop, no custom monitor
- `core/autonomous-quality-discipline.md` — never propose bypass
- `core/autonomous-verification.md` — Playwright on dev (10.77.8.134:8080), never ask user to test
- `core/no-localhost-urls.md` — LAN IP only
- `ci/test-strictness.md` — no `#[ignore]`, no skipped tests
- `ci/browser-console-zero-errors.md` — Playwright asserts clean console
- `ci/e2e-real-user-testing.md` — E2E uses real browser
- `quality/database-migrations.md` — incremental migration, never edit initial
- `pr-merge-policy.md` — open PR, wait for "merge it"

## Estimated total

~600 LoC across 17 tasks. Solo PR per `autonomous-batch-issue-development.md` (schema change + cross-cutting refactor + audit boundary).
