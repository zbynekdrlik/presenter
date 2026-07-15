# Two-Way Song Sync Between Presenter Instances (PP ↔ SNV) — Design

**Issue:** #555
**Date:** 2026-07-15
**Status:** Accepted (delete behavior C confirmed by user 2026-07-15; rest is engineering)

## Problem

PP (companion-pp.lan) and SNV (10.77.9.205) run independent Presenter instances with
independent SQLite databases. Songs created or edited on one instance do not appear on the
other. The team maintains the same repertoire at both sites and currently has to redo every
change twice.

## Goal

1. A song (presentation) created on one instance appears on the other automatically.
2. A song edited on one instance (rename, slide text, group, slide add/remove/reorder)
   updates on the other automatically.
3. Initial reconciliation and every conflict: **the copy with the newest edit wins**
   (last-write-wins, LWW).
4. Deletes sync too, with a safety net: deleted songs go to a restorable trash
   (user decision C).

## Verified constraints (live discovery, 2026-07-15)

- **The two sites cannot reach each other over LAN** — both use overlapping
  `10.77.8.0/23` subnets with the same gateway IP; `PP → 10.77.9.205` is
  "No route to host" by construction. **Tailscale is the only inter-site path** and it
  works today, both directions: SNV `100.122.204.47` ↔ PP `100.101.72.101`
  (HTTP 200 on `/healthz`, both v0.4.198). `.lan` names are unreliable cross-site
  (`presenter.lan` resolves to PP ITSELF on the PP host) — config uses tailscale IPs.
- **No `updated_at` exists anywhere** (`presentations` and `slides` carry only
  `created_at`), and **no mutation bumps any timestamp today** — rename and slide-content
  edits touch none; structural slide ops (`replace_presentation_slides`) accidentally
  reset every slide's `created_at`. LWW has no clock without schema work.
- **No cross-instance identity**: all ids are fresh UUIDv4 per instance; the importer
  reads the `.pro` protobuf UUID (`raw.uuid`) but never persists it. Two instances
  importing the same `.pro` file hold the same song under different ids.
- **API has no auth** (open on the LAN today); instance-to-instance calls ride the
  tailnet, which is the trust boundary — same posture as everything else on these boxes.
- Library imports **wipe and recreate** the whole library (fresh ids, fresh
  `created_at`) — sync identity must survive a re-import.

## Design

### Schema (idempotent incremental migrations, per DB policy)

1. `presentations.updated_at: DateTimeWithTimeZone NOT NULL` — backfilled from
   `created_at`. Bumped by every mutation: `create_presentation`,
   `rename_presentation`, `update_slide_content*`, `replace_presentation_slides`
   (covers insert/duplicate/delete-slide/reorder), soft-delete, restore.
2. `presentations.sync_id: TEXT NOT NULL` + unique index — the cross-instance identity.
   - Importer: `sync_id = raw.uuid` from the `.pro` file → both sites importing the same
     file converge on the same identity, including after any re-import.
   - App-created songs: fresh UUIDv4.
   - Backfill for existing rows: `UUIDv5(namespace, library_name + "/" + name)` —
     deterministic, so the two sites' existing identical repertoires pair up without
     coordination.
3. `presentations.deleted_at: Option<DateTimeWithTimeZone>` — soft delete (the trash).
   `DELETE /presentations/{id}` now sets `deleted_at` + `updated_at` (and removes the
   song's playlist entries, preserving today's user-visible behavior). All existing list
   queries filter `deleted_at IS NULL`. A background task prunes rows deleted >30 days.
   Restore clears `deleted_at` (+bump).

### Identity edge — adopt-by-name

Transitional mismatches are possible (one site re-imported with `.pro`-UUID sync_ids, the
other still holds UUIDv5 backfills). Apply rule: when a peer song arrives whose `sync_id`
is unknown but a song with the SAME name exists in the same-named local library, treat
them as the same song — resolve by LWW and adopt the winner's `sync_id` — never duplicate.

