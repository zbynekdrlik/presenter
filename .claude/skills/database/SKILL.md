---
name: database
description: Presenter database policy — schema-change/migration rules, deploy safety and backups, settings audit log, manual import workflow, and ProPresenter library management. Use when adding a migration, changing schema, running the Import Data workflow, or touching settings persistence.
---

# Database Policy

## Schema Changes (Pre-release)

Schema is mutable during pre-release:

1. **New columns:** Add an incremental migration (e.g., `m20260408_000001_add_column.rs`) that uses `ALTER TABLE ADD COLUMN` with an idempotent guard (check `pragma_table_info` first). Register it in `lib.rs`. This ensures existing databases are upgraded automatically on startup.
2. **New tables:** May be added to the initial migration (uses `if_not_exists()`) or as an incremental migration.
3. **Destructive schema changes** (column renames, type changes): Add an incremental migration. If data must be re-imported, manually trigger the Import Data workflow after deploy.
4. The server auto-migrates on startup via `Repository::connect()`

## Deploy Safety

- Deploys NEVER delete the database — only binaries and service files are updated
- Database is backed up automatically before each deploy (5 retained in `backups/`)
- Imports happen only via the explicit Import Data workflow. Deploys never touch the database.
- New server installations start with an empty libraries table. Run the Import Data workflow once after first deploy to populate it.

## Settings Audit Log

All settings writes (ableset, osc, resolume hosts, android stage displays, video sources) are recorded in `settings_audit` (append-only). Each entry captures:

- `setting_table`, `setting_id` — which row changed
- `source` — `http_setter` | `companion_setter` | `startup_default` | `schema_migration`
- `actor` — caller IP (from `X-Forwarded-For` or `X-Real-IP`) or `"system"` / `"companion"`
- `before_json`, `after_json` — full row state before and after

Query: `GET /integrations/audit?table=<name>&settingId=<id>&since=<rfc3339>&limit=<n>`.

Startup MUST be read-only against settings tables. The only allowed startup write is creating a singleton row if missing (with `source=startup_default`). A second startup on an unchanged DB produces zero new audit rows — enforced by the regression test in `crates/presenter-persistence/src/repository/tests.rs::second_startup_writes_no_audit_rows`.

## Manual Import

To re-import source data (ProPresenter libraries, Bibles):

1. Go to Actions > "Import Data" > Run workflow
2. Select environment (dev/production) and import type
3. Default mode (`--keep`) preserves existing data
4. `--purge` mode replaces all libraries (WARNING: destroys playlists via FK cascade)

## Library Management

ProPresenter libraries are stored in `data/libraries/` as the single source of truth.

**To update songs:**

1. Export from ProPresenter on Mac
2. Copy `.pro` files to `data/libraries/<LIBRARY_NAME>/`
3. Commit and push to dev
4. Deploy syncs libraries to servers via `rsync`
5. Run Import Data workflow if needed

Deploy workflows automatically sync `data/libraries/` to `/opt/presenter*/libraries/` on target servers.
