---
paths:
  - "crates/presenter-persistence/src/repository/**"
  - "crates/presenter-server/src/router.rs"
  - "crates/presenter-server/src/router/**"
  - "crates/presenter-server/src/state/**"
  - "crates/presenter-ndi/src/manager/**"
  - "crates/presenter-ndi/src/pipeline/**"
---

# Typed repository-refusal → HTTP-status pattern (#584, extended by #586/#587, #588, #589, centralized by #633)

When a repository/state method refuses because the URL's resource doesn't exist, it must return
the router a **typed** error, never a bare `anyhow!("... not found")` string — a bare anyhow falls
through the router's default `impl From<anyhow::Error> for AppError` to a 500, even though the
correct response is 404 (or 422 for a body-referenced missing target).

**`paths:` gotcha (#633 review finding):** `"crates/presenter-server/src/router/**"` matches files
INSIDE the `router/` subdirectory — it does NOT match the sibling file `crates/presenter-server/src/router.rs`
itself, even though `router.rs` is the module root for everything under `router/`. Confirmed
empirically against four independent glob matchers (Claude Code's own `ignore` npm package,
`minimatch`, Python `fnmatch`, `pathlib`): matching is segment-by-segment split on `/`, and the
literal segment `router` never equals the literal segment `router.rs`. This is why `router.rs` is
listed as its own explicit `paths:` entry above — a `paths:` list for a `<name>/` + `<name>.rs`
pair always needs BOTH entries, never just the directory glob.

## The pattern (centralized since #633 — read this before adding a per-file helper)

1. **Producer** (repository or state method): `RepositoryError::NotFound(&'static str)` (URL
   resource missing → 404), `RepositoryError::TargetNotFound(&'static str)` (a resource named in
   the request BODY is missing → 422), or `RepositoryError::Conflict(&'static str)` (the resource
   exists but its current state forbids the operation — e.g. #636's restore_presentation under a
   still-tombstoned library → 409; ALSO for stale-SET conflicts where the client's view of a
   collection is outdated — #652 F5's reorder length mismatch, "refresh and retry", matching the
   `PasteSlidesError::AnchorVanished` precedent; a stale set is NOT `TargetNotFound`) — all defined in
   `crates/presenter-persistence/src/repository/util.rs`. Use `.ok_or(RepositoryError::NotFound("..."))?`
   or `return Err(RepositoryError::NotFound("...").into());` — see "clippy gotcha" below for why
   `ok_or`, not `ok_or_else`.
2. **Consumer (router handler): do nothing — a bare `?` is enough.** The blanket
   `impl From<anyhow::Error> for AppError` in `crates/presenter-server/src/router.rs` (right after
   `AppError`'s own `impl` block) downcasts to `presenter_persistence::RepositoryError` itself:
   `NotFound` → 404, `TargetNotFound` → 422, `Conflict` → 409; every other `RepositoryError` variant
   (a genuine internal/data-integrity fault, not a client-facing refusal) and every non-repository
   error still fall through to the existing 500 branch. A plain `state.some_method(...).await?` in
   ANY handler gets the correct status **by default** — there is no per-call-site opt-in left to
   forget, which is the exact defect (#608, #610, #611, #615, #630, #636, #652 F4) this centralization
   closes: a new call site that used to compile fine with a bare `?` and silently ship 500 now gets
   the right status for free. `downcast_ref` walks the WHOLE `.context(...)` chain (not just the
   outermost layer, see the anyhow-internals section below), so this stays correct no matter how much
   context gets added between the repository and the router.
3. **There is no per-file `map_repository_not_found`/`map_repository_error` helper anymore** — #633
   deleted all 10 of them (`router/libraries.rs`, `router/presentations.rs`, `router/playlists.rs`,
   `router/sync.rs`, `router/bible/presentations.rs`, `router/bible/broadcast.rs`,
   `router/integrations/{resolume,android_stage,video_source}.rs`, `router/stage.rs`) and converted
   every call site to a bare `?` (one non-`?` tail expression, `bible/broadcast.rs`'s
   `trigger_bible_broadcast`, uses `.map_err(AppError::from)` instead, since it's the function's
   return value rather than a `?`-propagated one — same centralized mapping, just invoked
   explicitly). **Never reintroduce a per-file downcast-and-map helper for `RepositoryError`** — the
   whole point of #633 was making the centralized mapping the only path, so a new call site can't
   silently skip it again. A DOMAIN error that is NOT `RepositoryError` (see "Not every refusal is a
   `RepositoryError`" below) still gets its own small per-file downcast — that part is unchanged;
   only the `RepositoryError` case moved to the router's blanket `From` impl.

## Clippy gotcha: `ok_or`, not `ok_or_else`

`.ok_or_else(|| RepositoryError::NotFound("..."))?` fails `cargo clippy` with
`unnecessary_lazy_evaluations` — constructing a `NotFound(&'static str)` tuple variant from a
string literal is cheap enough that clippy wants the eager form:
`.ok_or(RepositoryError::NotFound("..."))?`. This bit #586/#587 (8 sites, caught by CI's Clippy job,
fixed in a follow-up commit) — get it right the first time.

## The MIRROR anti-pattern: a blanket `.map_err(AppError::bad_request)` (#615, #652 F4)

A bare anyhow → 500 has a mirror: a router handler wrapping the WHOLE state call in
`.map_err(AppError::bad_request)` maps a genuine internal fault to 400 (the client is blamed for a
server bug — worse than a 500) AND flattens every refusal to one status. `POST /stage/state` did
exactly this until #652 F4. When touching a handler, grep it for blanket `bad_request`/`map_err`
wrappers — the fix (since #633) is simply to DELETE the wrapper: typed `RepositoryError` variants in
the state method + a bare `?` in the handler now get the correct status from the router's
centralized `From<anyhow::Error> for AppError` mapping, with everything unmatched still falling
through to 500. A blanket `.map_err(...)` wrapper is exactly what defeats that centralization — it
intercepts the error BEFORE it reaches the blanket `From` impl, same as a per-file downcast helper
used to.

## Live-verifying refusal paths with curl: WRITE DTOs are camelCase (#652 verification gotcha)

GET responses serialize snake_case (`presentation_id`, `id`), but WRITE payloads deserialize
camelCase (`presentationId`, `slideIds`, `entryId` — serde renames). There is no
`deny_unknown_fields`, so a wrong field name is SILENTLY ignored and can look like success (a
duplicate-`id` payload got 200 with fresh server-generated ids; only `entryId` hits the guard).
When a verification curl returns an unexpected 200/405/serde-422, check the DTO field names and
the route in `router.rs` (e.g. reorder is `POST /presentations/{id}/slides/reorder`, entries
listing is `/presentations/{id}/slide-stage-layouts`) before concluding the fix regressed.

## Trace every CALLER before assuming the repository layer is the only 500 source

A "500 instead of 404" ticket's issue body often only lists `repository/*.rs` sites. But
`state/*.rs` methods sometimes do their OWN existence check independent of (or in ADDITION to) the
repository layer — e.g. `state/bible.rs`'s `delete_bible_slide`/`reorder_bible_slides` fetch the
presentation themselves and raise their own `anyhow!("bible presentation not found")`, never
touching the repository-layer refusal the issue body pointed at. Before fixing, grep every HTTP
route's full call chain (router handler → state method → repository method) for `anyhow!(` — don't
assume the issue body's site list is complete, especially after a file-split refactor (#590)
changed line numbers.

## Only convert sites with a real HTTP consumer

Not every `anyhow!("... not found")` in these layers is reachable from HTTP traffic — some exist
only for an internal background task (e.g. `resolume.rs`'s `update_resolume_host_active_port`,
called only by the port-drift probe, which already `tracing::warn!`s and continues on error) or a
CLI-only path (e.g. `bible/import.rs`'s `set_bible_source_digest`, reachable only via
`ingest_bibles` / the `import-data.yml` workflow). Converting those adds scope with zero
user-visible bug fixed — check every caller before including a site in the batch.

## Historical note — the per-file "forgot to wire it" failure class this centralization closed (#608)

Before #633, a "sweep" batch (#586/#587) converting several sites across many files in one pass made
it easy to add a `map_repository_not_found` helper to a file for site A while a DIFFERENT call in
that same file (site B) still had a bare `?` — the helper existed and compiled fine, it just wasn't
wired everywhere it could be. Both #608 misses (`router/integrations/resolume.rs`'s
`test_resolume_host`, `router/playlists.rs`'s `replace_playlist_entries`) were exactly this. #633
removed the opt-in step entirely — a bare `?` now gets the correct status by construction, so this
whole failure class (four more incidents after #608: #610, #611, #615, #630, #636) cannot recur for
`RepositoryError`. Kept here as the reason the per-file-helper pattern was abandoned, not as
guidance to follow.

## Not every refusal is a `RepositoryError` — a DOMAIN error gets its OWN small enum (#588, #589)

`RepositoryError::NotFound`/`TargetNotFound` is for refusals that actually originate in the
PERSISTENCE layer. A refusal decided entirely in the STATE layer (no DB touched at all — e.g.
`stage_display.rs::validate_operator_selectable` rejecting an unknown layout `code`) or across a
DIFFERENT crate boundary (e.g. `presenter-ndi`'s WHEP session refusals) must NOT be forced through
`RepositoryError` — that's a wrong-layer dependency and `NotFound(&'static str)` can't cheaply carry
a dynamic message anyway. Define a small locally-scoped `thiserror::Error` enum next to where the
refusal is decided instead, and downcast on THAT type in the router — exactly the same shape,
just a different concrete type. Precedent for this ALREADY existed before #588/#589:
`router/timers.rs` downcasts `presenter_core::timer::TimerError`, a domain error from a THIRD crate
boundary. Two more examples now: `StageLayoutRefusal` (`state/stage_display.rs`, same-crate,
`pub(crate)`) and `NdiSessionError` (`presenter_ndi::manager`, `pub`, crosses the ndi↔server crate
boundary). When a single router-facing decision point needs to translate a DIFFERENT existing typed
error (e.g. the pipeline's own `AddConsumerError::CapReached`) into the shared vocabulary, do it at
the ONE call site where it crosses into the shared `anyhow::Result` (see `NdiManager::whep_post`) —
don't make the router downcast on two parallel taxonomies for the same decision.

## `anyhow::Error::downcast_ref` walks the WHOLE context chain — verified in anyhow's own source

The reason string-matching breaks under `.context(...)` while `downcast_ref` doesn't is NOT
incidental — it's the documented mechanism. `Error::context()` builds a `ContextError<C, E>` whose
vtable's `object_downcast` (`context_chain_downcast` in `anyhow`'s `error.rs`) checks the context
type `C` first and, if it doesn't match, RECURSES into the wrapped error's OWN vtable — so
`err.downcast_ref::<E>()` finds `E` no matter how many `.context(...)` layers were added on top,
while `err.to_string()` only ever shows the OUTERMOST context message. This is what makes the
typed-error pattern robust and string-matching fragile — cite it when justifying the pattern instead
of re-deriving it from scratch (confirmed against `anyhow-1.0.103/src/error.rs` on this box).

## `anyhow::Error::context()` is INHERENT — `use anyhow::Context;` is only for `Result`, not `Error` (#633)

Calling `.context(...)` directly on an already-constructed `anyhow::Error` (e.g.
`anyhow::Error::from(RepositoryError::NotFound("x")).context("while renaming")`, as the #633 tests
do to prove `downcast_ref` walks the context chain) resolves to `anyhow::Error`'s own INHERENT
`context()` method — it does NOT need the `anyhow::Context` trait, which exists only to add
`.context(...)` to a `Result<T, E: std::error::Error>`. Importing `use anyhow::Context;` in a test
that only ever calls `.context()` on an `Error` value (never on a `Result`) triggers `cargo clippy -- -D
warnings`'s `unused_imports` and fails CI. Only import the trait when you're chaining `.context(...)`
off something that returns `Result`, not `Error`.

## Verify a sweep is COMPLETE with `cargo check --workspace --tests`, not just `grep` + a careful read (#633)

When deleting a duplicated helper and converting call sites across many files in one pass, a
sufficiently careful read-through can still miss one — #633's own salvaged patch missed
`router/integrations/resolume.rs`'s `test_resolume_host`, which still called the deleted
`map_repository_not_found` after every OTHER call site in that file had been converted. This is
the exact #608 failure class this ticket exists to close, recurring inside the fix itself. `grep -rn
"map_repository_not_found\|map_repository_error"` catches leftover REFERENCES, but the
authoritative check on a Tier-0 box (no local `cargo test`) is `cargo check --workspace --tests` —
it fails loudly (`E0425: cannot find value ... in this scope`) on a dangling call to a function you
just deleted, and it is allowed to run locally per this project's Tier-0 policy. Run it after any
multi-file deletion/rename sweep, before considering the sweep done.

## clippy `unused_must_use` on a discarded handler-call in a test (#588)

A router handler that returns `Result<Json<T>, AppError>` makes `Json<T>` `#[must_use]` through
axum. A test that calls the handler directly ONLY to seed state (drop the actual response) —
`set_stage_layout(State(state.clone()), Json(payload)).await.unwrap();` with no assignment — fails
`cargo clippy -- -D warnings` on `unused_must_use`, because `.unwrap()`'s returned `Json<T>` is a
bare statement. Bind it: `let _ = handler(...).await.unwrap();`.

## Test-only imports must live inside `#[cfg(test)]`, not at module level (#616)

When you extract a pure function for testability (the `map_post_whep_error` / `translate_add_consumer_error` / `map_delete_whep_error` pattern) and the test uses a CONSTANT from the parent module's imports (e.g. `MAX_CONSUMERS_PER_SOURCE` from `crate::pipeline`), that constant MUST be imported INSIDE the `#[cfg(test)] mod tests` block — NOT at the module level alongside the function's own imports. CI runs `cargo clippy -- -D warnings`, which rejects a module-level import used only in tests as `unused import` in the non-test build. The fix: move the offending item from the top-level `use crate::pipeline::{..., MAX_CONSUMERS_PER_SOURCE}` into the test module's own `use crate::pipeline::MAX_CONSUMERS_PER_SOURCE;` line.

## `classify_restore`-shape helper: return the destructured invariant-proven value, never `Option<&Model>` + `.expect()` (#644)

`classify_restore_library` (`repository/sync.rs`, #646) established the "missing → NotFound (404),
wrong-state → Conflict (409), otherwise Ok" split for a restore-style refusal. `#644`'s
`restore_library` (`repository/library.rs`) extended the SAME shape one step further: the first cut
returned a bare `&library::Model` from the classifier, forcing the caller to re-derive the
already-proven-`Some` field with `lib.deleted_at.expect("classify_restore only returns Ok for a row
with deleted_at set")` — a review finding (`47a82b33`) caught this as a banned `.expect()` in
production code (`presenter-persistence` has no test/WASM exemption). Fix: have the classifier return
the DESTRUCTURED value alongside the row — `Result<(&library::Model, DateTime<FixedOffset>),
RepositoryError>` — so the "the trashed branch always has a tombstone timestamp" invariant is upheld
by the FUNCTION SIGNATURE at the one place that already pattern-matched it, never by a caller trusting
an `.expect()` that "can never actually fire". Reach for this shape whenever a classify/validate helper
proves a field is `Some`/non-empty/in-range and the caller would otherwise re-extract it with `.expect()`
or `.unwrap()`.

## Cascade-scoped restore: compare timestamps in RUST, never via a SQL equality filter (#644)

When a soft-delete cascades a timestamp onto several rows in ONE transaction (`delete_library` stamping
the identical `now` on the library row and every presentation it cascades — see `sync-lww.md`'s own
"cascade-scoped restore" entry for the LWW-specific half of this), and a later RESTORE must bring back
only the rows that were part of THAT cascade (not a row trashed independently, at a different instant):
fetch both sides (the parent's own tombstone timestamp, and the candidate rows' `deleted_at`) and
compare the already-decoded `DateTime<FixedOffset>` values in RUST — `*deleted_at == Some(tombstoned_at)`
— never build a SQL `.eq(fetched_datetime_value)` filter for this. The write path stores the raw
`to_rfc3339()` STRING directly via `Expr::value(now: String)`, not through sea_orm's own chrono value
binding, so there is no guarantee a re-serialized bound parameter would byte-match that raw string on
the SQL side. Two Rust values decoded from byte-identical stored TEXT via the SAME driver parse path are
guaranteed `==` regardless of that risk — comparing in Rust sidesteps the question entirely instead of
having to reason about it.