### Sync engine (new `state/sync.rs`, ableset-tracker pattern)

- **Config:** `PRESENTER_SYNC_PEER_URL` env var in each deploy unit
  (SNV: `http://100.101.72.101`, PP: `http://100.122.204.47`). Unset → sync disabled
  (dev instance and E2E servers unaffected).
- **Symmetric pull loops** — each instance runs the same loop against its peer every 30 s
  (`tokio::time::interval`, `MissedTickBehavior::Skip`, oneshot shutdown — the AbleSet
  template), PLUS an immediate debounced (~2 s) trigger after every local song mutation,
  so edits propagate in seconds, not at the next tick. Pull-only symmetry means the
  initial reconciliation, runtime sync, and recovery after downtime are all the same code
  path — no push queue, no retry bookkeeping.
- **Protocol** (two new endpoints, serving both directions):
  - `GET /sync/manifest` → `[{ syncId, libraryName, name, updatedAt, deletedAt }]` for
    ALL songs including trashed ones.
  - `GET /sync/presentations/{sync_id}` → full content: name, library name, slides
    (main/translation/stage/group), updatedAt, deletedAt.
- **Apply (LWW):** for each manifest row where the peer's `updatedAt` is strictly newer
  than local (or the song is unknown): pull full content and upsert by `sync_id`
  (adopt-by-name first, see above). Local presentation id is PRESERVED on update
  (playlist references stay intact); slides are replaced wholesale carrying the peer's
  slide ids (UUIDv4 global uniqueness makes collisions a non-issue). **The applied row
  stores the PEER's `updated_at`, never `now()`** — an applied change is not a new edit,
  which is what prevents echo/ping-pong loops. Peer-newer `deletedAt` applies the soft
  delete; peer-newer restore clears it.
- **Clock caveat:** LWW trusts host clocks (both boxes run NTP). Human-timescale edits
  make sub-second skew irrelevant; documented, not engineered around.
- **Status:** `GET /integrations/sync/status` → peer URL, peer version/health, last run,
  last success, last error, counts pulled/applied last cycle (AbleSet status pattern).
  Every apply/skip/error is logged with sync_id + names (comprehensive-logging).

### Trash UI (minimal)

Settings page section "Zmazané piesne": list of soft-deleted songs (name, library,
deleted date) + "Obnoviť" button per row. Nothing else.

### Out of scope

- Bible presentations, playlists, settings — songs only (user request).
- More than two peers (config is a single peer URL; the protocol wouldn't change).
- Cross-site clipboard/#554 interplay — none; #554 operates within one instance.

## Testing

- **Rust unit:** LWW decision function (newer/older/equal/unknown/deleted matrix),
  adopt-by-name matching, sync_id backfill determinism, soft-delete filtering in list
  queries, updated_at bumped by every mutation path (each repository mutation asserted).
- **Rust integration (the core proof):** two full `AppState`s with separate temp SQLite
  DBs, each serving its real router on an ephemeral local port; run sync cycles both ways
  and assert: create propagates, edit propagates, rename propagates, delete propagates to
  trash, restore propagates, LWW picks the newer edit in a two-sided conflict, echo test
  (a fully-synced pair produces zero writes on the next cycle — the audit/no-echo guard).
- **Playwright E2E:** trash section renders + restore works against a soft-deleted song;
  existing specs must stay green with sync disabled (no PRESENTER_SYNC_PEER_URL in test
  servers).
- **Post-deploy verification:** create a test song on SNV → observe it on PP within a
  minute via the real UIs (both directions), then delete it on one side and restore from
  trash on the other; `/integrations/sync/status` healthy on both.

## Deploy

- `deploy.yml` (SNV) and `release.yml` (PP) add `PRESENTER_SYNC_PEER_URL` to the service
  units. PP deploys only on GitHub Release — shipping this needs a release cut after the
  merge, same as every PP change.
