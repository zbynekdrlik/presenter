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
   call it too.

5. Known open flaw: presentations join libraries by NAME on the wire (`SyncPresentation.library_name`)
   → mis-filing/phantom libraries across rename races (#647, cross-cutting protocol change).
