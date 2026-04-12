# Companion Plugin Sync Design

> **Issue:** #214 — Companion plugin needs rework
> **Date:** 2026-04-12

## Problem

The Companion plugin (v0.5.0 deployed on both machines) is out of sync with the Presenter server. The server supports 14 commands but the plugin only exposes 11. New features (preach timer limits, NDI fullscreen layout, stage.set) are unreachable from Companion buttons. Additionally, BibleSlide events from the new Bible tab don't update Companion variables, and there is no automated deployment — module updates require manual SSH.

## Scope

1. Add 3 missing commands to the JS plugin: `stage.set`, `timer.set_preach_limit`, `timer.clear_preach_limit`
2. Add `ndi-fullscreen` to stage layout choices
3. Add `timer_preach_limit_seconds` variable definition
4. Make `BibleSlide` events update Companion `bible_*` variables (server-side Rust fix)
5. Fix version mismatch (manifest.json 0.5.0 vs package.json 0.6.0) — bump both to 0.7.0
6. Add CI/CD step to auto-deploy the plugin to both Companion hosts after pipeline success

## Architecture

### Plugin Changes (`ops/companion/presenter/index.js`)

**New commands** added to `COMMANDS` array:

| Command | Options | Payload |
|---------|---------|---------|
| `stage.set` | presentationId (text, UUID), currentSlideId (text, UUID), nextSlideId (text, UUID, optional) | `{presentationId, currentSlideId, nextSlideId?}` |
| `timer.set_preach_limit` | seconds (number, min 1) | `{seconds}` |
| `timer.clear_preach_limit` | none | `{}` |

**New layout choice:** `{ id: "ndi-fullscreen", label: "NDI FULLSCREEN" }` appended to `STAGE_LAYOUT_CHOICES`.

**New variable:** `timer_preach_limit_seconds` added to `VARIABLE_DEFINITIONS` (after `timer_preach_elapsed_readable`, before `bible_translation_code`).

**New feedback:** `preach_running` boolean feedback (mirrors `countdown_running` pattern) — active when `timer_preach_state === "running"`.

### Server Change (`companion/variables.rs`)

Replace the `BibleSlide` no-op handler with a method that maps `BibleSlideOutput` fields to the existing `bible_*` variables:

- `bible_reference` ← `output.main_reference`
- `bible_text` ← `output.main_text`
- `bible_triggered_at` ← `output.triggered_at.to_rfc3339()`
- `bible_translation_code` ← extracted from reference parentheses, e.g. "John 3:16 (KJV)" → "KJV", or empty string if not parseable
- `bible_translation_name` ← same as code (no separate name available in BibleSlideOutput)

This uses a new `apply_bible_slide` method that stores the data in the existing `bible: Option<BibleBroadcast>` field — but since `BibleSlideOutput` has different structure, we'll instead directly set a parallel `bible_slide_override` field or, simpler, just bypass the `BibleBroadcast` struct and set the variable values directly when serializing.

**Chosen approach:** Add a `bible_override: Option<BibleSlideOverride>` field to `CompanionVariableState` with just the 5 string values. When `BibleSlide` arrives, set this and clear `bible`. When legacy `Bible` arrives, set `bible` and clear `bible_override`. `write_bible_variables` checks `bible_override` first, falls back to `bible`.

```rust
struct BibleSlideOverride {
    translation_code: String,
    translation_name: String,
    reference: String,
    text: String,
    triggered_at: String,
}
```

### Version Bump

Both `package.json` and `manifest.json` updated to `0.7.0`. New tarball `presenter-companion-ws-0.7.0.tgz` generated and committed to `ops/companion/releases/`.

### CI/CD Deployment

A new job `deploy-companion` added to `pipeline.yml` (runs after `deploy-dev` succeeds, on the self-hosted runner) that:

1. Packages the module: `bash scripts/companion/package-module.sh`
2. Deploys to **companion-snv.lan** (10.77.9.205):
   - SCP module files to `/home/companion/.config/companion-nodejs/modules/presenter/`
   - `sudo systemctl restart companion`
   - Health check: `curl -sf http://127.0.0.1:8000`
3. Deploys to **companion-pp.lan**:
   - SCP module files to `/opt/companion/v4.1/modules/presenter-0.7.0/`
   - Run `npm install --omit=dev` inside the module dir
   - `docker restart companion`
   - Health check: `curl -sf http://127.0.0.1:8000`

**SSH keys:** SNV uses `DEPLOY_SSH_KEY` (same machine as production). PP uses `DEPLOY_SSH_KEY_PP`.

**Trigger:** Runs on every successful dev pipeline, ensuring both Companion hosts always have the latest plugin.

## Deployment Targets

| Host | Type | Module Path | Restart | SSH Key |
|------|------|-------------|---------|---------|
| companion-snv.lan (10.77.9.205) | Native CompanionPi | `/home/companion/.config/companion-nodejs/modules/presenter/` | `sudo systemctl restart companion` | `DEPLOY_SSH_KEY` |
| companion-pp.lan | Docker | `/opt/companion/v4.1/modules/presenter-0.7.0/` | `docker restart companion` | `DEPLOY_SSH_KEY_PP` |

## Testing

- **Existing E2E tests** (`companion-session.spec.ts`) already cover `stage.set`, all timer commands, bible trigger/clear — these validate the server side
- **New E2E tests:** `timer.set_preach_limit` + `timer.clear_preach_limit` commands, `ndi-fullscreen` layout switching
- **New unit test:** BibleSlide event updates Companion variables (in `companion/tests.rs`)
- **Companion JS tests:** Update `lib/commands.test.js` if needed for new command option definitions

## Out of Scope

- NDI source control from Companion (no server commands exist)
- Resolume control from Companion (separate integration)
- Presentation CRUD from Companion
- Playlist management from Companion
