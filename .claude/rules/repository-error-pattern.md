---
paths:
  - "crates/presenter-persistence/src/repository/**"
  - "crates/presenter-server/src/router/**"
  - "crates/presenter-server/src/state/**"
  - "crates/presenter-ndi/src/manager/**"
  - "crates/presenter-ndi/src/pipeline/**"
---

# Typed repository-refusal → HTTP-status pattern (#584, extended by #586/#587, #588, #589)

When a repository/state method refuses because the URL's resource doesn't exist, it must return
the router a **typed** error, never a bare `anyhow!("... not found")` string — a bare anyhow falls
through the router's default `impl From<anyhow::Error> for AppError` to a 500, even though the
correct response is 404 (or 422 for a body-referenced missing target).

## The pattern

1. **Producer** (repository or state method): `RepositoryError::NotFound(&'static str)` (URL
   resource missing → 404) or `RepositoryError::TargetNotFound(&'static str)` (a resource named in
   the request BODY is missing → 422) — both already defined in
   `crates/presenter-persistence/src/repository/util.rs`. Use `.ok_or(RepositoryError::NotFound("..."))?`
   or `return Err(RepositoryError::NotFound("...").into());` — see "clippy gotcha" below for why
   `ok_or`, not `ok_or_else`.
2. **Consumer** (router handler): add a small **private, per-file** helper —
   ```rust
   fn map_repository_not_found(err: anyhow::Error) -> AppError {
       match err.downcast_ref::<presenter_persistence::RepositoryError>() {
           Some(presenter_persistence::RepositoryError::NotFound(msg)) => AppError::not_found(*msg),
           _ => err.into(),
       }
   }
   ```
   and wire it with `.map_err(map_repository_not_found)?` on the specific call that can fail this
   way. **Never match on `err.to_string()`** — that silently stops matching the moment a
   `.context(...)` gets added anywhere upstream (the #578→#584 regression).
3. This is a **per-file private helper**, duplicated across `router/libraries.rs`,
   `router/presentations.rs`, `router/playlists.rs`, `router/sync.rs`,
   `router/bible/presentations.rs`, `router/bible/broadcast.rs`,
   `router/integrations/{resolume,android_stage,video_source}.rs` — 9 modules total (verify with
   `grep -rln "fn map_repository_not_found\|fn map_repository_error" crates/presenter-server/src/router/`
   if this list drifts again). NOT a shared/exported helper. Each PR that touches this pattern
   should keep it that way: a shared helper is a bigger refactor than a scoped bug-fix batch needs,
   and increases review surface for no behavior change.
   **Naming exception:** every module above names the helper `map_repository_not_found` EXCEPT
   `router/presentations.rs`, which names it `map_repository_error` — don't assume the name is
   uniform when grepping for it.

## Clippy gotcha: `ok_or`, not `ok_or_else`

`.ok_or_else(|| RepositoryError::NotFound("..."))?` fails `cargo clippy` with
`unnecessary_lazy_evaluations` — constructing a `NotFound(&'static str)` tuple variant from a
string literal is cheap enough that clippy wants the eager form:
`.ok_or(RepositoryError::NotFound("..."))?`. This bit #586/#587 (8 sites, caught by CI's Clippy job,
fixed in a follow-up commit) — get it right the first time.

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

## Grep the router file for an ALREADY-EXISTING unwired helper before assuming one is missing (#608)

A "sweep" batch (#586/#587) converts several sites across many files in one pass, and it is easy to
add the `map_repository_not_found` helper to a file for site A while a DIFFERENT call in that same
file (site B) still has a bare `?` — the helper exists and compiles fine, it's just not wired
everywhere it could be. Both #608 misses (`router/integrations/resolume.rs`'s `test_resolume_host`,
`router/playlists.rs`'s `replace_playlist_entries`) were exactly this: the helper was already
defined and used elsewhere in the SAME file. Before adding a new helper to a router file, `grep -n
"map_repository_not_found" <file>` first — if it exists, the fix is a one-line `.map_err(...)` wire,
not a new helper.

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

## clippy `unused_must_use` on a discarded handler-call in a test (#588)

A router handler that returns `Result<Json<T>, AppError>` makes `Json<T>` `#[must_use]` through
axum. A test that calls the handler directly ONLY to seed state (drop the actual response) —
`set_stage_layout(State(state.clone()), Json(payload)).await.unwrap();` with no assignment — fails
`cargo clippy -- -D warnings` on `unused_must_use`, because `.unwrap()`'s returned `Json<T>` is a
bare statement. Bind it: `let _ = handler(...).await.unwrap();`.

## Test-only imports must live inside `#[cfg(test)]`, not at module level (#616)

When you extract a pure function for testability (the `map_post_whep_error` / `translate_add_consumer_error` / `map_delete_whep_error` pattern) and the test uses a CONSTANT from the parent module's imports (e.g. `MAX_CONSUMERS_PER_SOURCE` from `crate::pipeline`), that constant MUST be imported INSIDE the `#[cfg(test)] mod tests` block — NOT at the module level alongside the function's own imports. CI runs `cargo clippy -- -D warnings`, which rejects a module-level import used only in tests as `unused import` in the non-test build. The fix: move the offending item from the top-level `use crate::pipeline::{..., MAX_CONSUMERS_PER_SOURCE}` into the test module's own `use crate::pipeline::MAX_CONSUMERS_PER_SOURCE;` line.
