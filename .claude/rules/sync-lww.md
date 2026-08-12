---
paths:
  - "crates/presenter-persistence/src/repository/sync*.rs"
  - "crates/presenter-persistence/src/repository/library_sync*.rs"
  - "crates/presenter-server/src/state/sync.rs"
---

# Sync/LWW invariants (#634/#636/#646 — learned the hard way)

Two-site LWW sync (PP↔SNV), strict-`>` gate on `updated_at` (sync_id tie-break, #594).

1. **A SYNTHETIC (locally manufactured) tombstone must be LWW-NEUTRAL.** When apply-side
   integrity forces a row into a state the peer never sent (e.g. #634's forced tombstone of a
   presentation arriving under a still-tombstoned library), stamp `updated_at` with the INCOMING
   event's own clock — never a locally-derived newer clock. A manufactured newer clock propagates
   and beats the peer's legitimately live copy; back-dated `deleted_at` older than PRUNE_HORIZON
   (30 d) additionally bypasses the trash window entirely (the #646 🔴). Semantic state changes
   reach the peer only through the proper channel (library-level reconciliation → peer's own
   cascade), never as a side effect of a defensive local write.

2. **A DELIBERATE local divergence must BUMP the clock to converge.** The mirror case: when a
   site intentionally diverges from the wire event (e.g. #636's rename disambiguation writing
   `Bar (2)`), it must stamp local `now()` — writing the peer's own clock creates equal clocks
   and the strict-`>` gate then skips forever in BOTH directions (permanent divergence).

   Rule of thumb: defensive/derived write → incoming clock (never propagates); intentional
   correction → bumped clock (always propagates).

3. **Library reconciliation is BEST-EFFORT** (`state/sync.rs` — silently skipped on 404/old
   peer/decode error). Never design an apply-side fix that assumes the local library state is
   current; the peer may have revived/renamed a library you still hold tombstoned.

4. **Tombstone cascades must clean dependents everywhere**: `playlist_entry` rows +
   `slide_stage_layout` markers — `delete_library`/`delete_presentation`, the forced-tombstone
   path, AND a genuine (non-forced) incoming tombstone (#649, fixed) all do, via the shared
   `clean_playlist_entries_for_tombstone` helper (`sync_apply_tombstone_cleanup.rs`) gated on
   `effective_deleted_at.is_some()` in `write_synced_row` — a new tombstone-writing path must
   call it too. A tombstone that arrived BEFORE a cleanup gap was fixed never gets a second
   chance to clean up on its own (its re-arrival is `SkippedNotNewer` under the strict `>` gate,
   invariant 1) — `Repository::backfill_orphaned_playlist_entries` (same file) is the one-time
   startup sweep that backfills that pre-fix residue on deployed DBs (#658).

5. **Presentations join libraries by IDENTITY, name only as a compat fallback (#647, fixed).**
   `SyncPresentation`/`SyncManifestRow`/the wire DTOs now carry `library_sync_id: Option<String>`
   alongside `library_name`. Every library-resolution call site
   (`sync_apply_library.rs`'s `find_library_id`/`ensure_library`/`ensure_library_for_tombstone`,
   used by `apply_sync_presentation`, `resolve_sync_apply_target`, and `apply_unknown_sync_id`)
   tries the identity FIRST via `find_library_by_sync_id` (live or tombstoned — authoritative; a
   tombstoned identity match is never allowed to fall through to a name lookup, which would
   reintroduce mis-filing) and degrades to the pre-#647 name-only join, byte-for-byte unchanged,
   only when the identity is `None` (an old, un-upgraded peer) or hasn't converged locally yet
   (rare — a transient library-manifest fetch failure this cycle). `#[serde(default)]` on the new
   field is what makes the wire compat safe in both directions (no `deny_unknown_fields` on either
   DTO). This is why invariant 3 above still matters: library reconciliation is best-effort and
   runs BEFORE presentations in the same cycle, but a fresh identity that hasn't converged yet
   still correctly falls back to name — the fallback path is not a legacy-only concern.
