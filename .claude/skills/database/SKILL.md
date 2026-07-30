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

## Inspecting a production database (read-only)

Many data questions ("did this bug leave bad rows in production?", "how many
favorites are dangling?") are answerable with a single read-only query. The
live service holds the DB open in WAL mode, so a **read-only** handle cannot
disturb it — inspect freely. Never re-derive the invocation from scratch; this
is the standing procedure.

The credential is **never** committed (skills are git-committed). Keep the
value in local memory and reference it via the env-var form:

```bash
# SNV production (presenter.lan) — sqlite3 IS installed at /usr/bin/sqlite3
sshpass -p "$PRESENTER_PROD_PW" ssh newlevel@presenter.lan \
  "sqlite3 -readonly -header /opt/presenter/presenter.db 'SELECT ...'"

# PP (companion-pp.lan) — same path; sqlite3 is installed there too
sshpass -p "$PRESENTER_PROD_PW" ssh newlevel@companion-pp.lan \
  "sqlite3 -readonly -header /opt/presenter/presenter.db 'SELECT ...'"
```

**Always `-readonly` for inspection.** It opens the handle in read-only mode,
guaranteeing no statement can ever write — a safety net independent of the
query text. The DB path is `/opt/presenter/presenter.db` on both hosts.

**A prod-DB write / `DELETE` / `DROP` is a gated destructive action** and needs
explicit user approval every time (per the global `no-destructive-remote-actions`
rule). Inspection never does — but the moment a query mutates state, stop and
ask first. Never run an ad-hoc `DELETE`/`UPDATE`/`DROP` against a production DB
without approval, no matter how harmless it looks.

The useful orientation query when investigating an integrity question — run it
first to get a snapshot of the relevant counts:

```sql
SELECT (SELECT COUNT(*) FROM library_favorites),
       (SELECT COUNT(*) FROM libraries),
       (SELECT COUNT(*) FROM libraries WHERE deleted_at IS NOT NULL);
```

**Fallback when `sqlite3` is unavailable on a host** (a stripped-down image, a
restricted shell): every Python install ships the `sqlite3` stdlib module, so
the same read-only query works through it:

```bash
sshpass -p "$PRESENTER_PROD_PW" ssh newlevel@presenter.lan \
  "python3 -c \"import sqlite3; c=sqlite3.connect('file:/opt/presenter/presenter.db?mode=ro', uri=True); print(list(c.execute('SELECT COUNT(*) FROM library_favorites')))\""
```

The `?mode=ro` URI enforces read-only the same way the CLI's `-readonly` does.

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

## Testing a SQL secondary-sort tie-break — insert the LOSER first (#594 lesson)

When testing that an `order_by_desc(SecondaryColumn)` tie-break actually matters (e.g.
`ensure_library`'s `order_by_desc(UpdatedAt).order_by_desc(SyncId)` in
`sync_apply.rs` — the `SyncId` tie-break decides which of several tombstoned rows sharing an
identical `UpdatedAt` wins), **insert the row that must LOSE the tie-break FIRST and the row that
must WIN it SECOND.** SQLite's `ORDER BY <primary> DESC` with NO secondary sort breaks a tie by
returning matching rows in their natural/insertion order — so if you insert the expected WINNER
first, the test still passes even with the real tie-break clause deleted from production code
(insertion order coincidentally produces the same answer). This is a genuinely vacuous oracle: it
looks like a real regression test but never fails when the thing it claims to test is removed.
Caught by an independent `superpowers:requesting-code-review` pass on #594 (the reviewer
empirically deleted the production tie-break line and reran the test to confirm). The fix costs
nothing — just insert in loser-then-winner order — but you must think about it deliberately; it is
not something `cargo test` or CI catches on its own, only a genuinely adversarial re-read (mutate
the prod line, rerun the test, see if it fails) does.

## Library Management

ProPresenter libraries are stored in `data/libraries/` as the single source of truth.

**To update songs:**

1. Export from ProPresenter on Mac
2. Copy `.pro` files to `data/libraries/<LIBRARY_NAME>/`
3. Commit and push to dev
4. Deploy syncs libraries to servers via `rsync`
5. Run Import Data workflow if needed

Deploy workflows automatically sync `data/libraries/` to `/opt/presenter*/libraries/` on target servers.
