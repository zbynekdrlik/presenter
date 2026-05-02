# Explicit-Only Imports: Stop Auto-Importing Libraries on Server Startup — Design

**Date:** 2026-05-02
**Status:** Proposed
**Scope:** Backend (presenter-server) — remove auto-import-on-startup logic
**Issue:** [#228](https://github.com/zbynekdrlik/presenter/issues/228) — `ensure_seed_library` race defeats deploy import detection

## Goal

Stop the `presenter-server` from auto-importing seed libraries on startup. After this change, the **only** way to (re)populate libraries is the manual GitHub Actions Import Data workflow. Operators' hand-curated playlists, presentation renames, and other custom edits stay safe across every deploy.

## Why

User reports that re-imports happen on every deploy, overwriting hand-curated playlists. Per CLAUDE.md the design intent is *"First-time deploys auto-import; subsequent deploys preserve all user data"*, but the auto-import-detection logic has a race (issue #228 — `ensure_seed_library race defeats deploy import detection`) that re-fires the seed import on subsequent deploys. The user explicitly wants: *"leave production database untouched, not import after every deploy, we imported it once and that's all, now all work is done in presenter and cannot be lost after every upgrade"*.

The simplest, least-surprising solution: remove the auto-import code entirely. The Import Data workflow already exists for explicit (re)populates — we don't need a second pathway.

## Approach

Delete the auto-import-on-startup logic from `presenter-server`. Server startup keeps doing what it must:

- Open the SQLite DB.
- Run schema migrations (additive only — column adds, table creates; never destructive).
- Serve traffic.

Server startup will NOT:

- Check whether libraries are empty.
- Run `ensure_seed_library` or any equivalent.
- Read `data/libraries/` or trigger the importer.

The Import Data workflow (`.github/workflows/import-data.yml`) is unchanged. Operators trigger it from the Actions UI when they want a fresh import — usually after Mac → server library sync.

## Components

1. **Find and delete the auto-import logic.** Likely candidates: `crates/presenter-server/src/main.rs`, `crates/presenter-server/src/state.rs`, or a `crates/presenter-server/src/init/*.rs`. Implementation tasks:
   - Grep for `ensure_seed_library` / `auto_import` / `seed_library` / `bootstrap_libraries` / similar.
   - Remove the function and its call site at startup.
   - Remove any `present_seeds_path` / `seed_library_path` / similar config fields if no longer referenced.
   - Remove imports of `presenter-importer` from `presenter-server` if the importer is now unused server-side. (The importer crate stays — it's used by the Import Data workflow's binary.)

2. **Update CLAUDE.md.** Find the line *"First-time deploys auto-import; subsequent deploys preserve all user data"* and replace with: *"Imports happen only via the explicit Import Data workflow. Deploys never touch the database."* Plus update the surrounding context paragraphs to remove any mention of auto-import.

3. **Update any related docs** — search for "auto-import", "first-time deploy", "ensure_seed_library" across `docs/` and `crates/*/README.md` to catch other references.

## Production data treatment

**Untouched.** The user's existing production data (libraries imported once, plus operator-curated playlists/renames) stays exactly as-is. The fix takes effect on the NEXT deploy after this PR merges — that deploy is guaranteed not to call any import code.

Pre-deploy backups (5 retained per CLAUDE.md `## Database Policy`) remain as insurance.

## Behavior after this change

| Scenario | Before | After |
|---|---|---|
| Fresh deploy on empty DB | Auto-imports seed libraries | DB stays empty until operator runs Import Data |
| Subsequent deploy on populated DB | Race in #228 may trigger re-import | DB stays as-is, ALWAYS |
| Operator edits playlist, then deploys | Re-import may overwrite the playlist | Playlist preserved |
| Operator wants to refresh from new `.pro` files | Run Import Data workflow | Same — Run Import Data workflow |
| Schema migration adds a column | Migration runs, data preserved | Same — migration runs, data preserved |
| Brand-new server install | Auto-import on first start | Operator runs Import Data once |

## Testing

### Unit / integration test (Rust)

Add an integration test that:

1. Boots `presenter-server` pointed at an empty in-memory or temp-file SQLite DB.
2. Waits for startup to complete (health endpoint or readiness signal).
3. Asserts the libraries table is still empty after startup (`SELECT COUNT(*) FROM libraries == 0`).

Place under `crates/presenter-server/tests/` or in `crates/presenter-server/src/state/tests.rs` if there's an existing pattern. The test must FAIL if someone re-introduces an auto-import path, even via a different name.

### Manual verification on dev

1. SSH to dev (this machine, no SSH needed).
2. Stop the dev service: `sudo systemctl stop presenter-dev`.
3. Back up the dev DB: `cp /opt/presenter-dev/presenter.db /tmp/presenter-dev.db.backup`.
4. Truncate libraries: `sqlite3 /opt/presenter-dev/presenter.db 'DELETE FROM libraries;'` (this is dev only — never run on prod).
5. Start service: `sudo systemctl start presenter-dev`.
6. Curl `http://10.77.8.134:8080/libraries` — must return `[]` (no auto-import happened).
7. Restore backup: `cp /tmp/presenter-dev.db.backup /opt/presenter-dev/presenter.db`, restart service, confirm libraries are back.

### CI

The new integration test runs in the standard `cargo test -p presenter-server` job. No new workflow changes.

## Closes

- Issue #228 — `ensure_seed_library` race defeats deploy import detection.

The race is gone because there's no auto-import to race with.

## Risks / unknowns

- **Where exactly is the auto-import code.** The implementer must grep before editing. If the auto-import calls a chain of helpers (e.g. `ensure_seed_library` → `import_libraries_from_disk` → ...), the helpers may be reusable by the Import Data workflow — leave those alone, only remove the call from server startup.
- **Tests that depend on auto-import.** Some existing tests may set up scenarios assuming startup populates libraries. Those tests need to either explicitly seed via the test helper, or be removed if they were just covering the auto-import path.
- **First-deploy ergonomics.** New server installations now have an extra step ("run Import Data after first deploy"). For this project (single user, single production server), that's fine. Worth noting in CLAUDE.md.

## Out of scope

- **Concern A — regression test for `.pro` importer behavior.** Filed as a separate PR after this one ships. PR #285 (just merged) added a unit-level regression guard; the larger property-based or golden-file test is a separate work item.
- **Removing the rsync of `data/libraries/` from deploy workflows.** That stays — it stages files on disk for the Import Data workflow to read.
- **Changing the Import Data workflow itself.** It stays manual / operator-triggered.
- **Production data migration / cleanup.** Prod stays as-is per user instruction.
